use crate::Data;
use actix_web::{get, http::header, patch, post, web, HttpRequest, HttpResponse};
use headers::{authorization::Bearer, Authorization, Header};
use jwt_compact::Token;
use mr_messages_lib::{
    ErrorKind, ErrorResponse, GetLevelResponse, GetLevelsRequest, GetLevelsUserFilter,
    GetUserResponse, LevelDto, LevelPermissionDto, LevelsListItem, LinkAccount, LinkAccountError,
    LinkAccountLoginMethod, LinkAccountRequest, PaginationParams, PatchUserError, PatchUserRequest,
    RegisterAccountError, RegisteredUser,
};
use mr_utils_lib::JwtAuthClaims;
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

#[get("/users/{id}")]
pub async fn get_user(data: web::Data<Data>, user_id: web::Path<i64>) -> HttpResponse {
    let mut connection = match data.pool.acquire().await {
        Ok(c) => c,
        Err(err) => {
            log::error!("Failed to acquire a connection: {:?}", err);
            return HttpResponse::InternalServerError().finish();
        }
    };

    let user = sqlx::query_as!(
        GetUserResponse,
        "SELECT id, display_name, created_at, updated_at FROM users WHERE id = $1",
        user_id.into_inner()
    )
    .fetch_one(&mut connection)
    .await;

    match user {
        Ok(user) => HttpResponse::Ok().json(user),
        Err(sqlx::Error::RowNotFound) => HttpResponse::NotFound().json(ErrorResponse::<()> {
            message: "User doesn't exist".to_owned(),
            error_kind: ErrorKind::NotFound,
        }),
        Err(err) => {
            log::error!("Failed to get a user: {:?}", err);
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

    if decoded_bearer_token.claims().custom.email.is_none()
        || decoded_existing_account_token
            .claims()
            .custom
            .email
            .is_none()
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
        oidc.issuer == decoded_token.claims().custom.iss
            && oidc.subject == decoded_token.claims().custom.sub
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
        let err: sqlx::Error = err;
        if let Some("users_display_name_key") =
            err.as_database_error().and_then(|err| err.constraint())
        {
            return HttpResponse::BadRequest().json(ErrorResponse::<PatchUserError> {
                message: "Display name is already taken".to_owned(),
                error_kind: ErrorKind::RouteSpecific(PatchUserError::DisplayNameTaken),
            });
        }

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
    existing_account: &Token<JwtAuthClaims>,
    new_oidc: &Token<JwtAuthClaims>,
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
        oidc.issuer == existing_account.claims().custom.iss
            && oidc.subject == existing_account.claims().custom.sub
    });
    let Some(user_oidc) = user_oidc else {
        log::debug!("Existing user claims mismatch");
        return Err(InsertOidcError::Forbidden);
    };

    if user_oidc.email != new_oidc.claims().custom.email {
        log::debug!("Email mismatch");
        return Err(InsertOidcError::ClaimsMismatch);
    }

    let already_linked = user_oidcs.iter().any(|oidc| {
        oidc.issuer == new_oidc.claims().custom.iss && oidc.subject == new_oidc.claims().custom.sub
    });
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
        new_oidc.claims().custom.iss,
        new_oidc.claims().custom.sub,
        new_oidc.claims().custom.email,
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
    user_data: Token<JwtAuthClaims>,
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
        user_data.claims().custom.iss.clone(),
        user_data.claims().custom.sub.clone(),
        user_data.claims().custom.email,
    )
    .fetch_all(&mut connection)
    .await?;

    let already_registered_user = user_oidcs.iter().find(|user| {
        user.issuer.as_ref() == Some(&user_data.claims().custom.iss)
            && user.subject.as_ref() == Some(&user_data.claims().custom.sub)
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

    let email = user_data.claims().custom.email.clone();

    let mut transaction = connection.begin().await?;
    let NewUserDto { id, created_at } = sqlx::query_as!(
        NewUserDto,
        "INSERT INTO users (email) VALUES ($1) RETURNING id, created_at",
        user_data.claims().custom.email.clone(),
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
        user_data.claims().custom.iss,
        user_data.claims().custom.sub,
        user_data.claims().custom.email,
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

#[get("/levels")]
pub async fn get_levels(data: web::Data<Data>, body: web::Query<GetLevelsRequest>) -> HttpResponse {
    let GetLevelsRequest {
        user_filter,
        pagination,
    } = body.into_inner();
    if pagination.limit == 0 || pagination.limit > 100 {
        return HttpResponse::BadRequest().json(ErrorResponse::<()> {
            message: "The `limit` parameter must be in the range of 1..=100".to_owned(),
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

    let levels: Result<Vec<LevelsListItem>, sqlx::Error> = match user_filter {
        Some(GetLevelsUserFilter::AuthorId(author_id)) => {
            query_levels_by_author(&mut connection, Some(author_id), pagination).await
        }
        Some(GetLevelsUserFilter::BuilderId(builder_id)) => {
            query_levels_by_builder(&mut connection, builder_id, pagination).await
        }
        None => query_levels_by_author(&mut connection, None, pagination).await,
    };

    match levels {
        Ok(levels) => HttpResponse::Ok().json(&levels),
        Err(err) => {
            log::error!("Failed to get levels: ${:?}", err);
            HttpResponse::InternalServerError().finish()
        }
    }
}

#[get("/levels/{id}")]
pub async fn get_level(data: web::Data<Data>, level_id: web::Path<i64>) -> HttpResponse {
    let id = level_id.into_inner();
    let mut connection = match data.pool.acquire().await {
        Ok(c) => c,
        Err(err) => {
            log::error!("Failed to acquire a connection: {:?}", err);
            return HttpResponse::InternalServerError().finish();
        }
    };

    let level = sqlx::query_as!(
        LevelDto,
        r#"
SELECT l.id, l.title, l.data, u.id AS user_id, u.display_name AS user_name, l.parent_id, l.created_at, l.updated_at
FROM levels AS l
JOIN users AS u ON u.id = l.user_id
WHERE l.id = $1 AND l.is_autosaved = FALSE
        "#,
        id,
    )
        .fetch_one(&mut connection)
        .await;

    let level = match level {
        Ok(level) => level,
        Err(sqlx::Error::RowNotFound) => {
            return HttpResponse::NotFound().json(ErrorResponse::<()> {
                message: "Level doesn't exist".to_owned(),
                error_kind: ErrorKind::NotFound,
            })
        }
        Err(err) => {
            log::error!("Failed to get a level: ${:?}", err);
            return HttpResponse::InternalServerError().finish();
        }
    };

    let autosaved_versions = sqlx::query_as!(
        LevelsListItem,
        r#"
SELECT l.id, l.title, u.id AS user_id, u.display_name AS user_name, l.parent_id, l.created_at, l.updated_at
FROM levels AS l
JOIN users AS u ON u.id = l.user_id
WHERE l.parent_id = $1 AND l.is_autosaved = TRUE
        "#,
        id,
    )
        .fetch_all(&mut connection)
        .await;

    let autosaved_versions = match autosaved_versions {
        Ok(autosaved_versions) => autosaved_versions,
        Err(err) => {
            log::error!("Failed to get autosaved levels: ${:?}", err);
            return HttpResponse::InternalServerError().finish();
        }
    };

    let level_permissions = sqlx::query_as!(
        LevelPermissionDto,
        r#"
SELECT l.user_id, u.display_name AS user_name, l.created_at
FROM level_permissions l
JOIN users AS u ON u.id = l.user_id
WHERE level_id = $1"#,
        id
    )
    .fetch_all(&mut connection)
    .await;

    match level_permissions {
        Ok(level_permissions) => HttpResponse::Ok().json(&GetLevelResponse {
            level,
            autosaved_versions,
            level_permissions,
        }),
        Err(err) => {
            log::error!("Failed to get level permissions: ${:?}", err);
            HttpResponse::InternalServerError().finish()
        }
    }
}

async fn query_levels_by_author(
    connection: &mut sqlx::PgConnection,
    author_id: Option<i64>,
    pagination: PaginationParams,
) -> sqlx::Result<Vec<LevelsListItem>> {
    sqlx::query_as!(
        LevelsListItem,
        r#"
SELECT l.id as "id!", l.title as "title!", u.id AS "user_id!", u.display_name AS user_name, l.parent_id, l.created_at as "created_at!", l.updated_at as "updated_at!"
FROM levels l
INNER JOIN users AS u ON u.id = l.user_id
WHERE ($1::bigint IS NULL OR u.id = $1) AND l.is_autosaved = FALSE
LIMIT $2 OFFSET $3
        "#,
        author_id,
        pagination.limit,
        pagination.offset,
    )
        .fetch_all(connection)
        .await
}

async fn query_levels_by_builder(
    connection: &mut sqlx::PgConnection,
    builder_id: i64,
    pagination: PaginationParams,
) -> sqlx::Result<Vec<LevelsListItem>> {
    sqlx::query_as!(
        LevelsListItem,
        r#"
SELECT l.id as "id!", l.title as "title!", u.id AS "user_id!", u.display_name AS user_name, l.parent_id, l.created_at as "created_at!", l.updated_at as "updated_at!"
FROM levels l
JOIN users AS u ON u.id = l.user_id
JOIN level_permissions AS lp ON lp.level_id = l.id
WHERE lp.user_id = $1 AND l.is_autosaved = FALSE
LIMIT $2 OFFSET $3
        "#,
        builder_id,
        pagination.limit,
        pagination.offset,
    )
        .fetch_all(connection)
        .await
}
