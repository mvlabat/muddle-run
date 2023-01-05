use crate::Data;
use actix_web::{delete, get, patch, post, web, HttpResponse};
use mr_messages_lib::{
    ErrorKind, ErrorResponse, GetRegisteredUserQuery, LevelData, PatchLevelRequest,
    PostLevelRequest, PostLevelResponse, RegisteredUser,
};
use sqlx::Connection;

#[get("/user")]
pub async fn get_registered_user(
    data: web::Data<Data>,
    body: web::Query<GetRegisteredUserQuery>,
) -> HttpResponse {
    let mut connection = match data.pool.acquire().await {
        Ok(c) => c,
        Err(err) => {
            log::error!("Failed to acquire a connection: {:?}", err);
            return HttpResponse::InternalServerError().finish();
        }
    };

    let GetRegisteredUserQuery { subject, issuer } = body.into_inner();
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

#[post("/levels")]
pub async fn post_level(data: web::Data<Data>, body: web::Json<PostLevelRequest>) -> HttpResponse {
    log::debug!("Posting a level: {:?}", body);

    let PostLevelRequest {
        title,
        user_id,
        data: level_data,
    } = body.into_inner();

    let mut connection = match data.pool.acquire().await {
        Ok(c) => c,
        Err(err) => {
            log::error!("Failed to acquire a connection: {:?}", err);
            return HttpResponse::InternalServerError().finish();
        }
    };

    let (data, parent_id, old_data) = match level_data {
        LevelData::Forked { parent_id } => {
            let data = match get_level_data(&mut connection, parent_id, false).await {
                Ok(data) => data,
                Err(sqlx::Error::RowNotFound) => {
                    return HttpResponse::BadRequest().json(ErrorResponse::<()> {
                        message: "Invalid parent_id: level doesn't exist".to_owned(),
                        error_kind: ErrorKind::NotFound,
                    });
                }
                Err(err) => {
                    log::error!("Failed to get a level: ${:?}", err);
                    return HttpResponse::InternalServerError().finish();
                }
            };
            (data, Some(parent_id), None)
        }
        LevelData::Autosaved {
            autosaved_level_id,
            data,
        } => {
            let old_data = match get_level_data(&mut connection, autosaved_level_id, false).await {
                Ok(data) => {
                    log::debug!(
                        "Autosaving level {} with the following data: {:?}",
                        autosaved_level_id,
                        data
                    );
                    data
                }
                Err(sqlx::Error::RowNotFound) => {
                    return HttpResponse::BadRequest().json(ErrorResponse::<()> {
                        message: "Invalid parent_id: level doesn't exist".to_owned(),
                        error_kind: ErrorKind::NotFound,
                    });
                }
                Err(err) => {
                    log::error!("Failed to get a parent level: ${:?}", err);
                    return HttpResponse::InternalServerError().finish();
                }
            };
            (data, Some(autosaved_level_id), Some(old_data))
        }
        LevelData::Data { data } => (data, None, None),
    };

    let is_autosaved = old_data.is_some();
    let inserted_level: sqlx::Result<PostLevelResponse> = try {
        let mut tx = connection.begin().await?;

        let inserted_level = sqlx::query_as!(
            PostLevelResponse,
            r#"
INSERT INTO levels
(title, user_id, parent_id, data, is_autosaved)
VALUES ($1, $2, $3, $4, $5)
RETURNING id, data, created_at, updated_at
            "#,
            title,
            user_id,
            parent_id,
            old_data.unwrap_or_else(|| data.clone()),
            is_autosaved
        )
        .fetch_one(&mut tx)
        .await?;

        if is_autosaved {
            sqlx::query!("UPDATE levels SET data = $1 WHERE id = $2", data, parent_id)
                .execute(&mut tx)
                .await?;
            sqlx::query!(
                r#"
DELETE FROM levels
WHERE id NOT IN (
    SELECT id
    FROM levels
    WHERE parent_id = $1 AND is_autosaved = TRUE
    ORDER BY id DESC
    LIMIT 5
) AND parent_id = $1 AND is_autosaved = TRUE
                "#,
                parent_id
            )
            .execute(&mut tx)
            .await?;
        }

        tx.commit().await?;
        inserted_level
    };

    match inserted_level {
        Ok(inserted_level) => HttpResponse::Ok().json(&inserted_level),
        Err(err) => {
            log::error!("Failed to insert a level: ${:?}", err);
            HttpResponse::InternalServerError().finish()
        }
    }
}

async fn get_level_data(
    connection: &mut sqlx::PgConnection,
    id: i64,
    is_autosaved: bool,
) -> sqlx::Result<serde_json::Value> {
    log::info!("Getting level ({}) data", id);
    struct JsonValue {
        data: serde_json::Value,
    }
    sqlx::query_as!(
        JsonValue,
        "SELECT data FROM levels WHERE id = $1 AND is_autosaved = $2",
        id,
        is_autosaved
    )
    .fetch_one(connection)
    .await
    .map(|JsonValue { data }| data)
}

#[patch("/levels/{id}")]
pub async fn patch_level(
    data: web::Data<Data>,
    id: web::Path<i64>,
    body: web::Json<PatchLevelRequest>,
) -> HttpResponse {
    let id = id.into_inner();
    let PatchLevelRequest { title, builder_ids } = body.into_inner();

    let mut connection = match data.pool.acquire().await {
        Ok(c) => c,
        Err(err) => {
            log::error!("Failed to acquire a connection: {:?}", err);
            return HttpResponse::InternalServerError().finish();
        }
    };

    struct UserId {
        user_id: i64,
    }
    let result: sqlx::Result<()> = try {
        let mut tx = connection.begin().await?;

        let UserId { user_id } = if let Some(title) = title {
            sqlx::query_as!(
                UserId,
                "UPDATE levels SET title = $1 WHERE id = $2 RETURNING user_id",
                title,
                id
            )
            .fetch_one(&mut tx)
            .await?
        } else {
            sqlx::query_as!(UserId, "SELECT user_id FROM levels WHERE id = $1", id)
                .fetch_one(&mut tx)
                .await?
        };

        match builder_ids {
            Some(builder_ids) if !builder_ids.is_empty() => {
                sqlx::query!(
                    "INSERT INTO level_permissions (user_id, level_id) VALUES ($1, $2)",
                    user_id,
                    id
                )
                .execute(&mut tx)
                .await?;
            }
            _ => {}
        }

        tx.commit().await?;
    };

    match result {
        Ok(()) => HttpResponse::Ok().json(()),
        Err(sqlx::Error::RowNotFound) => HttpResponse::NotFound().json(ErrorResponse::<()> {
            message: "Level doesn't exist".to_owned(),
            error_kind: ErrorKind::NotFound,
        }),
        Err(err) => {
            log::error!("Failed to update a level: ${:?}", err);
            HttpResponse::InternalServerError().finish()
        }
    }
}

#[delete("/levels/{id}")]
pub async fn delete_level(data: web::Data<Data>, id: web::Path<i64>) -> HttpResponse {
    let id = id.into_inner();

    let mut connection = match data.pool.acquire().await {
        Ok(c) => c,
        Err(err) => {
            log::error!("Failed to acquire a connection: {:?}", err);
            return HttpResponse::InternalServerError().finish();
        }
    };

    let result = sqlx::query!("DELETE FROM levels WHERE id = $1", id)
        .execute(&mut connection)
        .await;
    match result {
        Ok(result) => {
            if result.rows_affected() > 0 {
                HttpResponse::Ok().json(())
            } else {
                HttpResponse::NotFound().json(ErrorResponse::<()> {
                    message: "Level doesn't exist".to_owned(),
                    error_kind: ErrorKind::NotFound,
                })
            }
        }
        Err(err) => {
            log::error!("Failed to delete a level: ${:?}", err);
            HttpResponse::InternalServerError().finish()
        }
    }
}
