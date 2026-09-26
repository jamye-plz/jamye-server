use std::{collections::HashSet, io, sync::Arc};

use axum::{
    body::{Body, to_bytes},
    http::{Request, Response, StatusCode, header::AUTHORIZATION},
};
use jamye_server::transport::http::chatrooms::{ChatroomsHttpState, router as chatrooms_router};
use serde_json::Value;
use time::OffsetDateTime;
use tower::ServiceExt;
use uuid::Uuid;

use crate::{
    TestResult,
    chatroom_helpers::{TestAccessVerifier, bearer, harness, insert_user_message, topology},
    postgres_support::TestDatabase,
};

#[tokio::test]
async fn c5_lists_image_and_video_attachments_with_item_cursor_pagination() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let fixture = topology(&pool).await?;
    let harness = harness(pool.clone());
    let old_message = insert_user_message(
        &pool,
        fixture.chatroom_id,
        fixture.owner_id,
        "예전 사진",
        OffsetDateTime::UNIX_EPOCH + time::Duration::hours(1),
    )
    .await?;
    let middle_message = insert_user_message(
        &pool,
        fixture.chatroom_id,
        fixture.owner_id,
        "사진과 영상",
        OffsetDateTime::UNIX_EPOCH + time::Duration::hours(2),
    )
    .await?;
    let latest_message = insert_user_message(
        &pool,
        fixture.chatroom_id,
        fixture.owner_id,
        "최신 사진",
        OffsetDateTime::UNIX_EPOCH + time::Duration::hours(3),
    )
    .await?;
    let foreign_message = insert_user_message(
        &pool,
        fixture.other_chatroom_id,
        fixture.outsider_id,
        "다른 방 사진",
        OffsetDateTime::UNIX_EPOCH + time::Duration::hours(4),
    )
    .await?;

    let poster_upload_id =
        insert_poster_upload(&pool, fixture.owner_id, fixture.chatroom_id).await?;
    let old_image = insert_attachment(
        &pool,
        InsertAttachment {
            user_id: fixture.owner_id,
            chatroom_id: fixture.chatroom_id,
            message_id: old_message,
            content_type: "image/png",
            position: 0,
            poster_upload_id: None,
        },
    )
    .await?;
    let middle_image = insert_attachment(
        &pool,
        InsertAttachment {
            user_id: fixture.owner_id,
            chatroom_id: fixture.chatroom_id,
            message_id: middle_message,
            content_type: "image/jpeg",
            position: 0,
            poster_upload_id: None,
        },
    )
    .await?;
    let middle_video = insert_attachment(
        &pool,
        InsertAttachment {
            user_id: fixture.owner_id,
            chatroom_id: fixture.chatroom_id,
            message_id: middle_message,
            content_type: "video/mp4",
            position: 1,
            poster_upload_id: Some(poster_upload_id),
        },
    )
    .await?;
    let audio = insert_attachment(
        &pool,
        InsertAttachment {
            user_id: fixture.owner_id,
            chatroom_id: fixture.chatroom_id,
            message_id: middle_message,
            content_type: "audio/mpeg",
            position: 2,
            poster_upload_id: None,
        },
    )
    .await?;
    let latest_image = insert_attachment(
        &pool,
        InsertAttachment {
            user_id: fixture.owner_id,
            chatroom_id: fixture.chatroom_id,
            message_id: latest_message,
            content_type: "image/webp",
            position: 0,
            poster_upload_id: None,
        },
    )
    .await?;
    let foreign_image = insert_attachment(
        &pool,
        InsertAttachment {
            user_id: fixture.outsider_id,
            chatroom_id: fixture.other_chatroom_id,
            message_id: foreign_message,
            content_type: "image/png",
            position: 0,
            poster_upload_id: None,
        },
    )
    .await?;

    let router = chatrooms_router(ChatroomsHttpState::new(
        harness.service,
        Arc::new(TestAccessVerifier),
    ));
    let path = format!("/api/v1/chatrooms/{}/media", fixture.chatroom_id);

    let unauthorized = router
        .clone()
        .oneshot(empty_request("GET", &path, None)?)
        .await?;
    assert_error(
        unauthorized,
        StatusCode::UNAUTHORIZED,
        "authentication_required",
    )
    .await?;

    for request in [
        empty_request("GET", &path, Some(fixture.outsider_id))?,
        empty_request(
            "GET",
            &format!("/api/v1/chatrooms/{}/media", Uuid::new_v4()),
            Some(fixture.owner_id),
        )?,
    ] {
        let denied = router.clone().oneshot(request).await?;
        assert_error(denied, StatusCode::FORBIDDEN, "membership_required").await?;
    }

    let first_page = router
        .clone()
        .oneshot(empty_request(
            "GET",
            &format!("{path}?limit=2"),
            Some(fixture.owner_id),
        )?)
        .await?;
    assert_eq!(first_page.status(), StatusCode::OK);
    let first_page = response_json(first_page).await?;
    assert_item_ids(&first_page, &[latest_image.id, middle_image.id])?;
    assert_eq!(
        first_page["next_cursor"],
        middle_image.id.to_string(),
        "cursor must be the last returned media item id"
    );
    assert_eq!(
        first_page["items"][0]["message_id"],
        latest_message.to_string()
    );
    assert_eq!(
        first_page["items"][0]["message_created_at"].as_str(),
        Some("1970-01-01T03:00:00Z")
    );

    let second_page = router
        .clone()
        .oneshot(empty_request(
            "GET",
            &format!("{path}?limit=2&before={}", middle_image.id),
            Some(fixture.owner_id),
        )?)
        .await?;
    assert_eq!(second_page.status(), StatusCode::OK);
    let second_page = response_json(second_page).await?;
    assert_item_ids(&second_page, &[middle_video.id, old_image.id])?;
    assert_eq!(second_page["next_cursor"], Value::Null);
    assert_eq!(
        second_page["items"][0]["poster_media_id"],
        poster_upload_id.to_string()
    );

    let all_ids = first_page["items"]
        .as_array()
        .into_iter()
        .flatten()
        .chain(second_page["items"].as_array().into_iter().flatten())
        .map(|item| {
            item["id"]
                .as_str()
                .ok_or_else(|| io::Error::other("media id missing"))
                .and_then(|id| Uuid::try_parse(id).map_err(io::Error::other))
        })
        .collect::<Result<Vec<_>, _>>()?;
    assert_eq!(
        all_ids,
        vec![
            latest_image.id,
            middle_image.id,
            middle_video.id,
            old_image.id
        ]
    );
    assert_eq!(all_ids.iter().copied().collect::<HashSet<_>>().len(), 4);
    assert!(
        !all_ids.contains(&audio.id),
        "audio attachments are not listed"
    );

    for uri in [
        format!("{path}?before=not-a-uuid"),
        format!("{path}?before={}", foreign_image.id),
        format!("{path}?limit=0"),
        format!("{path}?limit=101"),
        format!("{path}?limit=2&limit=2"),
        format!("{path}?unexpected=1"),
    ] {
        let response = router
            .clone()
            .oneshot(empty_request("GET", &uri, Some(fixture.owner_id))?)
            .await?;
        assert_error(
            response,
            StatusCode::UNPROCESSABLE_ENTITY,
            "request_validation_failed",
        )
        .await?;
    }

    pool.close().await;
    database.dispose().await
}

#[derive(Clone, Copy)]
struct InsertAttachment {
    user_id: Uuid,
    chatroom_id: Uuid,
    message_id: Uuid,
    content_type: &'static str,
    position: i32,
    poster_upload_id: Option<Uuid>,
}

#[derive(Clone, Copy)]
struct InsertedAttachment {
    id: Uuid,
}

async fn insert_poster_upload(
    pool: &sqlx::PgPool,
    user_id: Uuid,
    chatroom_id: Uuid,
) -> TestResult<Uuid> {
    let id = Uuid::new_v4();
    let now = OffsetDateTime::now_utc();
    let created_at = now - time::Duration::minutes(4);
    let confirmed_at = now - time::Duration::minutes(3);
    sqlx::query(
        "INSERT INTO media_uploads \
             (id, user_id, object_key, scope, target_id, content_type, byte_size, \
              status, confirmed_at, expires_at, created_at) \
         VALUES ($1, $2, $3, 'chat', $4, 'image/jpeg', 1024, \
                 'confirmed', $5, $6, $7)",
    )
    .bind(id)
    .bind(user_id)
    .bind(format!("chat/{chatroom_id}/{id}-poster"))
    .bind(chatroom_id)
    .bind(confirmed_at)
    .bind(now + time::Duration::hours(1))
    .bind(created_at)
    .execute(pool)
    .await?;
    Ok(id)
}

async fn insert_attachment(
    pool: &sqlx::PgPool,
    attachment: InsertAttachment,
) -> TestResult<InsertedAttachment> {
    let upload_id = Uuid::new_v4();
    let attachment_id = Uuid::new_v4();
    let now = OffsetDateTime::now_utc();
    let created_at = now - time::Duration::minutes(4);
    let confirmed_at = now - time::Duration::minutes(3);
    let consumed_at = now - time::Duration::minutes(2);
    let object_key = format!(
        "chat/{}/{}-{}",
        attachment.chatroom_id, upload_id, attachment.position
    );
    sqlx::query(
        "INSERT INTO media_uploads \
             (id, user_id, object_key, scope, target_id, content_type, byte_size, \
              status, bound_message_id, confirmed_at, consumed_at, expires_at, created_at, \
              poster_upload_id) \
         VALUES ($1, $2, $3, 'chat', $4, $5, 4096, \
                 'bound', $6, $7, $8, $9, $10, $11)",
    )
    .bind(upload_id)
    .bind(attachment.user_id)
    .bind(&object_key)
    .bind(attachment.chatroom_id)
    .bind(attachment.content_type)
    .bind(attachment.message_id)
    .bind(confirmed_at)
    .bind(consumed_at)
    .bind(now + time::Duration::hours(1))
    .bind(created_at)
    .bind(attachment.poster_upload_id)
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO message_media \
             (id, message_id, media_upload_id, type, object_key, width, height, byte_size, \
              duration, position, filename, created_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, 4096, $8, $9, $10, $11)",
    )
    .bind(attachment_id)
    .bind(attachment.message_id)
    .bind(upload_id)
    .bind(attachment.content_type)
    .bind(&object_key)
    .bind((attachment.content_type.starts_with("image/")).then_some(320))
    .bind((attachment.content_type.starts_with("image/")).then_some(240))
    .bind((attachment.content_type.starts_with("video/")).then_some(15))
    .bind(attachment.position)
    .bind(format!("media-{}.bin", attachment.position))
    .bind(consumed_at)
    .execute(pool)
    .await?;
    Ok(InsertedAttachment { id: attachment_id })
}

fn assert_item_ids(page: &Value, expected: &[Uuid]) -> TestResult {
    let actual = page["items"]
        .as_array()
        .ok_or_else(|| io::Error::other("items must be an array"))?
        .iter()
        .map(|item| {
            item["id"]
                .as_str()
                .ok_or_else(|| io::Error::other("item id must be a string"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    assert_eq!(
        actual,
        expected.iter().map(Uuid::to_string).collect::<Vec<_>>()
    );
    Ok(())
}

fn empty_request(method: &str, uri: &str, user_id: Option<Uuid>) -> TestResult<Request<Body>> {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(user_id) = user_id {
        builder = builder.header(AUTHORIZATION, bearer(user_id));
    }
    Ok(builder.body(Body::empty())?)
}

async fn assert_error(
    response: Response<Body>,
    status: StatusCode,
    code: &str,
) -> TestResult<Value> {
    assert_eq!(response.status(), status);
    let body = response_json(response).await?;
    assert_eq!(body["error"]["code"], code);
    assert!(body["error"]["request_id"].as_str().is_some());
    assert_eq!(body["error"]["details"], Value::Null);
    Ok(body)
}

async fn response_json(response: Response<Body>) -> TestResult<Value> {
    let bytes = to_bytes(response.into_body(), 64 * 1024).await?;
    if bytes.is_empty() {
        return Err(io::Error::other("response body was empty").into());
    }
    Ok(serde_json::from_slice(&bytes)?)
}
