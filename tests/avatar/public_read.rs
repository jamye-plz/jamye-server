//! U6 `GET /api/v1/avatars/{avatar_id}`.

use axum::http::StatusCode;
use uuid::Uuid;

use crate::{
    TestResult,
    support::{
        Harness, READ_LIMIT, assert_error, header, insert_row, insert_user, jpeg_bytes, object_key,
        public_get, raw_body, row_status,
    },
};

#[tokio::test]
async fn u6_serves_the_active_jpeg_without_authentication_and_only_the_three_headers() -> TestResult
{
    let harness = Harness::new().await?;
    let result: TestResult = async {
        let actor = insert_user(&harness.pool).await?;
        let upload_id = harness.create_intent(actor, 2048).await?;
        let bytes = jpeg_bytes(2048);
        harness
            .storage
            .put(&object_key(actor, upload_id), "image/jpeg", bytes.clone());
        let response = harness.finalize(actor, upload_id).await?;
        assert_eq!(response.status(), StatusCode::OK);

        let response = harness
            .send(public_get(&format!("/api/v1/avatars/{upload_id}"))?)
            .await?;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            header(&response, "content-type").as_deref(),
            Some("image/jpeg")
        );
        assert_eq!(
            header(&response, "cache-control").as_deref(),
            Some("public, max-age=31536000, immutable")
        );
        assert_eq!(
            header(&response, "x-content-type-options").as_deref(),
            Some("nosniff")
        );
        for absent in [
            "etag",
            "last-modified",
            "content-security-policy",
            "cross-origin-resource-policy",
            "content-disposition",
        ] {
            assert!(
                response.headers().get(absent).is_none(),
                "U6 unexpectedly set {absent}"
            );
        }
        assert_eq!(raw_body(response).await?, bytes);

        // A client conditional request is not honored: no ETag/304 handling exists.
        let conditional = harness
            .send({
                let mut request = public_get(&format!("/api/v1/avatars/{upload_id}"))?;
                request
                    .headers_mut()
                    .insert("if-none-match", "\"anything\"".parse()?);
                request
            })
            .await?;
        assert_eq!(conditional.status(), StatusCode::OK);
        Ok(())
    }
    .await;
    harness.finish(result).await
}

#[tokio::test]
async fn u6_answers_every_non_active_or_malformed_id_with_the_same_no_store_404() -> TestResult {
    let harness = Harness::new().await?;
    let result: TestResult = async {
        let actor = insert_user(&harness.pool).await?;
        let other = insert_user(&harness.pool).await?;
        let pending = Uuid::new_v4();
        insert_row(&harness.pool, actor, pending, "pending").await?;
        let released = Uuid::new_v4();
        insert_row(&harness.pool, other, released, "released").await?;
        for upload in [pending, released] {
            harness.storage.put(
                &object_key(if upload == pending { actor } else { other }, upload),
                "image/jpeg",
                jpeg_bytes(2048),
            );
        }

        for path in [
            "not-a-uuid".to_owned(),
            "%2e%2e".to_owned(),
            Uuid::new_v4().to_string(),
            pending.to_string(),
            released.to_string(),
        ] {
            let response = harness
                .send(public_get(&format!("/api/v1/avatars/{path}"))?)
                .await?;
            assert_eq!(
                header(&response, "cache-control").as_deref(),
                Some("no-store"),
                "404 for {path} is cacheable"
            );
            assert_error(response, StatusCode::NOT_FOUND, "avatar_not_found").await?;
        }
        Ok(())
    }
    .await;
    harness.finish(result).await
}

#[tokio::test]
async fn u6_applies_the_public_read_limit_keyed_by_the_peer_address() -> TestResult {
    let harness = Harness::new().await?;
    let result: TestResult = async {
        let actor = insert_user(&harness.pool).await?;
        let upload_id = harness.upload_and_finalize(actor).await?;

        let response = harness
            .send(public_get(&format!("/api/v1/avatars/{upload_id}"))?)
            .await?;
        assert_eq!(response.status(), StatusCode::OK);
        let requests = harness.requests_for("avatar_public_read");
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].limit, READ_LIMIT);
        assert_eq!(requests[0].window.as_secs(), 10);
        assert!(requests[0].subject.contains(crate::support::PEER_IP));
        assert!(
            !requests[0].subject.contains(&actor.to_string()),
            "rate-limit subject carries the user id"
        );

        // Malformed ids also count against the limit.
        harness
            .send(public_get("/api/v1/avatars/not-a-uuid")?)
            .await?;
        assert_eq!(harness.requests_for("avatar_public_read").len(), 2);

        harness.limiter.deny("avatar_public_read");
        let response = harness
            .send(public_get(&format!("/api/v1/avatars/{upload_id}"))?)
            .await?;
        assert_eq!(header(&response, "retry-after").as_deref(), Some("7"));
        assert_error(
            response,
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limit_exceeded",
        )
        .await
    }
    .await;
    harness.finish(result).await
}

#[tokio::test]
async fn u6_reports_a_storage_outage_as_a_no_store_503() -> TestResult {
    let harness = Harness::new().await?;
    let result: TestResult = async {
        let actor = insert_user(&harness.pool).await?;
        let upload_id = harness.upload_and_finalize(actor).await?;
        harness.storage.set_get_unavailable(true);
        let response = harness
            .send(public_get(&format!("/api/v1/avatars/{upload_id}"))?)
            .await?;
        assert_eq!(
            header(&response, "cache-control").as_deref(),
            Some("no-store")
        );
        assert_error(
            response,
            StatusCode::SERVICE_UNAVAILABLE,
            "object_storage_degraded",
        )
        .await
    }
    .await;
    harness.finish(result).await
}

#[tokio::test]
async fn u6_keeps_serving_during_the_account_grace_period_until_the_row_is_purged() -> TestResult {
    let harness = Harness::new().await?;
    let result: TestResult = async {
        let actor = insert_user(&harness.pool).await?;
        let upload_id = harness.upload_and_finalize(actor).await?;
        let path = format!("/api/v1/avatars/{upload_id}");

        sqlx::query("UPDATE users SET deleted_at = clock_timestamp() WHERE id = $1")
            .bind(actor)
            .execute(&harness.pool)
            .await?;
        let response = harness.send(public_get(&path)?).await?;
        assert_eq!(response.status(), StatusCode::OK);

        // Purge hard-deletes the row; the same URL then answers 404.
        sqlx::query("DELETE FROM user_avatar_uploads WHERE id = $1")
            .bind(upload_id)
            .execute(&harness.pool)
            .await?;
        let response = harness.send(public_get(&path)?).await?;
        assert_error(response, StatusCode::NOT_FOUND, "avatar_not_found").await
    }
    .await;
    harness.finish(result).await
}

#[tokio::test]
async fn u6_answers_a_no_store_404_when_the_active_object_was_overwritten_with_non_jpeg_bytes()
-> TestResult {
    let harness = Harness::new().await?;
    let result: TestResult = async {
        let actor = insert_user(&harness.pool).await?;
        let upload_id = harness.upload_and_finalize(actor).await?;
        let path = format!("/api/v1/avatars/{upload_id}");
        let response = harness.send(public_get(&path)?).await?;
        assert_eq!(response.status(), StatusCode::OK);

        // The presigned PUT stays valid after finalize, so the stored object can be replaced
        // with bytes that were never checked. U6 must not serve them under the active id.
        let mut overwritten = jpeg_bytes(2048);
        overwritten[..4].copy_from_slice(&[0x89, 0x50, 0x4E, 0x47]);
        harness
            .storage
            .put(&object_key(actor, upload_id), "image/jpeg", overwritten);

        let response = harness.send(public_get(&path)?).await?;
        assert_eq!(
            header(&response, "cache-control").as_deref(),
            Some("no-store")
        );
        assert_error(response, StatusCode::NOT_FOUND, "avatar_not_found").await?;
        // Serving is refused without changing state; the row itself is untouched.
        assert_eq!(
            row_status(&harness.pool, upload_id).await?.as_deref(),
            Some("active")
        );
        Ok(())
    }
    .await;
    harness.finish(result).await
}
