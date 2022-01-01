#![feature(let_else)]
#![feature(try_blocks)]

use actix_web::{http::header, post, web, App, HttpRequest, HttpResponse, HttpServer};
use headers::{
    authorization::{Authorization, Bearer},
    Header,
};
use jsonwebtoken::{
    decode, decode_header,
    jwk::{AlgorithmParameters, Jwk, JwkSet},
    DecodingKey, TokenData, Validation,
};
use mr_messages_lib::{
    ErrorKind, ErrorResponse, LinkAccount, LinkAccountError, LinkAccountLoginMethod,
    LinkAccountRequest, RegisterAccountError, RegisteredUser,
};
use reqwest::Url;
use serde::Deserialize;
use sqlx::{postgres::PgPoolOptions, types::chrono, Connection, Error};
use std::time::Duration;

const DEFAULT_JWK_CACHE_TTL: u64 = 15;

#[derive(Clone)]
struct Config {
    google_certs_url: Url,
    auth0_certs_url: Url,
    google_client_id: String,
    auth0_client_id: String,
}

#[derive(Clone)]
struct Data {
    pool: sqlx::PgPool,
    jwks: Jwks,
    config: Config,
}

#[derive(Clone)]
struct Jwks {
    // A pair of a certs url and Jwk.
    keys: std::sync::Arc<tokio::sync::RwLock<Vec<(Url, Jwk)>>>,
}

impl Jwks {
    pub fn new() -> Self {
        Self {
            keys: std::sync::Arc::new(tokio::sync::RwLock::new(Vec::new())),
        }
    }

    pub async fn update(&self, issuer: &Url, new_keys: Vec<Jwk>) {
        let mut keys = self.keys.write().await;
        keys.retain(|(key_issuer, _)| key_issuer != issuer);
        keys.extend(new_keys.into_iter().map(|key| (issuer.clone(), key)));
    }

    pub async fn get(&self, kid: &str) -> Option<Jwk> {
        let keys = self.keys.read().await;
        keys.iter()
            .find(|(_, key)| key.common.key_id.as_deref() == Some(kid))
            .map(|(_, key)| key.clone())
    }
}

#[derive(Deserialize, Debug)]
struct JwtClaims {
    iss: String,
    sub: String,
    email: Option<String>,
}

#[post("/users/auth")]
async fn register(data: web::Data<Data>, req: HttpRequest) -> HttpResponse {
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

    let decoded_token = match decode_token_helper(&data, &jwt, "bearer").await {
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
            user_id,
            login_methods,
        }) => HttpResponse::BadRequest().json(ErrorResponse::<RegisterAccountError> {
            message: "User is already registered with using a different OIDC provider".to_owned(),
            error_kind: ErrorKind::RouteSpecific(RegisterAccountError::UserWithEmailAlreadyExists(
                LinkAccount {
                    user_id,
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
async fn link_account(
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
    let decoded_bearer_token = match decode_token_helper(&data, &bearer_jwt, "bearer").await {
        Ok(token) => token,
        Err(err) => {
            return err;
        }
    };

    let LinkAccountRequest {
        existing_account_jwt,
    } = body.into_inner();
    let decoded_existing_account_token =
        match decode_token_helper(&data, &existing_account_jwt, "existing account").await {
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

enum InsertOidcError {
    ClaimsMismatch,
    AlreadyLinked,
    Sql(sqlx::Error),
}

impl From<sqlx::Error> for InsertOidcError {
    fn from(err: Error) -> Self {
        Self::Sql(err)
    }
}

async fn insert_oidc(
    mut connection: sqlx::pool::PoolConnection<sqlx::Postgres>,
    user_id: i64,
    existing_account: &TokenData<JwtClaims>,
    new_oidc: &TokenData<JwtClaims>,
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

    let user_oidc = user_oidcs.iter().find(|oidc| {
        oidc.issuer == existing_account.claims.iss && oidc.subject == existing_account.claims.sub
    });
    let Some(user_oidc) = user_oidc else {
        log::debug!("Existing user claims mismatch");
        return Err(InsertOidcError::ClaimsMismatch);
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
        user_id: i64,
        login_methods: Vec<LinkAccountLoginMethod>,
    },
    Sql(sqlx::Error),
}

impl From<sqlx::Error> for InsertUserError {
    fn from(err: Error) -> Self {
        Self::Sql(err)
    }
}

async fn insert_user(
    mut connection: sqlx::pool::PoolConnection<sqlx::Postgres>,
    user_data: TokenData<JwtClaims>,
) -> Result<RegisteredUser, InsertUserError> {
    // Wrapping everything into optionals shouldn't be needed.
    // TODO: track https://github.com/launchbadge/sqlx/issues/1266.
    struct UserOidcDto {
        id: Option<i64>,
        email: Option<String>,
        username: Option<String>,
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
SELECT u.id, u.email, u.username, o.email AS oidc_email, o.issuer, o.subject, o.created_at, o.updated_at
FROM users u
JOIN openids AS o ON u.id = o.user_id
WHERE o.issuer = $1 AND o.subject = $2
UNION
SELECT u.id, u.email, u.username, o.email AS oidc_email, o.issuer, o.subject, o.created_at, o.updated_at
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
            username: user.username.clone(),
            created_at: user.created_at.unwrap(),
            updated_at: user.updated_at.unwrap(),
        });
    }

    if !user_oidcs.is_empty() {
        return Err(InsertUserError::AlreadyExists {
            user_id: user_oidcs[0].id.unwrap(),
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
        username: None,
        created_at,
        updated_at: created_at,
    })
}

async fn decode_token_helper(
    data: &Data,
    token: &str,
    kind: &str,
) -> Result<TokenData<JwtClaims>, HttpResponse> {
    let kid = match decode_header(token).ok().and_then(|header| header.kid) {
        Some(k) => k,
        None => {
            return Err(HttpResponse::BadRequest().json(ErrorResponse::<()> {
                message: format!("Token doesn't have a `kid` header field (kind: {})", kind),
                error_kind: ErrorKind::Unauthorized,
            }))
        }
    };

    if let Some(key) = data.jwks.get(&kid).await {
        match key.algorithm {
            AlgorithmParameters::RSA(ref rsa) => {
                let decoding_key = DecodingKey::from_rsa_components(&rsa.n, &rsa.e).unwrap();
                let mut validation = Validation::new(key.common.algorithm.unwrap());
                validation.set_audience(&[
                    data.config.auth0_client_id.clone(),
                    data.config.google_client_id.clone(),
                ]);
                decode::<JwtClaims>(token, &decoding_key, &validation).map_err(|err| {
                    log::warn!("Invalid or expired JWT (kind: {}): {:?}", kind, err);
                    HttpResponse::BadRequest().json(ErrorResponse::<()> {
                        message: format!("Invalid or expired JWT (kind: {})", kind),
                        error_kind: ErrorKind::BadRequest,
                    })
                })
            }
            _ => {
                // TODO: sentry error.
                log::error!("Non-RSA JWK: {}", kid);
                Err(HttpResponse::InternalServerError().finish())
            }
        }
    } else {
        log::warn!(
            "No matching JWK found for the given kid (kind: {}): {}",
            kind,
            &kid
        );
        Err(HttpResponse::BadRequest().json(ErrorResponse::<()> {
            message: "No matching JWK found for the given kid".to_owned(),
            error_kind: ErrorKind::BadRequest,
        }))
    }
}

async fn poll_jwks(client: reqwest::Client, certs_url: Url, jwks: Jwks) {
    loop {
        let jwk_ttl = match client.get(certs_url.clone()).send().await {
            Ok(response) => {
                let max_age: Result<Duration, anyhow::Error> = try {
                    let cache_control = response.headers().get_all(reqwest::header::CACHE_CONTROL);
                    let cache_control = headers::CacheControl::decode(&mut cache_control.iter())
                        .map_err(anyhow::Error::msg)?;
                    cache_control
                        .max_age()
                        .ok_or_else(|| anyhow::Error::msg("No max-age directive"))?
                };
                let jwk_ttl = max_age.unwrap_or_else(|err| {
                    log::error!(
                        "Unable to determine cache TTL (will invalidate in {} seconds): {:?}",
                        DEFAULT_JWK_CACHE_TTL,
                        err
                    );
                    Duration::from_secs(DEFAULT_JWK_CACHE_TTL)
                });

                if let Ok(jwk_set) = response.json::<JwkSet>().await {
                    log::info!("Updating a JWK set: {}", certs_url);
                    jwks.update(&certs_url, jwk_set.keys).await;
                }

                jwk_ttl
            }
            Err(err) => {
                log::error!(
                    "Failed to fetch JwtSet (will re-try in {} seconds): {:?}",
                    DEFAULT_JWK_CACHE_TTL,
                    err
                );
                Duration::from_secs(DEFAULT_JWK_CACHE_TTL)
            }
        };

        tokio::time::sleep(jwk_ttl).await
    }
}

#[actix_web::main]
async fn main() -> anyhow::Result<()> {
    // TODO: add sentry support and move panic handler to the utils crate.
    let orig_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        orig_hook(panic_info);

        // A kludge to let sentry send events first and then shutdown.
        std::thread::spawn(|| {
            std::thread::sleep(std::time::Duration::new(1, 0));
            std::process::exit(1);
        });
    }));

    let _guard = sentry::init(sentry::ClientOptions {
        release: sentry::release_name!(),
        ..Default::default()
    });

    let mut builder = env_logger::Builder::from_default_env();
    builder.filter_level(log::LevelFilter::Info).init();

    let config = Config {
        google_certs_url: "https://www.googleapis.com/oauth2/v3/certs"
            .parse()
            .unwrap(),
        auth0_certs_url: "https://muddle-run.eu.auth0.com/.well-known/jwks.json"
            .parse()
            .unwrap(),
        google_client_id: std::env::var("MUDDLE_GOOGLE_CLIENT_ID")
            .expect("Expected MUDDLE_GOOGLE_CLIENT_ID"),
        auth0_client_id: std::env::var("MUDDLE_AUTH0_CLIENT_ID")
            .expect("Expected MUDDLE_AUTH0_CLIENT_ID"),
    };

    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&std::env::var("DATABASE_URL").expect("Expected DATABASE_URL"))
        .await?;

    let jwks = Jwks::new();
    let client = reqwest::Client::new();
    tokio::spawn(poll_jwks(
        client.clone(),
        config.auth0_certs_url.clone(),
        jwks.clone(),
    ));
    tokio::spawn(poll_jwks(
        client,
        config.google_certs_url.clone(),
        jwks.clone(),
    ));

    let data = Data { pool, jwks, config };
    let f = move || {
        let data = data.clone();
        App::new()
            .app_data(web::Data::new(data))
            .service(register)
            .service(link_account)
    };
    HttpServer::new(f)
        .bind("0.0.0.0:8082")?
        .run()
        .await
        .map_err(anyhow::Error::msg)
}
