#![feature(let_else)]
#![feature(try_blocks)]

use actix_web::{post, web, App, HttpResponse, HttpServer, Responder};
use headers::Header;
use jsonwebtoken::{
    decode, decode_header,
    jwk::{AlgorithmParameters, Jwk, JwkSet},
    DecodingKey, TokenData, Validation,
};
use mr_messages_lib::{ErrorResponse, RegisterRequest, RegisterResponse, RegisteredUser};
use reqwest::Url;
use serde::Deserialize;
use sqlx::{postgres::PgPoolOptions, types::chrono, Connection};
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
async fn register(data: web::Data<Data>, params: web::Json<RegisterRequest>) -> impl Responder {
    let RegisterRequest { jwt } = params.into_inner();

    let header = decode_header(&jwt).unwrap();
    let kid = match header.kid {
        Some(k) => k,
        None => {
            return HttpResponse::BadRequest().json(ErrorResponse::new(
                "Token doesn't have a `kid` header field".to_owned(),
            ))
        }
    };
    let decoded_token = if let Some(key) = data.get_ref().jwks.get(&kid).await {
        match key.algorithm {
            AlgorithmParameters::RSA(ref rsa) => {
                let decoding_key = DecodingKey::from_rsa_components(&rsa.n, &rsa.e).unwrap();
                let mut validation = Validation::new(key.common.algorithm.unwrap());
                validation.set_audience(&[
                    data.config.auth0_client_id.clone(),
                    data.config.google_client_id.clone(),
                ]);
                match decode::<JwtClaims>(&jwt, &decoding_key, &validation) {
                    Ok(t) => t,
                    Err(err) => {
                        log::warn!("Invalid Jwt: {:?}", err);
                        return HttpResponse::BadRequest()
                            .json(ErrorResponse::new("Invalid Jwt".to_owned()));
                    }
                }
            }
            _ => unreachable!("this should be an RSA"),
        }
    } else {
        return HttpResponse::BadRequest().json(ErrorResponse::new(
            "No matching JWK found for the given kid".to_owned(),
        ));
    };

    let connection = match data.pool.acquire().await {
        Ok(c) => c,
        Err(err) => {
            log::error!("Failed to acquire a connection: {:?}", err);
            return HttpResponse::InternalServerError().finish();
        }
    };
    let (user, is_new) = match insert_user(connection, decoded_token).await {
        Ok(user) => user,
        Err(err) => {
            log::error!("Failed to upsert a user: {:?}", err);
            return HttpResponse::InternalServerError().finish();
        }
    };

    HttpResponse::Ok().json(RegisterResponse { user, is_new })
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

struct NewUser {
    id: i64,
    created_at: chrono::NaiveDateTime,
}

async fn insert_user(
    mut connection: sqlx::pool::PoolConnection<sqlx::Postgres>,
    user_data: TokenData<JwtClaims>,
) -> sqlx::Result<(RegisteredUser, bool)> {
    let registered_user = sqlx::query_as!(
        RegisteredUser,
        "
SELECT u.id, u.email, u.username, u.created_at, u.updated_at
FROM users u
JOIN openids AS o ON u.id = o.user_id
WHERE o.issuer = $1 AND o.subject = $2
        ",
        user_data.claims.iss,
        user_data.claims.sub,
    )
    .fetch_optional(&mut connection)
    .await?;

    if let Some(registered_user) = registered_user {
        return Ok((registered_user, false));
    }

    let email = user_data.claims.email.clone();

    let mut transaction = connection.begin().await?;
    let NewUser { id, created_at } = sqlx::query_as!(
        NewUser,
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

    Ok((
        RegisteredUser {
            id,
            email,
            username: None,
            created_at,
            updated_at: created_at,
        },
        true,
    ))
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
        App::new().app_data(web::Data::new(data)).service(register)
    };
    HttpServer::new(f)
        .bind("0.0.0.0:8082")?
        .run()
        .await
        .map_err(anyhow::Error::msg)
}
