use std::collections::BTreeSet;

use axum::{
    body::{Body, to_bytes},
    http::{HeaderValue, Request, Response, StatusCode, header::AUTHORIZATION},
};
use jamye_server::{
    adapters::{
        oauth::OsCredentialSource,
        postgres::{auth::PostgresAuthRepository, transactions::SqlxTransactionManager},
    },
    application::account_deletion::ANONYMOUS_AUTHOR_NICKNAME,
    ports::{
        auth::{AuthRepository, CredentialSource, NewRotatedSession, RotationOutcome},
        transactions::TransactionManager,
    },
};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use crate::{
    TestResult,
    postgres_support::TestDatabase,
    support::{
        authenticated_request, bearer, delete_request, finish_database_test, require, require_eq,
        test_router,
    },
};

#[tokio::test]
async fn delete_me_requires_one_bearer_rejects_input_and_preserves_the_error_envelope() -> TestResult
{
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let result: TestResult = async {
        let router = test_router(pool.clone())?;
        let user_id = Uuid::new_v4();

        let missing = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/api/v1/me")
                    .body(Body::empty())?,
            )
            .await?;
        assert_error(missing, StatusCode::UNAUTHORIZED, "authentication_required").await?;

        let malformed = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/api/v1/me")
                    .header(AUTHORIZATION, "Bearer malformed token")
                    .body(Body::empty())?,
            )
            .await?;
        assert_error(
            malformed,
            StatusCode::UNAUTHORIZED,
            "authentication_required",
        )
        .await?;

        let mut duplicate_request = Request::builder()
            .method("DELETE")
            .uri("/api/v1/me")
            .header(AUTHORIZATION, bearer(user_id))
            .body(Body::empty())?;
        duplicate_request
            .headers_mut()
            .append(AUTHORIZATION, HeaderValue::from_str(&bearer(user_id))?);
        let duplicate = router.clone().oneshot(duplicate_request).await?;
        assert_error(
            duplicate,
            StatusCode::UNAUTHORIZED,
            "authentication_required",
        )
        .await?;

        for request in [
            authenticated_request("DELETE", "/api/v1/me?confirm=true", user_id, Body::empty())?,
            authenticated_request("DELETE", "/api/v1/me", user_id, Body::from("{}"))?,
        ] {
            let response = router.clone().oneshot(request).await?;
            assert_error(
                response,
                StatusCode::UNPROCESSABLE_ENTITY,
                "request_validation_failed",
            )
            .await?;
        }

        pool.close().await;
        let placeholder = router
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/api/v1/me")
                    .header(AUTHORIZATION, bearer(user_id))
                    .header("content-type", "application/json")
                    .body(Body::empty())?,
            )
            .await?;
        assert_error(
            placeholder,
            StatusCode::SERVICE_UNAVAILABLE,
            "database_unavailable",
        )
        .await?;

        Ok(())
    }
    .await;
    finish_database_test(database, pool, result).await
}

#[tokio::test]
async fn delete_me_commits_one_empty_204_then_revokes_account_access_and_anonymizes_retained_content()
-> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let result: TestResult = async {
        let fixture = seed_deletable_account(&pool).await?;
        let router = test_router(pool.clone())?;

        let deleted = router
            .clone()
            .oneshot(delete_request(fixture.target_id)?)
            .await?;
        assert_empty(deleted, StatusCode::NO_CONTENT).await?;

        let repeated_delete = router
            .clone()
            .oneshot(delete_request(fixture.target_id)?)
            .await?;
        assert_error(
            repeated_delete,
            StatusCode::UNAUTHORIZED,
            "authentication_required",
        )
        .await?;

        let get_me = router
            .oneshot(authenticated_request(
                "GET",
                "/api/v1/me",
                fixture.target_id,
                Body::empty(),
            )?)
            .await?;
        assert_error(get_me, StatusCode::UNAUTHORIZED, "authentication_required").await?;

        let credential_source = OsCredentialSource;
        let parent_digest = credential_source.digest(&fixture.refresh_token)?;
        let child = credential_source.generate()?;
        let transactions = SqlxTransactionManager::new(pool.clone());
        let repository = PostgresAuthRepository::new(pool.clone());
        let mut transaction = transactions.begin().await?;
        let rotation = repository
            .rotate_session(
                transaction.as_mut(),
                &parent_digest,
                &NewRotatedSession {
                    id: Uuid::new_v4(),
                    token_hash: child.digest,
                    expires_at: time::OffsetDateTime::now_utc() + time::Duration::hours(1),
                },
                time::OffsetDateTime::now_utc(),
            )
            .await?;
        transactions.rollback(transaction).await?;
        require_eq(
            rotation,
            RotationOutcome::Invalid,
            "the deleted refresh credential remained reusable",
        )?;

        let private_references = sqlx::query_scalar::<_, i64>(
            "SELECT \
            (SELECT count(*) FROM users WHERE id = $1) + \
            (SELECT count(*) FROM auth_identities WHERE user_id = $1) + \
            (SELECT count(*) FROM refresh_sessions WHERE user_id = $1) + \
            (SELECT count(*) FROM memberships WHERE user_id = $1) + \
            (SELECT count(*) FROM invites WHERE created_by = $1) + \
            (SELECT count(*) FROM chatroom_reads WHERE user_id = $1) + \
            (SELECT count(*) FROM notifications WHERE user_id = $1) + \
            (SELECT count(*) FROM push_installations WHERE user_id = $1)",
        )
        .bind(fixture.target_id)
        .fetch_one(&pool)
        .await?;
        require_eq(
            private_references,
            0,
            "account deletion left private or live-access references",
        )?;

        let reclaimable_push = sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM push_delivery_intents \
         WHERE recipient_user_id = $1 AND status IN ('pending', 'claimed', 'retryable')",
        )
        .bind(fixture.target_id)
        .fetch_one(&pool)
        .await?;
        require_eq(
            reclaimable_push,
            0,
            "account deletion left reclaimable recipient push",
        )?;

        let retained = sqlx::query_as::<_, (Uuid, Uuid, String, Option<String>)>(
            "SELECT messages.sender_id, topics.author_id, users.nickname, users.avatar_url \
         FROM messages \
         INNER JOIN topics ON topics.id = $2 \
         INNER JOIN users ON users.id = messages.sender_id \
         WHERE messages.id = $1",
        )
        .bind(fixture.message_id)
        .bind(fixture.topic_id)
        .fetch_one(&pool)
        .await?;
        require_eq(
            retained.0,
            retained.1,
            "retained message and topic did not converge on one tombstone",
        )?;
        require(
            retained.0 != fixture.target_id,
            "retained content still references the authenticating account",
        )?;
        require_eq(
            retained.2,
            ANONYMOUS_AUTHOR_NICKNAME.to_owned(),
            "retained content did not use the anonymous projection",
        )?;
        require_eq(
            retained.3,
            None,
            "anonymous retained content unexpectedly exposed an avatar",
        )?;
        let retained_payload = sqlx::query_scalar::<_, String>(
            "SELECT payload::TEXT FROM conversation_events WHERE id = $1",
        )
        .bind(fixture.event_id)
        .fetch_one(&pool)
        .await?;
        require(
            !retained_payload.contains(&fixture.target_id.to_string()),
            "retained event payload contains the deleted account id",
        )?;
        require(
            !retained_payload.contains("private delete target"),
            "retained event payload contains the deleted nickname",
        )?;
        require(
            !retained_payload.contains("private.invalid"),
            "retained event payload contains the deleted avatar material",
        )?;

        Ok(())
    }
    .await;
    finish_database_test(database, pool, result).await
}

#[tokio::test]
async fn live_owned_group_returns_409_without_mutating_the_account_graph() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let result: TestResult = async {
        let target_id = Uuid::new_v4();
        insert_user(&pool, target_id, "D5 owner").await?;
        let group_id = Uuid::new_v4();
        sqlx::query("INSERT INTO groups (id, name, owner_id) VALUES ($1, 'D5 live', $2)")
            .bind(group_id)
            .bind(target_id)
            .execute(&pool)
            .await?;
        sqlx::query(
            "INSERT INTO memberships (id, group_id, user_id, role) VALUES ($1, $2, $3, 'owner')",
        )
        .bind(Uuid::new_v4())
        .bind(group_id)
        .bind(target_id)
        .execute(&pool)
        .await?;
        let before = account_graph(&pool, target_id).await?;

        let response = test_router(pool.clone())?
            .oneshot(delete_request(target_id)?)
            .await?;
        assert_error(
            response,
            StatusCode::CONFLICT,
            "group_ownership_transfer_required",
        )
        .await?;
        require_eq(
            account_graph(&pool, target_id).await?,
            before,
            "D5 conflict mutated the account graph",
        )?;

        Ok(())
    }
    .await;
    finish_database_test(database, pool, result).await
}

async fn seed_deletable_account(pool: &PgPool) -> TestResult<DeletionFixture> {
    let target_id = Uuid::new_v4();
    let owner_id = Uuid::new_v4();
    insert_user(pool, target_id, "private delete target").await?;
    insert_user(pool, owner_id, "retained group owner").await?;

    for (user_id, provider_id) in [(target_id, "delete-target"), (owner_id, "group-owner")] {
        sqlx::query(
            "INSERT INTO auth_identities (id, user_id, provider, provider_id) \
             VALUES ($1, $2, 'google', $3)",
        )
        .bind(Uuid::new_v4())
        .bind(user_id)
        .bind(provider_id)
        .execute(pool)
        .await?;
    }
    let refresh_token = "r".repeat(43);
    let refresh_digest = OsCredentialSource.digest(&refresh_token)?;
    sqlx::query(
        "INSERT INTO refresh_sessions (id, user_id, family_id, token_hash, expires_at) \
         VALUES ($1, $2, $3, $4, clock_timestamp() + INTERVAL '1 day')",
    )
    .bind(Uuid::new_v4())
    .bind(target_id)
    .bind(Uuid::new_v4())
    .bind(refresh_digest.as_bytes().as_slice())
    .execute(pool)
    .await?;

    let group_id = Uuid::new_v4();
    sqlx::query("INSERT INTO groups (id, name, owner_id) VALUES ($1, 'retained', $2)")
        .bind(group_id)
        .bind(owner_id)
        .execute(pool)
        .await?;
    for (user_id, role) in [(owner_id, "owner"), (target_id, "member")] {
        sqlx::query(
            "INSERT INTO memberships (id, group_id, user_id, role) VALUES ($1, $2, $3, $4)",
        )
        .bind(Uuid::new_v4())
        .bind(group_id)
        .bind(user_id)
        .bind(role)
        .execute(pool)
        .await?;
    }
    let chatroom_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO chatrooms (id, group_id, type, topic_id) VALUES ($1, $2, 'main', NULL)",
    )
    .bind(chatroom_id)
    .bind(group_id)
    .execute(pool)
    .await?;

    let topic_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO topics \
             (id, group_id, author_id, idempotency_key, request_fingerprint, title) \
         VALUES ($1, $2, $3, $4, $5, 'retained topic')",
    )
    .bind(topic_id)
    .bind(group_id)
    .bind(target_id)
    .bind(Uuid::new_v4())
    .bind("d".repeat(64))
    .execute(pool)
    .await?;
    let message_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO messages (id, chatroom_id, sender_id, client_msg_id, body, type) \
         VALUES ($1, $2, $3, $4, 'retained body', 'user')",
    )
    .bind(message_id)
    .bind(chatroom_id)
    .bind(target_id)
    .bind(Uuid::new_v4())
    .execute(pool)
    .await?;
    let event_id = Uuid::new_v4();
    let cursor = sqlx::query_scalar::<_, i64>(
        "INSERT INTO conversation_events \
             (id, conversation_id, event_type, event_version, payload) \
         VALUES ($1, $2, 'message.created', 1, $3) RETURNING cursor",
    )
    .bind(event_id)
    .bind(chatroom_id)
    .bind(json!({
        "message_id": message_id,
        "sender_id": target_id,
        "sender_display_name": "private delete target",
        "avatar_url": format!("https://private.invalid/{target_id}"),
    }))
    .fetch_one(pool)
    .await?;
    sqlx::query("INSERT INTO invites (id, group_id, code, created_by) VALUES ($1, $2, $3, $4)")
        .bind(Uuid::new_v4())
        .bind(group_id)
        .bind(format!("delete-{target_id}"))
        .bind(target_id)
        .execute(pool)
        .await?;
    sqlx::query(
        "INSERT INTO chatroom_reads (id, user_id, chatroom_id, last_read_cursor) \
         VALUES ($1, $2, $3, $4)",
    )
    .bind(Uuid::new_v4())
    .bind(target_id)
    .bind(chatroom_id)
    .bind(cursor)
    .execute(pool)
    .await?;
    let notification_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO notifications \
             (id, user_id, topic_id, conversation_id, source_cursor, type, payload, dedup_key) \
         VALUES ($1, $2, $3, $4, $5, 'chat_unread', $6, $7)",
    )
    .bind(notification_id)
    .bind(target_id)
    .bind(topic_id)
    .bind(chatroom_id)
    .bind(cursor)
    .bind(json!({"sender_id": target_id}))
    .bind(format!("delete:{notification_id}"))
    .execute(pool)
    .await?;
    let installation_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO push_installations \
             (id, user_id, installation_id, platform, provider, token, environment) \
         VALUES ($1, $2, $3, 'ios', 'expo', $4, 'development')",
    )
    .bind(installation_id)
    .bind(target_id)
    .bind(format!("delete-{target_id}"))
    .bind(format!("ExponentPushToken[delete-{target_id}]"))
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO push_delivery_intents \
             (id, notification_id, source_event_id, source_message_id, recipient_user_id, \
              push_installation_id, installation_owner_epoch, \
              message_preview_enabled_snapshot, payload) \
         VALUES ($1, $2, $3, $4, $5, $6, 1, false, $7)",
    )
    .bind(Uuid::new_v4())
    .bind(notification_id)
    .bind(event_id)
    .bind(message_id)
    .bind(target_id)
    .bind(installation_id)
    .bind(json!({"notification_id": notification_id, "sender_id": target_id}))
    .execute(pool)
    .await?;

    Ok(DeletionFixture {
        target_id,
        message_id,
        topic_id,
        event_id,
        refresh_token,
    })
}

async fn insert_user(pool: &PgPool, user_id: Uuid, nickname: &str) -> TestResult {
    sqlx::query("INSERT INTO users (id, nickname, avatar_url) VALUES ($1, $2, $3)")
        .bind(user_id)
        .bind(nickname)
        .bind(format!("https://private.invalid/{user_id}"))
        .execute(pool)
        .await?;
    Ok(())
}

async fn account_graph(pool: &PgPool, user_id: Uuid) -> TestResult<String> {
    Ok(sqlx::query_scalar(
        "SELECT jsonb_build_object( \
            'users', (SELECT jsonb_agg(to_jsonb(r) ORDER BY r.id) FROM users r), \
            'groups', (SELECT jsonb_agg(to_jsonb(r) ORDER BY r.id) FROM groups r), \
            'memberships', (SELECT jsonb_agg(to_jsonb(r) ORDER BY r.id) FROM memberships r), \
            'target', $1 \
         )::TEXT",
    )
    .bind(user_id)
    .fetch_one(pool)
    .await?)
}

async fn assert_error(response: Response<Body>, status: StatusCode, code: &str) -> TestResult {
    require_eq(
        response.status(),
        status,
        "account-deletion response status differed",
    )?;
    let body = response_json(response).await?;
    assert_exact_keys(&body, &["error"])?;
    assert_exact_keys(
        &body["error"],
        &["code", "details", "message", "request_id"],
    )?;
    require_eq(
        body["error"]["code"].clone(),
        Value::String(code.to_owned()),
        "account-deletion error code differed",
    )?;
    require_eq(
        body["error"]["details"].clone(),
        Value::Null,
        "account-deletion error details were not null",
    )?;
    require_eq(
        body["error"]["message"].clone(),
        Value::String(expected_error_message(code)?.to_owned()),
        "account-deletion error message differed",
    )?;
    require(
        body["error"]["request_id"].as_str().is_some(),
        "account-deletion error omitted request_id",
    )?;
    Ok(())
}

fn expected_error_message(code: &str) -> TestResult<&'static str> {
    match code {
        "authentication_required" => Ok("인증이 필요합니다."),
        "group_ownership_transfer_required" => {
            Ok("소유권 이양이 필요한 그룹이 있어 계정을 삭제할 수 없습니다.")
        }
        "request_validation_failed" => Ok("요청 형식이 올바르지 않습니다."),
        "database_unavailable" => Ok("데이터베이스를 사용할 수 없습니다."),
        _ => Err(std::io::Error::other(format!(
            "test requested an unknown account-deletion error code: {code}"
        ))
        .into()),
    }
}

async fn assert_empty(response: Response<Body>, status: StatusCode) -> TestResult {
    require_eq(
        response.status(),
        status,
        "account-deletion success status differed",
    )?;
    require(
        to_bytes(response.into_body(), 1024).await?.is_empty(),
        "account-deletion 204 contained a response body",
    )?;
    Ok(())
}

async fn response_json(response: Response<Body>) -> TestResult<Value> {
    Ok(serde_json::from_slice(
        &to_bytes(response.into_body(), 32 * 1024).await?,
    )?)
}

fn assert_exact_keys(value: &Value, expected: &[&str]) -> TestResult {
    let Some(object) = value.as_object() else {
        return Err(std::io::Error::other("expected a JSON object").into());
    };
    let actual = object.keys().map(String::as_str).collect::<BTreeSet<_>>();
    let expected = expected.iter().copied().collect::<BTreeSet<_>>();
    require_eq(actual, expected, "JSON object keys differed")
}

struct DeletionFixture {
    target_id: Uuid,
    message_id: Uuid,
    topic_id: Uuid,
    event_id: Uuid,
    refresh_token: String,
}
