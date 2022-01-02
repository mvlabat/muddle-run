use crate::Data;
use actix_web::{get, web, HttpResponse};
use mr_messages_lib::{ErrorKind, ErrorResponse, GetUserRequest, RegisteredUser};

#[get("/users")]
pub async fn get_user(data: web::Data<Data>, body: web::Json<GetUserRequest>) -> HttpResponse {
    let mut connection = match data.pool.acquire().await {
        Ok(c) => c,
        Err(err) => {
            log::error!("Failed to acquire a connection: {:?}", err);
            return HttpResponse::InternalServerError().finish();
        }
    };

    let GetUserRequest { subject, issuer } = body.into_inner();
    let user: Result<Option<RegisteredUser>, sqlx::Error> = sqlx::query_as!(
        RegisteredUser,
        "
SELECT u.id, u.email, u.display_name, u.created_at, u.updated_at
FROM users u
JOIN openids AS o ON u.id = o.user_id
WHERE o.subject = $1 AND o.issuer = $2
        ",
        subject,
        issuer,
    )
    .fetch_optional(&mut connection)
    .await;

    match user {
        Ok(Some(user)) => HttpResponse::Ok().json(&user),
        Ok(None) => HttpResponse::NotFound().json(ErrorResponse::<()> {
            message: "User doesn't exist".to_owned(),
            error_kind: ErrorKind::NotFound,
        }),
        Err(err) => {
            log::error!("Failed to get a user: ${:?}", err);
            HttpResponse::InternalServerError().finish()
        }
    }
}
