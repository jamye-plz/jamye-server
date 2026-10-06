//! U5 `POST /api/v1/me/avatar/uploads/{upload_id}/finalize`.

use axum::http::StatusCode;
use serde_json::json;
use uuid::Uuid;

use crate::{
    TestResult,
    support::{
        BASE, Harness, assert_error, count_rows, insert_row, insert_user, jpeg_bytes, json_body,
        json_post, object_key, queued_keys, row_status, user_avatar_url,
    },
};

fn finalize_uri(upload_id: Uuid) -> String {
    format!("/api/v1/me/avatar/uploads/{upload_id}/finalize")
}

#[tokio::test]
async fn u5_activates_the_upload_and_returns_the_user_with_the_public_url() -> TestResult {
    let harness = Harness::new().await?;
    let result: TestResult = async {
        let actor = insert_user(&harness.pool).await?;
        let upload_id = harness.create_intent(actor, 2048).await?;
        harness.storage.put(
            &object_key(actor, upload_id),
            "image/jpeg",
            jpeg_bytes(2048),
        );
        let limiter_calls = harness.limiter.requests().len();

        let response = harness.finalize(actor, upload_id).await?;
        assert_eq!(response.status(), StatusCode::OK);
        let body = json_body(response).await?;
        let expected_url = format!("{BASE}/api/v1/avatars/{upload_id}");
        assert_eq!(body["id"], actor.to_string());
        assert_eq!(body["provider"], "kakao");
        assert_eq!(body["avatar_url"], expected_url);
        for field in ["nickname", "created_at"] {
            assert!(body.get(field).is_some(), "User response omitted {field}");
        }
        assert!(
            !expected_url.contains(&actor.to_string()),
            "public URL exposes the user id"
        );

        assert_eq!(
            row_status(&harness.pool, upload_id).await?.as_deref(),
            Some("active")
        );
        assert_eq!(
            user_avatar_url(&harness.pool, actor).await?.as_deref(),
            Some(expected_url.as_str())
        );
        assert!(queued_keys(&harness.pool).await?.is_empty());
        assert_eq!(
            harness.limiter.requests().len(),
            limiter_calls,
            "finalize consulted a rate limiter"
        );

        // Idempotent: the same upload finalizes again with the same User and no side effect.
        let again = harness.finalize(actor, upload_id).await?;
        assert_eq!(again.status(), StatusCode::OK);
        assert_eq!(json_body(again).await?, body);
        assert!(queued_keys(&harness.pool).await?.is_empty());
        assert_eq!(count_rows(&harness.pool, actor, "active").await?, 1);
        Ok(())
    }
    .await;
    harness.finish(result).await
}

#[tokio::test]
async fn u5_replaces_the_previous_active_row_and_queues_its_object() -> TestResult {
    let harness = Harness::new().await?;
    let result: TestResult = async {
        let actor = insert_user(&harness.pool).await?;
        let first = harness.upload_and_finalize(actor).await?;
        let second = harness.upload_and_finalize(actor).await?;

        assert_eq!(
            row_status(&harness.pool, first).await?.as_deref(),
            Some("released")
        );
        assert!(crate::support::released_at_is_set(&harness.pool, first).await?);
        assert_eq!(
            row_status(&harness.pool, second).await?.as_deref(),
            Some("active")
        );
        assert_eq!(count_rows(&harness.pool, actor, "active").await?, 1);
        assert_eq!(
            user_avatar_url(&harness.pool, actor).await?,
            Some(format!("{BASE}/api/v1/avatars/{second}"))
        );
        assert_eq!(
            queued_keys(&harness.pool).await?,
            vec![object_key(actor, first)]
        );
        Ok(())
    }
    .await;
    harness.finish(result).await
}

#[tokio::test]
async fn u5_treats_unknown_and_foreign_uploads_alike_as_404() -> TestResult {
    let harness = Harness::new().await?;
    let result: TestResult = async {
        let owner = insert_user(&harness.pool).await?;
        let intruder = insert_user(&harness.pool).await?;
        let upload_id = harness.create_intent(owner, 1024).await?;
        harness.storage.put(
            &object_key(owner, upload_id),
            "image/jpeg",
            jpeg_bytes(1024),
        );

        for (actor, target) in [(intruder, upload_id), (owner, Uuid::new_v4())] {
            let response = harness.finalize(actor, target).await?;
            assert_error(response, StatusCode::NOT_FOUND, "avatar_upload_not_found").await?;
        }
        assert_eq!(
            row_status(&harness.pool, upload_id).await?.as_deref(),
            Some("pending")
        );
        assert_eq!(
            user_avatar_url(&harness.pool, owner).await?.as_deref(),
            Some("https://images.example/original.png")
        );
        Ok(())
    }
    .await;
    harness.finish(result).await
}

#[tokio::test]
async fn u5_rejects_released_superseded_and_expired_uploads_with_409() -> TestResult {
    let harness = Harness::new().await?;
    let result: TestResult = async {
        let actor = insert_user(&harness.pool).await?;

        let released = Uuid::new_v4();
        insert_row(&harness.pool, actor, released, "released").await?;
        let response = harness.finalize(actor, released).await?;
        assert_error(response, StatusCode::CONFLICT, "avatar_upload_not_pending").await?;

        // A newer intent supersedes (releases) the older pending upload.
        let superseded = harness.create_intent(actor, 1024).await?;
        let newer = harness.create_intent(actor, 1024).await?;
        harness.storage.put(
            &object_key(actor, superseded),
            "image/jpeg",
            jpeg_bytes(1024),
        );
        let response = harness.finalize(actor, superseded).await?;
        assert_error(response, StatusCode::CONFLICT, "avatar_upload_not_pending").await?;

        // Expired pending uploads cannot be confirmed.
        sqlx::query(
            "UPDATE user_avatar_uploads \
             SET created_at = clock_timestamp() - interval '1 hour', \
                 expires_at = clock_timestamp() - interval '1 minute' \
             WHERE id = $1",
        )
        .bind(newer)
        .execute(&harness.pool)
        .await?;
        harness
            .storage
            .put(&object_key(actor, newer), "image/jpeg", jpeg_bytes(1024));
        let response = harness.finalize(actor, newer).await?;
        assert_error(response, StatusCode::CONFLICT, "avatar_upload_not_pending").await?;
        assert_eq!(count_rows(&harness.pool, actor, "active").await?, 0);
        assert_eq!(
            user_avatar_url(&harness.pool, actor).await?.as_deref(),
            Some("https://images.example/original.png")
        );
        Ok(())
    }
    .await;
    harness.finish(result).await
}

#[tokio::test]
async fn u5_rejects_a_missing_or_mismatched_object_with_422_and_changes_nothing() -> TestResult {
    let harness = Harness::new().await?;
    let result: TestResult = async {
        let actor = insert_user(&harness.pool).await?;
        let png_signature = {
            let mut bytes = jpeg_bytes(1024);
            bytes[..4].copy_from_slice(&[0x89, 0x50, 0x4E, 0x47]);
            bytes
        };

        // (declared size, stored content type, stored bytes or none)
        let cases: [(u64, &str, Option<Vec<u8>>); 5] = [
            (1024, "image/jpeg", None),
            (2048, "image/jpeg", Some(jpeg_bytes(1024))),
            (1024, "image/png", Some(jpeg_bytes(1024))),
            (1024, "image/jpeg", Some(png_signature)),
            (1024, "image/jpeg", Some(Vec::new())),
        ];
        for (declared, content_type, stored) in cases {
            let upload_id = harness.create_intent(actor, declared).await?;
            if let Some(bytes) = stored {
                harness
                    .storage
                    .put(&object_key(actor, upload_id), content_type, bytes);
            }
            let response = harness.finalize(actor, upload_id).await?;
            assert_error(
                response,
                StatusCode::UNPROCESSABLE_ENTITY,
                "avatar_object_invalid",
            )
            .await?;
            assert_eq!(
                row_status(&harness.pool, upload_id).await?.as_deref(),
                Some("pending")
            );
        }
        assert_eq!(count_rows(&harness.pool, actor, "active").await?, 0);
        // Only the four superseded pending uploads were queued; failed finalizes queue nothing.
        assert_eq!(queued_keys(&harness.pool).await?.len(), 4);
        assert_eq!(
            user_avatar_url(&harness.pool, actor).await?.as_deref(),
            Some("https://images.example/original.png")
        );
        Ok(())
    }
    .await;
    harness.finish(result).await
}

#[tokio::test]
async fn u5_reports_a_storage_outage_as_503_and_keeps_the_upload_pending() -> TestResult {
    let harness = Harness::new().await?;
    let result: TestResult = async {
        let actor = insert_user(&harness.pool).await?;
        let upload_id = harness.create_intent(actor, 1024).await?;
        harness.storage.put(
            &object_key(actor, upload_id),
            "image/jpeg",
            jpeg_bytes(1024),
        );
        harness.storage.set_unavailable(true);
        let response = harness.finalize(actor, upload_id).await?;
        assert_error(
            response,
            StatusCode::SERVICE_UNAVAILABLE,
            "object_storage_degraded",
        )
        .await?;
        assert_eq!(
            row_status(&harness.pool, upload_id).await?.as_deref(),
            Some("pending")
        );
        Ok(())
    }
    .await;
    harness.finish(result).await
}

#[tokio::test]
async fn u5_requires_a_bearer_token_and_an_empty_json_object_body() -> TestResult {
    let harness = Harness::new().await?;
    let result: TestResult = async {
        let actor = insert_user(&harness.pool).await?;
        let upload_id = harness.create_intent(actor, 1024).await?;
        harness.storage.put(
            &object_key(actor, upload_id),
            "image/jpeg",
            jpeg_bytes(1024),
        );

        let response = harness
            .send(json_post(&finalize_uri(upload_id), None, &json!({}))?)
            .await?;
        assert_error(
            response,
            StatusCode::UNAUTHORIZED,
            "authentication_required",
        )
        .await?;

        let response = harness
            .send(json_post(
                &finalize_uri(upload_id),
                Some(actor),
                &json!({"width": 512}),
            )?)
            .await?;
        assert_error(
            response,
            StatusCode::UNPROCESSABLE_ENTITY,
            "request_validation_failed",
        )
        .await?;
        assert_eq!(
            row_status(&harness.pool, upload_id).await?.as_deref(),
            Some("pending")
        );
        Ok(())
    }
    .await;
    harness.finish(result).await
}
