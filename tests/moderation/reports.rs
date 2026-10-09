//! R1 `POST /api/v1/reports`.

use axum::http::StatusCode;
use jamye_server::{
    adapters::postgres::transactions::SqlxTransactionManager,
    domain::moderation::ReportReason,
    ports::{
        moderation::{
            InsertReportCommand, ModerationRepository, ModerationRepositoryError, ReportTarget,
        },
        transactions::TransactionManager,
    },
};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    TestResult,
    support::{
        Harness, MASKED_TERM, count, insert_group, insert_message, insert_user, json_body,
        json_request, report_message_body, report_user_body,
    },
};

#[tokio::test]
async fn r1_stores_a_message_report_with_a_masked_snapshot_and_returns_201() -> TestResult {
    let harness = Harness::new().await?;
    let result: TestResult = async {
        let reporter = insert_user(&harness.pool, "reporter").await?;
        let author = insert_user(&harness.pool, "author").await?;
        let group = insert_group(&harness.pool, &[reporter, author]).await?;
        let message_id = insert_message(
            &harness.pool,
            group.chatroom_id,
            author,
            &format!("hello {MASKED_TERM} and {}", MASKED_TERM.to_uppercase()),
        )
        .await?;

        let response = harness
            .report(reporter, &report_message_body(message_id, "harassment"))
            .await?;
        assert_eq!(response.status(), StatusCode::CREATED);
        let body = json_body(response).await?;
        assert_eq!(body.as_object().map(|object| object.len()), Some(3));
        assert_eq!(body["status"], "open");
        let report_id = Uuid::try_parse(body["id"].as_str().ok_or("id missing")?)?;
        assert!(body["created_at"].as_str().is_some());

        let row = sqlx::query_as::<
            _,
            (
                Uuid,
                String,
                Option<Uuid>,
                Option<Uuid>,
                Uuid,
                String,
                String,
                bool,
            ),
        >(
            "SELECT reporter_id, target_type, target_message_id, target_user_id, \
                    target_group_id, reason, status, handled_at IS NULL \
             FROM reports WHERE id = $1",
        )
        .bind(report_id)
        .fetch_one(&harness.pool)
        .await?;
        assert_eq!(row.0, reporter);
        assert_eq!(row.1, "message");
        assert_eq!(row.2, Some(message_id));
        assert_eq!(row.3, None);
        assert_eq!(row.4, group.group_id);
        assert_eq!(row.5, "harassment");
        assert_eq!(row.6, "open");
        assert!(row.7, "an open report has no handled_at");

        let snapshot =
            sqlx::query_scalar::<_, Value>("SELECT message_snapshot FROM reports WHERE id = $1")
                .bind(report_id)
                .fetch_one(&harness.pool)
                .await?;
        assert_eq!(snapshot["text"], "hello *** and ***");
        assert_eq!(snapshot["content_type"], "text");
        assert_eq!(snapshot["media"], json!([]));
        assert!(!snapshot.to_string().contains(MASKED_TERM));
        Ok(())
    }
    .await;
    harness.finish(result).await
}

#[tokio::test]
async fn r1_snapshot_text_is_bounded_and_media_references_are_kept() -> TestResult {
    let harness = Harness::new().await?;
    let result: TestResult = async {
        let reporter = insert_user(&harness.pool, "reporter").await?;
        let author = insert_user(&harness.pool, "author").await?;
        let group = insert_group(&harness.pool, &[reporter, author]).await?;
        let long = "가".repeat(4100);
        let long_message = insert_message(&harness.pool, group.chatroom_id, author, &long).await?;
        let response = harness
            .report(reporter, &report_message_body(long_message, "spam"))
            .await?;
        assert_eq!(response.status(), StatusCode::CREATED);
        let snapshot = sqlx::query_scalar::<_, Value>(
            "SELECT message_snapshot FROM reports WHERE target_message_id = $1",
        )
        .bind(long_message)
        .fetch_one(&harness.pool)
        .await?;
        assert_eq!(
            snapshot["text"].as_str().map(|text| text.chars().count()),
            Some(4000)
        );

        // A media-only message records its attachment references and no text.
        let media_message = insert_message(&harness.pool, group.chatroom_id, author, "").await?;
        sqlx::query("UPDATE messages SET body = NULL WHERE id = $1")
            .bind(media_message)
            .execute(&harness.pool)
            .await?;
        let upload_id = Uuid::new_v4();
        let object_key = format!("chat/{upload_id}");
        sqlx::query(
            "INSERT INTO media_uploads \
                 (id, user_id, object_key, scope, target_id, content_type, byte_size, status, \
                  bound_message_id, created_at, confirmed_at, consumed_at, expires_at) \
             VALUES ($1, $2, $3, 'chat', $4, 'image/jpeg', 1024, 'bound', $5, \
                     clock_timestamp() - interval '1 minute', \
                     clock_timestamp() - interval '30 seconds', \
                     clock_timestamp() - interval '20 seconds', \
                     clock_timestamp() + interval '1 hour')",
        )
        .bind(upload_id)
        .bind(author)
        .bind(&object_key)
        .bind(group.chatroom_id)
        .bind(media_message)
        .execute(&harness.pool)
        .await?;
        sqlx::query(
            "INSERT INTO message_media \
                 (id, message_id, media_upload_id, type, object_key, byte_size, position) \
             VALUES ($1, $2, $3, 'image', $4, 1024, 0)",
        )
        .bind(Uuid::new_v4())
        .bind(media_message)
        .bind(upload_id)
        .bind(&object_key)
        .execute(&harness.pool)
        .await?;
        let response = harness
            .report(reporter, &report_message_body(media_message, "violence"))
            .await?;
        assert_eq!(response.status(), StatusCode::CREATED);
        let snapshot = sqlx::query_scalar::<_, Value>(
            "SELECT message_snapshot FROM reports WHERE target_message_id = $1",
        )
        .bind(media_message)
        .fetch_one(&harness.pool)
        .await?;
        assert_eq!(snapshot["text"], Value::Null);
        assert_eq!(snapshot["content_type"], "media");
        assert_eq!(snapshot["media"][0]["upload_id"], upload_id.to_string());
        assert_eq!(snapshot["media"][0]["kind"], "image");
        assert_eq!(snapshot["media"][0]["object_key"], object_key);
        Ok(())
    }
    .await;
    harness.finish(result).await
}

#[tokio::test]
async fn r1_accepts_a_user_target_in_a_shared_group_and_allows_duplicates() -> TestResult {
    let harness = Harness::new().await?;
    let result: TestResult = async {
        let reporter = insert_user(&harness.pool, "reporter").await?;
        let target = insert_user(&harness.pool, "target").await?;
        let group = insert_group(&harness.pool, &[reporter, target]).await?;
        for reason in ["spam", "spam", "hate"] {
            let response = harness
                .report(reporter, &report_user_body(target, reason))
                .await?;
            assert_eq!(response.status(), StatusCode::CREATED);
        }
        let rows = sqlx::query_as::<_, (String, Option<Uuid>, Option<Uuid>, Uuid, bool)>(
            "SELECT target_type, target_user_id, target_message_id, target_group_id, \
                    message_snapshot IS NULL \
             FROM reports WHERE reporter_id = $1",
        )
        .bind(reporter)
        .fetch_all(&harness.pool)
        .await?;
        assert_eq!(rows.len(), 3, "duplicates are allowed and stored");
        for row in rows {
            assert_eq!(row.0, "user");
            assert_eq!(row.1, Some(target));
            assert_eq!(row.2, None);
            assert_eq!(row.3, group.group_id);
            assert!(row.4);
        }
        Ok(())
    }
    .await;
    harness.finish(result).await
}

#[tokio::test]
async fn r1_answers_the_same_404_for_missing_and_inaccessible_targets() -> TestResult {
    let harness = Harness::new().await?;
    let result: TestResult = async {
        let reporter = insert_user(&harness.pool, "reporter").await?;
        let outsider_group_member = insert_user(&harness.pool, "member").await?;
        let stranger = insert_user(&harness.pool, "stranger").await?;
        let own_group = insert_group(&harness.pool, &[reporter, outsider_group_member]).await?;
        let other_group = insert_group(&harness.pool, &[stranger]).await?;
        let foreign_message =
            insert_message(&harness.pool, other_group.chatroom_id, stranger, "hi").await?;
        let deleted_message = insert_message(
            &harness.pool,
            own_group.chatroom_id,
            outsider_group_member,
            "deleted",
        )
        .await?;
        sqlx::query("UPDATE messages SET deleted_at = clock_timestamp() WHERE id = $1")
            .bind(deleted_message)
            .execute(&harness.pool)
            .await?;
        let left_member = insert_user(&harness.pool, "left").await?;
        sqlx::query(
            "INSERT INTO memberships (id, group_id, user_id, role, deleted_at) \
             VALUES ($1, $2, $3, 'member', clock_timestamp())",
        )
        .bind(Uuid::new_v4())
        .bind(own_group.group_id)
        .bind(left_member)
        .execute(&harness.pool)
        .await?;

        let bodies = [
            report_message_body(Uuid::new_v4(), "spam"),
            report_message_body(foreign_message, "spam"),
            report_message_body(deleted_message, "spam"),
            report_user_body(Uuid::new_v4(), "spam"),
            report_user_body(stranger, "spam"),
            report_user_body(left_member, "spam"),
        ];
        let mut messages = Vec::new();
        for body in &bodies {
            let response = harness.report(reporter, body).await?;
            messages.push(
                crate::support::assert_error(
                    response,
                    StatusCode::NOT_FOUND,
                    "report_target_not_found",
                )
                .await?,
            );
        }
        assert!(
            messages.windows(2).all(|pair| pair[0] == pair[1]),
            "missing and inaccessible targets must be indistinguishable"
        );
        assert_eq!(
            count(
                &harness.pool,
                "SELECT count(*) FROM reports WHERE reporter_id = $1",
                reporter
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
async fn r1_rejects_self_reports_and_invalid_input_with_422() -> TestResult {
    let harness = Harness::new().await?;
    let result: TestResult = async {
        let reporter = insert_user(&harness.pool, "reporter").await?;
        let other = insert_user(&harness.pool, "other").await?;
        let group = insert_group(&harness.pool, &[reporter, other]).await?;
        let own_message =
            insert_message(&harness.pool, group.chatroom_id, reporter, "mine").await?;

        for body in [
            report_user_body(reporter, "spam"),
            report_message_body(own_message, "spam"),
        ] {
            let response = harness.report(reporter, &body).await?;
            crate::support::assert_error(
                response,
                StatusCode::UNPROCESSABLE_ENTITY,
                "report_self_target",
            )
            .await?;
        }

        let invalid: [Value; 9] = [
            json!({"target_type": "user", "user_id": other, "reason": "rude"}),
            json!({"target_type": "user", "user_id": other}),
            json!({"target_type": "group", "user_id": other, "reason": "spam"}),
            json!({"target_type": "message", "reason": "spam"}),
            json!({"target_type": "user", "reason": "spam"}),
            json!({"target_type": "user", "user_id": other, "message_id": own_message, "reason": "spam"}),
            json!({"target_type": "user", "user_id": "not-a-uuid", "reason": "spam"}),
            json!({"target_type": "user", "user_id": other, "reason": "spam", "detail": "free text"}),
            json!({}),
        ];
        for body in &invalid {
            let response = harness.report(reporter, body).await?;
            crate::support::assert_error(
                response,
                StatusCode::UNPROCESSABLE_ENTITY,
                "request_validation_failed",
            )
            .await?;
        }
        assert_eq!(
            count(
                &harness.pool,
                "SELECT count(*) FROM reports WHERE reporter_id = $1",
                reporter
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
async fn r1_uses_a_fixed_twenty_per_hour_per_user_limit_and_stores_nothing_when_denied()
-> TestResult {
    let harness = Harness::new().await?;
    let result: TestResult = async {
        let reporter = insert_user(&harness.pool, "reporter").await?;
        let target = insert_user(&harness.pool, "target").await?;
        insert_group(&harness.pool, &[reporter, target]).await?;

        let response = harness
            .report(reporter, &report_user_body(target, "spam"))
            .await?;
        assert_eq!(response.status(), StatusCode::CREATED);
        let requests = harness.limiter.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].endpoint, "report_create");
        assert_eq!(requests[0].subject, format!("user:{reporter}"));
        assert_eq!(requests[0].limit, 20);
        assert_eq!(requests[0].window.as_secs(), 3600);

        harness.limiter.deny_all();
        let response = harness
            .report(reporter, &report_user_body(target, "spam"))
            .await?;
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(
            response
                .headers()
                .get("retry-after")
                .and_then(|value| value.to_str().ok()),
            Some("7")
        );
        let body = json_body(response).await?;
        assert_eq!(body["error"]["code"], "rate_limit_exceeded");
        assert_eq!(
            count(
                &harness.pool,
                "SELECT count(*) FROM reports WHERE reporter_id = $1",
                reporter
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
async fn r1_requires_a_bearer_token() -> TestResult {
    let harness = Harness::new().await?;
    let result: TestResult = async {
        let response = harness
            .send(json_request(
                "POST",
                "/api/v1/reports",
                None,
                Some(&report_user_body(Uuid::new_v4(), "spam")),
            )?)
            .await?;
        crate::support::assert_error(
            response,
            StatusCode::UNAUTHORIZED,
            "authentication_required",
        )
        .await?;
        Ok(())
    }
    .await;
    harness.finish(result).await
}

#[tokio::test]
async fn a_foreign_key_violation_while_inserting_a_report_is_target_not_found() -> TestResult {
    let harness = Harness::new().await?;
    let result: TestResult = async {
        let reporter = insert_user(&harness.pool, "reporter").await?;
        let member = insert_user(&harness.pool, "member").await?;
        let group = insert_group(&harness.pool, &[reporter, member]).await?;
        let transactions = SqlxTransactionManager::new(harness.pool.clone());
        // The purge race: the access check passed, then the target row vanished before the
        // insert. A target user id with no row hits the reports foreign key.
        let mut transaction = transactions.begin().await?;
        let error = harness
            .repository
            .insert_report(
                transaction.as_mut(),
                &InsertReportCommand {
                    id: Uuid::new_v4(),
                    reporter_id: reporter,
                    target: ReportTarget::User(Uuid::new_v4()),
                    group_id: group.group_id,
                    reason: ReportReason::Spam,
                    message_snapshot: None,
                },
            )
            .await
            .err();
        transactions.rollback(transaction).await?;
        assert_eq!(
            error,
            Some(ModerationRepositoryError::TargetNotFound),
            "the purge race must surface as report_target_not_found (404), not user_not_found"
        );
        assert_eq!(
            count(
                &harness.pool,
                "SELECT count(*) FROM reports WHERE reporter_id = $1",
                reporter
            )
            .await?,
            0
        );
        Ok(())
    }
    .await;
    harness.finish(result).await
}
