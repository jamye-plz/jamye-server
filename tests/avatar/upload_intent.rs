//! U4 `POST /api/v1/me/avatar/uploads`.

use axum::http::StatusCode;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    TestResult,
    support::{
        Harness, UPLOAD_LIMIT, assert_error, count_rows, header, insert_user, json_body, json_post,
        object_key, queued_keys, row_status,
    },
};

const URI: &str = "/api/v1/me/avatar/uploads";

#[tokio::test]
async fn u4_returns_a_server_minted_uuid_v4_upload_and_a_900_second_put() -> TestResult {
    let harness = Harness::new().await?;
    let result: TestResult = async {
        let actor = insert_user(&harness.pool).await?;
        let response = harness
            .send(json_post(
                URI,
                Some(actor),
                &json!({"content_type": "image/jpeg", "byte_size": 2048}),
            )?)
            .await?;
        assert_eq!(response.status(), StatusCode::CREATED);
        let body = json_body(response).await?;
        assert_eq!(body.as_object().map(|object| object.len()), Some(2));
        let upload_id = Uuid::try_parse(body["upload_id"].as_str().ok_or("upload_id missing")?)?;
        assert_eq!(upload_id.get_version_num(), 4);
        assert_eq!(body["presigned_put"]["expires_in"], 900);
        let url = body["presigned_put"]["url"]
            .as_str()
            .ok_or("presigned url missing")?;
        assert!(url.starts_with("https://"));
        assert_eq!(
            body["presigned_put"].as_object().map(|object| object.len()),
            Some(2)
        );

        let requests = harness.storage.presigned_requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].object_key, object_key(actor, upload_id));
        assert_eq!(requests[0].content_type, "image/jpeg");
        assert_eq!(requests[0].byte_size, 2048);
        assert_eq!(requests[0].expires_in.as_secs(), 900);

        let (status, stored_key, byte_size, expires_in_window) =
            sqlx::query_as::<_, (String, String, i32, bool)>(
                "SELECT status, object_key, byte_size, \
                        expires_at > clock_timestamp() + interval '14 minutes' \
                        AND expires_at <= clock_timestamp() + interval '15 minutes' \
                 FROM user_avatar_uploads WHERE id = $1 AND user_id = $2",
            )
            .bind(upload_id)
            .bind(actor)
            .fetch_one(&harness.pool)
            .await?;
        assert_eq!(status, "pending");
        assert_eq!(stored_key, object_key(actor, upload_id));
        assert_eq!(byte_size, 2048);
        assert!(expires_in_window, "pending expiry is not 15 minutes");
        Ok(())
    }
    .await;
    harness.finish(result).await
}

#[tokio::test]
async fn u4_validates_the_declared_jpeg_and_one_mebibyte_cap_with_422() -> TestResult {
    let harness = Harness::new().await?;
    let result: TestResult = async {
        let actor = insert_user(&harness.pool).await?;
        let invalid: [Value; 9] = [
            json!({"content_type": "image/png", "byte_size": 1024}),
            json!({"content_type": "image/webp", "byte_size": 1024}),
            json!({"content_type": "image/jpeg", "byte_size": 0}),
            json!({"content_type": "image/jpeg", "byte_size": 1_048_577}),
            json!({"content_type": "image/jpeg", "byte_size": -1}),
            json!({"content_type": "image/jpeg", "byte_size": "1024"}),
            json!({"content_type": "image/jpeg"}),
            json!({"byte_size": 1024}),
            json!({"content_type": "image/jpeg", "byte_size": 1024, "filename": "me.jpg"}),
        ];
        for body in &invalid {
            let response = harness.send(json_post(URI, Some(actor), body)?).await?;
            assert_error(
                response,
                StatusCode::UNPROCESSABLE_ENTITY,
                "request_validation_failed",
            )
            .await?;
        }
        assert_eq!(count_rows(&harness.pool, actor, "pending").await?, 0);
        assert!(harness.storage.presigned_requests().is_empty());

        for byte_size in [1_u64, 1_048_576] {
            let upload_id = harness.create_intent(actor, byte_size).await?;
            assert_eq!(
                row_status(&harness.pool, upload_id).await?.as_deref(),
                Some("pending")
            );
        }
        Ok(())
    }
    .await;
    harness.finish(result).await
}

#[tokio::test]
async fn u4_requires_a_bearer_token() -> TestResult {
    let harness = Harness::new().await?;
    let result: TestResult = async {
        let response = harness
            .send(json_post(
                URI,
                None,
                &json!({"content_type": "image/jpeg", "byte_size": 1024}),
            )?)
            .await?;
        assert_error(
            response,
            StatusCode::UNAUTHORIZED,
            "authentication_required",
        )
        .await
    }
    .await;
    harness.finish(result).await
}

#[tokio::test]
async fn u4_reuses_the_media_presign_limiter_with_a_separate_counter_key() -> TestResult {
    let harness = Harness::new().await?;
    let result: TestResult = async {
        let actor = insert_user(&harness.pool).await?;
        harness.create_intent(actor, 1024).await?;
        let requests = harness.requests_for("media_upload_presign");
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].limit, UPLOAD_LIMIT);
        assert_eq!(requests[0].window.as_secs(), 60);
        // The chat presign subject is `user:{id}:scope:chat:target:{id}`; avatar counts apart.
        assert!(requests[0].subject.contains("avatar"));
        assert!(
            !requests[0].subject.contains("scope:chat"),
            "avatar presign shares the chat counter key"
        );

        harness.limiter.deny("media_upload_presign");
        let response = harness
            .send(json_post(
                URI,
                Some(actor),
                &json!({"content_type": "image/jpeg", "byte_size": 1024}),
            )?)
            .await?;
        assert_eq!(header(&response, "retry-after").as_deref(), Some("7"));
        assert_error(
            response,
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limit_exceeded",
        )
        .await?;
        assert_eq!(count_rows(&harness.pool, actor, "pending").await?, 1);
        Ok(())
    }
    .await;
    harness.finish(result).await
}

#[tokio::test]
async fn u4_releases_the_previous_pending_row_and_queues_its_object_for_deletion() -> TestResult {
    let harness = Harness::new().await?;
    let result: TestResult = async {
        let actor = insert_user(&harness.pool).await?;
        let first = harness.create_intent(actor, 1024).await?;
        assert!(queued_keys(&harness.pool).await?.is_empty());

        let second = harness.create_intent(actor, 2048).await?;
        assert_ne!(first, second);
        assert_eq!(
            row_status(&harness.pool, first).await?.as_deref(),
            Some("released")
        );
        assert!(crate::support::released_at_is_set(&harness.pool, first).await?);
        assert_eq!(
            row_status(&harness.pool, second).await?.as_deref(),
            Some("pending")
        );
        assert_eq!(count_rows(&harness.pool, actor, "pending").await?, 1);
        assert_eq!(
            queued_keys(&harness.pool).await?,
            vec![object_key(actor, first)]
        );

        // Another user's pending upload is never touched.
        let other = insert_user(&harness.pool).await?;
        let other_upload = harness.create_intent(other, 1024).await?;
        assert_eq!(
            row_status(&harness.pool, other_upload).await?.as_deref(),
            Some("pending")
        );
        assert_eq!(
            row_status(&harness.pool, second).await?.as_deref(),
            Some("pending")
        );
        Ok(())
    }
    .await;
    harness.finish(result).await
}

#[tokio::test]
async fn u4_reports_a_storage_outage_as_503_without_leaving_a_pending_row() -> TestResult {
    let harness = Harness::new().await?;
    let result: TestResult = async {
        let actor = insert_user(&harness.pool).await?;
        harness.storage.set_unavailable(true);
        let response = harness
            .send(json_post(
                URI,
                Some(actor),
                &json!({"content_type": "image/jpeg", "byte_size": 1024}),
            )?)
            .await?;
        assert_error(
            response,
            StatusCode::SERVICE_UNAVAILABLE,
            "object_storage_degraded",
        )
        .await?;
        assert_eq!(count_rows(&harness.pool, actor, "pending").await?, 0);
        Ok(())
    }
    .await;
    harness.finish(result).await
}
