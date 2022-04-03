#![feature(let_else)]
#![feature(try_blocks)]

mod private;
mod public;

use actix_web::{web, App, HttpResponse, HttpServer};
use futures::{select, FutureExt};
use jwt_compact::Token;
use mr_messages_lib::{ErrorKind, ErrorResponse};
use mr_utils_lib::{
    jwks::{poll_jwks, InvalidTokenError, Jwks},
    JwtAuthClaims,
};
use reqwest::Url;
use sqlx::postgres::PgPoolOptions;

#[derive(Clone)]
pub struct Data {
    pool: sqlx::PgPool,
    jwks: Jwks,
    config: Config,
}

#[derive(Clone)]
struct Config {
    google_certs_url: Url,
    auth0_certs_url: Url,
    google_web_client_id: String,
    google_desktop_client_id: String,
    auth0_client_id: String,
}

async fn decode_token_helper(
    data: &Data,
    token: &str,
    kind: &str,
) -> Result<Token<JwtAuthClaims>, HttpResponse> {
    data.jwks
        .decode(
            token,
            &[
                &data.config.google_web_client_id,
                &data.config.google_desktop_client_id,
                &data.config.auth0_client_id,
            ],
        )
        .await
        .map_err(|err| match err {
            InvalidTokenError::Malformed(err) => {
                log::warn!("Failed to parse JWT (kind: {}): {:?}", kind, err);
                HttpResponse::BadRequest().json(ErrorResponse::<()> {
                    message: format!("Failed to parse JWT (kind: {})", kind),
                    error_kind: ErrorKind::Unauthorized,
                })
            }
            InvalidTokenError::KeyIdMissing => {
                log::warn!("Token doesn't have a `kid` header field (kind: {})", kind);
                HttpResponse::BadRequest().json(ErrorResponse::<()> {
                    message: format!("Token doesn't have a `kid` header field (kind: {})", kind),
                    error_kind: ErrorKind::Unauthorized,
                })
            }
            InvalidTokenError::Invalid(err) => {
                log::warn!("Invalid or expired JWT (kind: {}): {:?}", kind, err);
                HttpResponse::BadRequest().json(ErrorResponse::<()> {
                    message: format!("Invalid or expired JWT (kind: {})", kind),
                    error_kind: ErrorKind::BadRequest,
                })
            }
            InvalidTokenError::InvalidAudience => {
                log::warn!("Invalid audience (kind: {})", kind);
                HttpResponse::BadRequest().json(ErrorResponse::<()> {
                    message: format!("Invalid audience (kind: {})", kind),
                    error_kind: ErrorKind::BadRequest,
                })
            }
            InvalidTokenError::UnknownSigner => {
                log::warn!("Unknown signer (kind: {})", kind);
                HttpResponse::BadRequest().json(ErrorResponse::<()> {
                    message: format!("Unknown signer (kind: {})", kind),
                    error_kind: ErrorKind::BadRequest,
                })
            }
        })
}

#[actix_web::main]
async fn main() -> anyhow::Result<()> {
    mr_utils_lib::env::load_env();

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
        google_web_client_id: std::env::var("MUDDLE_GOOGLE_WEB_CLIENT_ID")
            .expect("Expected MUDDLE_GOOGLE_WEB_CLIENT_ID"),
        google_desktop_client_id: std::env::var("MUDDLE_GOOGLE_DESKTOP_CLIENT_ID")
            .expect("Expected MUDDLE_GOOGLE_DESKTOP_CLIENT_ID"),
        auth0_client_id: std::env::var("MUDDLE_AUTH0_CLIENT_ID")
            .expect("Expected MUDDLE_AUTH0_CLIENT_ID"),
    };

    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&std::env::var("DATABASE_URL").expect("Expected DATABASE_URL"))
        .await?;

    sqlx::migrate!().run(&pool).await?;

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

    let public_data = data.clone();
    let public = move || {
        let cors = actix_cors::Cors::default()
            .allowed_origin("http://muddle.run")
            .allow_any_method()
            .allow_any_header();
        let data = public_data.clone();
        App::new()
            .wrap(cors)
            .app_data(web::Data::new(data))
            .service(public::get_user)
            .service(public::register)
            .service(public::link_account)
            .service(public::patch_user)
            .service(public::get_levels)
    };
    let mut public_server = HttpServer::new(public)
        .workers(2)
        .bind("0.0.0.0:8082")?
        .run()
        .fuse();

    let private = move || {
        let data = data.clone();
        App::new()
            .app_data(web::Data::new(data))
            .service(private::get_registered_user)
            .service(private::get_level)
            .service(private::post_level)
            .service(private::patch_level)
            .service(private::delete_level)
    };
    let mut private_server = HttpServer::new(private)
        .workers(3)
        .bind("0.0.0.0:8083")?
        .run()
        .fuse();

    select! {
        r = public_server => {
            log::error!("Public server shutdown: {:?}", r);
        }
        r = private_server => {
            log::error!("Private server shutdown: {:?}", r);
        }
    }

    Ok(())
}
