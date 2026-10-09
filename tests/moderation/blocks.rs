//! B1 `PUT`, B2 `DELETE` and B3 `GET /api/v1/me/blocks[/{user_id}]`.

use axum::http::StatusCode;
use uuid::Uuid;

use crate::{
    TestResult,
    support::{Harness, assert_error, count, empty_body, insert_group, insert_user, json_body},
};

#[tokio::test]
async fn b1_blocks_idempotently_and_keeps_the_original_blocked_at() -> TestResult {
    let harness = Harness::new().await?;
    let result: TestResult = async {
        let blocker = insert_user(&harness.pool, "blocker").await?;
        let target = insert_user(&harness.pool, "target").await?;
        let first = harness.put_block(blocker, &target.to_string()).await?;
        assert_eq!(first.status(), StatusCode::OK);
        let first = json_body(first).await?;
        assert_eq!(first["user_id"], target.to_string());
        assert_eq!(first.as_object().map(|object| object.len()), Some(2));

        let second = harness.put_block(blocker, &target.to_string()).await?;
        assert_eq!(second.status(), StatusCode::OK);
        let second = json_body(second).await?;
        assert_eq!(first["blocked_at"], second["blocked_at"]);
        assert_eq!(
            count(
                &harness.pool,
                "SELECT count(*) FROM user_blocks WHERE blocker_id = $1",
                blocker
            )
            .await?,
            1
        );
        // One-directional: the blocked user has no block row of their own.
        assert_eq!(
            count(
                &harness.pool,
                "SELECT count(*) FROM user_blocks WHERE blocker_id = $1",
                target
            )
            .await?,
            0
        );
        Ok(())
    }
    .await;
    harness.finish(result).await
}

#[tokio::test]
async fn b1_rejects_self_blocks_and_unknown_users() -> TestResult {
    let harness = Harness::new().await?;
    let result: TestResult = async {
        let blocker = insert_user(&harness.pool, "blocker").await?;
        let response = harness.put_block(blocker, &blocker.to_string()).await?;
        assert_error(
            response,
            StatusCode::UNPROCESSABLE_ENTITY,
            "block_self_not_allowed",
        )
        .await?;
        let response = harness
            .put_block(blocker, &Uuid::new_v4().to_string())
            .await?;
        assert_error(response, StatusCode::NOT_FOUND, "user_not_found").await?;
        let deleted = insert_user(&harness.pool, "deleted").await?;
        sqlx::query("UPDATE users SET deleted_at = clock_timestamp() WHERE id = $1")
            .bind(deleted)
            .execute(&harness.pool)
            .await?;
        let response = harness.put_block(blocker, &deleted.to_string()).await?;
        assert_error(response, StatusCode::NOT_FOUND, "user_not_found").await?;
        let response = harness.put_block(blocker, "not-a-uuid").await?;
        assert_error(
            response,
            StatusCode::UNPROCESSABLE_ENTITY,
            "request_validation_failed",
        )
        .await?;
        assert_eq!(
            count(
                &harness.pool,
                "SELECT count(*) FROM user_blocks WHERE blocker_id = $1",
                blocker
            )
            .await?,
            0
        );
        Ok(())
    }
    .await;
    harness.finish(result).await
}

#[tokio::test]
async fn b2_unblocks_idempotently_with_an_empty_204() -> TestResult {
    let harness = Harness::new().await?;
    let result: TestResult = async {
        let blocker = insert_user(&harness.pool, "blocker").await?;
        let target = insert_user(&harness.pool, "target").await?;
        harness.put_block(blocker, &target.to_string()).await?;
        for _ in 0..2 {
            let response = harness.delete_block(blocker, &target.to_string()).await?;
            assert_eq!(response.status(), StatusCode::NO_CONTENT);
            assert!(empty_body(response).await?);
        }
        // No block ever existed and the user does not exist: still 204.
        let response = harness
            .delete_block(blocker, &Uuid::new_v4().to_string())
            .await?;
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        assert_eq!(
            count(
                &harness.pool,
                "SELECT count(*) FROM user_blocks WHERE blocker_id = $1",
                blocker
            )
            .await?,
            0
        );
        Ok(())
    }
    .await;
    harness.finish(result).await
}

#[tokio::test]
async fn b3_lists_every_blocked_user_in_a_stable_order_without_pagination() -> TestResult {
    let harness = Harness::new().await?;
    let result: TestResult = async {
        let blocker = insert_user(&harness.pool, "blocker").await?;
        let mut targets = Vec::new();
        for index in 0..25 {
            let target = insert_user(&harness.pool, &format!("target-{index}")).await?;
            sqlx::query("UPDATE users SET avatar_url = $2 WHERE id = $1")
                .bind(target)
                .bind(format!("https://images.example/{index}.png"))
                .execute(&harness.pool)
                .await?;
            targets.push(target);
        }
        // Distinct timestamps for the first 24, a shared timestamp for the last two.
        for (index, target) in targets.iter().enumerate() {
            let offset = i32::try_from(index.min(23))?;
            sqlx::query(
                "INSERT INTO user_blocks (blocker_id, blocked_id, created_at) \
                 VALUES ($1, $2, TIMESTAMPTZ '2026-01-01 00:00:00+00' + make_interval(secs => $3))",
            )
            .bind(blocker)
            .bind(target)
            .bind(f64::from(offset))
            .execute(&harness.pool)
            .await?;
        }
        let response = harness.list_blocks(blocker).await?;
        assert_eq!(response.status(), StatusCode::OK);
        let body = json_body(response).await?;
        assert_eq!(body.as_object().map(|object| object.len()), Some(1));
        let items = body["items"].as_array().ok_or("items missing")?;
        assert_eq!(items.len(), 25, "all blocked users, no pagination");

        let mut expected = targets.clone();
        // blocked_at descending, then user_id ascending for the shared timestamp.
        let shared = {
            let mut pair = vec![targets[23], targets[24]];
            pair.sort();
            pair
        };
        expected.truncate(23);
        expected.reverse();
        let mut ordered = shared;
        ordered.extend(expected);
        let actual = items
            .iter()
            .map(|item| item["user_id"].as_str().map(str::to_owned))
            .collect::<Vec<_>>();
        let wanted = ordered
            .iter()
            .map(|id| Some(id.to_string()))
            .collect::<Vec<_>>();
        assert_eq!(actual, wanted);
        for item in items {
            assert!(item["nickname"].as_str().is_some());
            assert!(item["avatar_url"].as_str().is_some());
            assert!(item["blocked_at"].as_str().is_some());
            assert_eq!(item.as_object().map(|object| object.len()), Some(4));
        }

        // Only the caller's own blocks are visible.
        let other = insert_user(&harness.pool, "other").await?;
        let response = harness.list_blocks(other).await?;
        let body = json_body(response).await?;
        assert_eq!(body["items"].as_array().map(Vec::len), Some(0));
        Ok(())
    }
    .await;
    harness.finish(result).await
}

#[tokio::test]
async fn blocks_do_not_change_membership_or_message_delivery() -> TestResult {
    let harness = Harness::new().await?;
    let result: TestResult = async {
        let blocker = insert_user(&harness.pool, "blocker").await?;
        let blocked = insert_user(&harness.pool, "blocked").await?;
        let group = insert_group(&harness.pool, &[blocker, blocked]).await?;
        harness.put_block(blocker, &blocked.to_string()).await?;
        let memberships = count(
            &harness.pool,
            "SELECT count(*) FROM memberships WHERE group_id = $1 AND deleted_at IS NULL",
            group.group_id,
        )
        .await?;
        assert_eq!(memberships, 2);
        // The blocked user's messages are still stored and delivered by the server.
        crate::support::insert_message(&harness.pool, group.chatroom_id, blocked, "still here")
            .await?;
        assert_eq!(
            count(
                &harness.pool,
                "SELECT count(*) FROM messages WHERE chatroom_id = $1",
                group.chatroom_id
            )
            .await?,
            1
        );
        Ok(())
    }
    .await;
    harness.finish(result).await
}

#[tokio::test]
async fn block_routes_require_a_bearer_token() -> TestResult {
    let harness = Harness::new().await?;
    let result: TestResult = async {
        for (method, uri) in [
            ("GET", "/api/v1/me/blocks".to_owned()),
            ("PUT", format!("/api/v1/me/blocks/{}", Uuid::new_v4())),
            ("DELETE", format!("/api/v1/me/blocks/{}", Uuid::new_v4())),
        ] {
            let response = harness
                .send(crate::support::json_request(method, &uri, None, None)?)
                .await?;
            assert_error(
                response,
                StatusCode::UNAUTHORIZED,
                "authentication_required",
            )
            .await?;
        }
        Ok(())
    }
    .await;
    harness.finish(result).await
}
