use crate::Data;
use actix_web::{http::header, patch, post, web, HttpRequest, HttpResponse};
use headers::{authorization::Bearer, Authorization, Header};
use jsonwebtoken::TokenData;
use mr_messages_lib::{
    ErrorKind, ErrorResponse, JwtAuthClaims, LinkAccount, LinkAccountError, LinkAccountLoginMethod,
    LinkAccountRequest, PatchUserRequest, RegisterAccountError, RegisteredUser,
};
use sqlx::{types::chrono, Connection};

#[post("/users/auth")]
pub async fn register(data: web::Data<Data>, req: HttpRequest) -> HttpResponse {
    let mut authorization = req.headers().get_all(header::AUTHORIZATION);
    let jwt = match Authorization::<Bearer>::decode(&mut authorization) {
        Ok(header_value) => header_value.0.token().to_owned(),
        Err(_) => {
            return HttpResponse::Unauthorized().json(ErrorResponse::<()> {
                message: "Unauthorized".to_owned(),
                error_kind: ErrorKind::Unauthorized,
            });
        }
    };

    let decoded_token = match crate::decode_token_helper(&data, &jwt, "bearer").await {
        Ok(token) => token,
        Err(err) => {
            return err;
        }
    };

    let connection = match data.pool.acquire().await {
        Ok(c) => c,
        Err(err) => {
            log::error!("Failed to acquire a connection: {:?}", err);
            return HttpResponse::InternalServerError().finish();
        }
    };

    match insert_user(connection, decoded_token).await {
        Ok(user) => HttpResponse::Ok().json(user),
        Err(InsertUserError::AlreadyExists {
            user,
            login_methods,
        }) => HttpResponse::BadRequest().json(ErrorResponse::<RegisterAccountError> {
            message: "User is already registered with using a different OIDC provider".to_owned(),
            error_kind: ErrorKind::RouteSpecific(RegisterAccountError::UserWithEmailAlreadyExists(
                LinkAccount {
                    user,
                    login_methods,
                },
            )),
        }),
        Err(InsertUserError::Sql(err)) => {
            log::error!("Failed to upsert a user: {:?}", err);
            HttpResponse::InternalServerError().finish()
        }
    }
}

#[post("/users/{id}/link")]
pub async fn link_account(
    data: web::Data<Data>,
    req: HttpRequest,
    user_id: web::Path<i64>,
    body: web::Json<LinkAccountRequest>,
) -> HttpResponse {
    let mut authorization = req.headers().get_all(header::AUTHORIZATION);
    let bearer_jwt = match Authorization::<Bearer>::decode(&mut authorization) {
        Ok(header_value) => header_value.0.token().to_owned(),
        Err(_) => {
            return HttpResponse::Unauthorized().json(ErrorResponse::<()> {
                message: "Unauthorized".to_owned(),
                error_kind: ErrorKind::Unauthorized,
            });
        }
    };
    let decoded_bearer_token = match crate::decode_token_helper(&data, &bearer_jwt, "bearer").await
    {
        Ok(token) => token,
        Err(err) => {
            return err;
        }
    };

    let LinkAccountRequest {
        existing_account_jwt,
    } = body.into_inner();
    let decoded_existing_account_token =
        match crate::decode_token_helper(&data, &existing_account_jwt, "existing account").await {
            Ok(token) => token,
            Err(err) => {
                return err;
            }
        };

    if decoded_bearer_token.claims.email.is_none()
        || decoded_existing_account_token.claims.email.is_none()
    {
        return HttpResponse::BadRequest().json(ErrorResponse::<()> {
            message: "JWTs of connected accounts must contain email claims".to_owned(),
            error_kind: ErrorKind::BadRequest,
        });
    }

    let connection = match data.pool.acquire().await {
        Ok(c) => c,
        Err(err) => {
            log::error!("Failed to acquire a connection: {:?}", err);
            return HttpResponse::InternalServerError().finish();
        }
    };

    match insert_oidc(
        connection,
        user_id.into_inner(),
        &decoded_existing_account_token,
        &decoded_bearer_token,
    )
    .await
    {
        Ok(()) => HttpResponse::Ok().json(&()),
        Err(InsertOidcError::NotFound) => {
            HttpResponse::NotFound().json(ErrorResponse::<LinkAccountError> {
                message: "User doesn't exist".to_owned(),
                error_kind: ErrorKind::NotFound,
            })
        }
        Err(InsertOidcError::Forbidden) => {
            HttpResponse::Forbidden().json(ErrorResponse::<LinkAccountError> {
                message: "JWT claims mismatch".to_owned(),
                error_kind: ErrorKind::Forbidden,
            })
        }
        Err(InsertOidcError::ClaimsMismatch) => {
            HttpResponse::BadRequest().json(ErrorResponse::<LinkAccountError> {
                message: "Claims mismatch".to_owned(),
                error_kind: ErrorKind::RouteSpecific(LinkAccountError::ClaimsMismatch),
            })
        }
        Err(InsertOidcError::AlreadyLinked) => {
            HttpResponse::BadRequest().json(ErrorResponse::<LinkAccountError> {
                message: "Account is already linked".to_owned(),
                error_kind: ErrorKind::RouteSpecific(LinkAccountError::AlreadyLinked),
            })
        }
        Err(InsertOidcError::Sql(err)) => {
            log::error!("Failed to insert an OIDC: {:?}", err);
            HttpResponse::InternalServerError().finish()
        }
    }
}

#[patch("/users/{id}")]
pub async fn patch_user(
    data: web::Data<Data>,
    req: HttpRequest,
    user_id: web::Path<i64>,
    body: web::Json<PatchUserRequest>,
) -> HttpResponse {
    let user_id = user_id.into_inner();
    let mut authorization = req.headers().get_all(header::AUTHORIZATION);
    let jwt = match Authorization::<Bearer>::decode(&mut authorization) {
        Ok(header_value) => header_value.0.token().to_owned(),
        Err(_) => {
            return HttpResponse::Unauthorized().json(ErrorResponse::<()> {
                message: "Unauthorized".to_owned(),
                error_kind: ErrorKind::Unauthorized,
            });
        }
    };

    let decoded_token = match crate::decode_token_helper(&data, &jwt, "bearer").await {
        Ok(token) => token,
        Err(err) => {
            return err;
        }
    };

    let display_name = body.0.display_name.trim();
    if display_name.is_empty() || display_name.len() > 255 || !display_name.is_ascii() {
        return HttpResponse::BadRequest().json(ErrorResponse::<()> {
            message: "Display name must not be empty and can contain only ASCII characters"
                .to_owned(),
            error_kind: ErrorKind::BadRequest,
        });
    }

    let mut connection = match data.pool.acquire().await {
        Ok(c) => c,
        Err(err) => {
            log::error!("Failed to acquire a connection: {:?}", err);
            return HttpResponse::InternalServerError().finish();
        }
    };

    struct UserOidcDto {
        issuer: String,
        subject: String,
    }

    let user_oidcs: Vec<UserOidcDto> = match sqlx::query_as!(
        UserOidcDto,
        "
SELECT o.issuer, o.subject
FROM users u
JOIN openids AS o ON u.id = o.user_id
WHERE u.id = $1
        ",
        user_id,
    )
    .fetch_all(&mut connection)
    .await
    {
        Ok(u) => u,
        Err(err) => {
            log::error!("Failed to get user: {:?}", err);
            return HttpResponse::InternalServerError().finish();
        }
    };

    if user_oidcs.is_empty() {
        log::debug!("User {} doesn't exist", user_id);
        return HttpResponse::NotFound().json(ErrorResponse::<()> {
            message: "User doesn't exist".to_owned(),
            error_kind: ErrorKind::NotFound,
        });
    }

    let oidc_found = user_oidcs.iter().any(|oidc| {
        oidc.issuer == decoded_token.claims.iss && oidc.subject == decoded_token.claims.sub
    });
    if !oidc_found {
        log::debug!("Existing user claims mismatch");
        return HttpResponse::Forbidden().json(ErrorResponse::<()> {
            message: "JWT claims mismatch".to_owned(),
            error_kind: ErrorKind::Forbidden,
        });
    }

    if let Err(err) = sqlx::query!(
        "UPDATE users SET display_name = $1 WHERE id = $2",
        display_name,
        user_id,
    )
    .execute(&mut connection)
    .await
    {
        log::error!("Failed to patch user: {:?}", err);
        return HttpResponse::InternalServerError().finish();
    }

    HttpResponse::Ok().json(&())
}

enum InsertOidcError {
    NotFound,
    Forbidden,
    ClaimsMismatch,
    AlreadyLinked,
    Sql(sqlx::Error),
}

impl From<sqlx::Error> for InsertOidcError {
    fn from(err: sqlx::Error) -> Self {
        Self::Sql(err)
    }
}

async fn insert_oidc(
    mut connection: sqlx::pool::PoolConnection<sqlx::Postgres>,
    user_id: i64,
    existing_account: &TokenData<JwtAuthClaims>,
    new_oidc: &TokenData<JwtAuthClaims>,
) -> Result<(), InsertOidcError> {
    struct UserOidcDto {
        id: i64,
        email: Option<String>,
        issuer: String,
        subject: String,
    }

    let user_oidcs: Vec<UserOidcDto> = sqlx::query_as!(
        UserOidcDto,
        "
SELECT u.id, u.email, o.issuer, o.subject
FROM users u
JOIN openids AS o ON u.id = o.user_id
WHERE u.id = $1
        ",
        user_id,
    )
    .fetch_all(&mut connection)
    .await?;

    if user_oidcs.is_empty() {
        log::debug!("User {} doesn't exist", user_id);
        return Err(InsertOidcError::NotFound);
    }

    let user_oidc = user_oidcs.iter().find(|oidc| {
        oidc.issuer == existing_account.claims.iss && oidc.subject == existing_account.claims.sub
    });
    let Some(user_oidc) = user_oidc else {
        log::debug!("Existing user claims mismatch");
        return Err(InsertOidcError::Forbidden);
    };

    if user_oidc.email != new_oidc.claims.email {
        log::debug!("Email mismatch");
        return Err(InsertOidcError::ClaimsMismatch);
    }

    let already_linked = user_oidcs
        .iter()
        .any(|oidc| oidc.issuer == new_oidc.claims.iss && oidc.subject == new_oidc.claims.sub);
    if already_linked {
        return Err(InsertOidcError::AlreadyLinked);
    }

    sqlx::query!(
        "
INSERT INTO openids
(user_id, issuer, subject, email)
VALUES ($1, $2, $3, $4)
        ",
        user_oidc.id,
        new_oidc.claims.iss,
        new_oidc.claims.sub,
        new_oidc.claims.email,
    )
    .execute(&mut connection)
    .await?;

    Ok(())
}

enum InsertUserError {
    AlreadyExists {
        user: RegisteredUser,
        login_methods: Vec<LinkAccountLoginMethod>,
    },
    Sql(sqlx::Error),
}

impl From<sqlx::Error> for InsertUserError {
    fn from(err: sqlx::Error) -> Self {
        Self::Sql(err)
    }
}

async fn insert_user(
    mut connection: sqlx::pool::PoolConnection<sqlx::Postgres>,
    user_data: TokenData<JwtAuthClaims>,
) -> Result<RegisteredUser, InsertUserError> {
    // Wrapping everything into optionals shouldn't be needed.
    // TODO: track https://github.com/launchbadge/sqlx/issues/1266.
    struct UserOidcDto {
        id: Option<i64>,
        email: Option<String>,
        display_name: Option<String>,
        oidc_email: Option<String>,
        issuer: Option<String>,
        subject: Option<String>,
        created_at: Option<chrono::NaiveDateTime>,
        updated_at: Option<chrono::NaiveDateTime>,
    }

    struct NewUserDto {
        id: i64,
        created_at: chrono::NaiveDateTime,
    }

    let user_oidcs: Vec<UserOidcDto> = sqlx::query_as!(
        UserOidcDto,
        "
SELECT u.id, u.email, u.display_name, o.email AS oidc_email, o.issuer, o.subject, o.created_at, o.updated_at
FROM users u
JOIN openids AS o ON u.id = o.user_id
WHERE o.issuer = $1 AND o.subject = $2
UNION
SELECT u.id, u.email, u.display_name, o.email AS oidc_email, o.issuer, o.subject, o.created_at, o.updated_at
FROM users u
JOIN openids AS o ON u.id = o.user_id
WHERE u.email = $3 AND $3 IS NOT NULL
        ",
        user_data.claims.iss.clone(),
        user_data.claims.sub.clone(),
        user_data.claims.email,
    )
    .fetch_all(&mut connection)
    .await?;

    let already_registered_user = user_oidcs.iter().find(|user| {
        user.issuer.as_ref() == Some(&user_data.claims.iss)
            && user.subject.as_ref() == Some(&user_data.claims.sub)
    });
    if let Some(user) = already_registered_user {
        return Ok(RegisteredUser {
            id: user.id.unwrap(),
            email: user.email.clone(),
            display_name: user.display_name.clone(),
            created_at: user.created_at.unwrap(),
            updated_at: user.updated_at.unwrap(),
        });
    }

    if !user_oidcs.is_empty() {
        let user = &user_oidcs[0];
        return Err(InsertUserError::AlreadyExists {
            user: RegisteredUser {
                id: user.id.unwrap(),
                email: user.email.clone(),
                display_name: user.display_name.clone(),
                created_at: user.created_at.unwrap(),
                updated_at: user.updated_at.unwrap(),
            },
            login_methods: user_oidcs
                .into_iter()
                .map(|user_oidc| LinkAccountLoginMethod {
                    issuer: user_oidc.issuer.unwrap(),
                    login_hint: user_oidc.oidc_email.or(user_oidc.subject).unwrap(),
                })
                .collect(),
        });
    }

    let email = user_data.claims.email.clone();

    let mut transaction = connection.begin().await?;
    let NewUserDto { id, created_at } = sqlx::query_as!(
        NewUserDto,
        "INSERT INTO users (email) VALUES ($1) RETURNING id, created_at",
        user_data.claims.email.clone(),
    )
    .fetch_one(&mut transaction)
    .await?;
    sqlx::query!(
        "
INSERT INTO openids
(user_id, issuer, subject, email)
VALUES ($1, $2, $3, $4)
        ",
        id,
        user_data.claims.iss,
        user_data.claims.sub,
        user_data.claims.email,
    )
    .execute(&mut transaction)
    .await?;
    transaction.commit().await?;

    Ok(RegisteredUser {
        id,
        email,
        display_name: None,
        created_at,
        updated_at: created_at,
    })
}
