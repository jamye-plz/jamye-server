use std::{
    io,
    sync::{Arc, Mutex},
};

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header::AUTHORIZATION},
    response::Response,
};
use jamye_server::{
    adapters::oauth::OsCredentialSource,
    application::auth::ExchangeInput,
    platform::logging::build_json_subscriber,
    ports::{
        apple_identity_provider::{
            AppleIdentity, AppleIdentityProviderError, AppleRevocationProviderError,
        },
        auth::CredentialSource,
    },
    transport::http::auth::{AuthHttpState, router as auth_router},
};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use tracing_subscriber::fmt::MakeWriter;
use uuid::Uuid;

use crate::{
    TestResult,
    auth_helpers::{
        KAKAO_REDIRECT, TEST_VERIFIER, authorize, harness, harness_with_apple_error,
        harness_with_apple_identity,
    },
    postgres_support::TestDatabase,
};

#[allow(
    dead_code,
    reason = "auth logging tests reuse account deletion HTTP support for Apple deletion flows"
)]
#[path = "../account_deletion/support.rs"]
mod account_deletion_support;

#[tokio::test(flavor = "current_thread")]
async fn structured_auth_logs_exclude_every_credential_class() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let fixture = harness(pool.clone(), None)?;
    let writer = SharedWriter::default();
    let output = writer.clone();
    let subscriber = build_json_subscriber(writer, "info")?;
    let _guard = tracing::subscriber::set_default(subscriber);
    let oauth_code = "TASK5_OAUTH_CODE_SENTINEL";

    let state = authorize(&fixture.service).await?;
    let issued = fixture
        .service
        .exchange(
            "kakao",
            ExchangeInput {
                authorization_code: oauth_code.to_owned(),
                state,
                code_verifier: TEST_VERIFIER.to_owned(),
                redirect_uri: KAKAO_REDIRECT.to_owned(),
            },
            "ip:logging-fixture",
        )
        .await?
        .token_pair;
    let refresh_digest = OsCredentialSource.digest(&issued.refresh_token)?;
    let refresh_digest_hex = encode_hex(refresh_digest.as_bytes());
    assert_eq!(
        format!("{refresh_digest:?}"),
        "CredentialDigest([REDACTED])"
    );

    let rotated = fixture.service.refresh(&issued.refresh_token).await?;
    let router = auth_router(AuthHttpState::new(
        fixture.service.clone(),
        fixture.codec.clone(),
    ));
    let reused = router
        .clone()
        .oneshot(
            Request::post("/api/v1/auth/refresh")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&json!({
                    "refresh_token": issued.refresh_token.clone()
                }))?))?,
        )
        .await?;
    assert_eq!(reused.status(), StatusCode::UNAUTHORIZED);
    let reused_body: Value = serde_json::from_slice(&to_bytes(reused.into_body(), 4096).await?)?;
    assert_eq!(reused_body["error"]["code"], "refresh_token_reused");

    let logout = router
        .oneshot(
            Request::post("/api/v1/auth/logout")
                .header(AUTHORIZATION, format!("Bearer {}", rotated.access_token))
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(logout.status(), StatusCode::NO_CONTENT);

    let logs = output.snapshot()?;
    assert!(logs.contains("refresh_token_reused"));
    for forbidden in [
        oauth_code,
        issued.access_token.as_str(),
        issued.refresh_token.as_str(),
        rotated.access_token.as_str(),
        rotated.refresh_token.as_str(),
        refresh_digest_hex.as_str(),
        TEST_VERIFIER,
    ] {
        assert!(
            !logs.contains(forbidden),
            "logs leaked auth credential material"
        );
    }

    pool.close().await;
    database.dispose().await
}

#[tokio::test(flavor = "current_thread")]
async fn apple_auth_and_deletion_logs_exclude_transient_credentials() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let result: TestResult = async {
        let writer = SharedWriter::default();
        let output = writer.clone();
        let subscriber = build_json_subscriber(writer, "info")?;
        let _guard = tracing::subscriber::set_default(subscriber);
        // Keep a second dispatcher registered so a parallel test thread that
        // reaches the same callsite first cannot cache it as disabled.
        let _interest_sentinel =
            tracing::Dispatch::new(tracing::subscriber::NoSubscriber::default());
        let forbidden = [
            "APPLE_LOG_EXCHANGE_TOKEN_SENTINEL",
            "APPLE_LOG_EXCHANGE_RAW_NONCE_SENTINEL",
            "APPLE_LOG_INVALID_TOKEN_SENTINEL",
            "APPLE_LOG_INVALID_RAW_NONCE_SENTINEL",
            "APPLE_LOG_DELETE_IDENTITY_TOKEN_SENTINEL",
            "APPLE_LOG_DELETE_RAW_NONCE_SENTINEL",
            "APPLE_LOG_AUTHORIZATION_CODE_SENTINEL",
            "APPLE_LOG_SUBJECT_TOKEN_SENTINEL",
            "APPLE_LOG_SUBJECT_RAW_NONCE_SENTINEL",
            "APPLE_LOG_SUBJECT_CODE_SENTINEL",
            "APPLE_LOG_CODE_TOKEN_SENTINEL",
            "APPLE_LOG_CODE_RAW_NONCE_SENTINEL",
            "APPLE_LOG_INVALID_CODE_SENTINEL",
        ];

        let success_fixture = harness_with_apple_identity(
            pool.clone(),
            AppleIdentity {
                provider_id: "apple-log-success-subject".to_owned(),
                client_id: "dev.local.jamyeapp".to_owned(),
            },
        )?;
        let success_router = auth_router(AuthHttpState::new(
            success_fixture.service.clone(),
            success_fixture.codec.clone(),
        ));
        let success = success_router
            .oneshot(apple_exchange_request(
                "APPLE_LOG_EXCHANGE_TOKEN_SENTINEL",
                "APPLE_LOG_EXCHANGE_RAW_NONCE_SENTINEL",
                Some("로그 성공"),
            )?)
            .await?;
        assert_response_excludes(success, StatusCode::OK, &forbidden).await?;

        let invalid_fixture =
            harness_with_apple_error(pool.clone(), AppleIdentityProviderError::InvalidIdentity)?;
        let invalid_router = auth_router(AuthHttpState::new(
            invalid_fixture.service.clone(),
            invalid_fixture.codec.clone(),
        ));
        let invalid = invalid_router
            .oneshot(apple_exchange_request(
                "APPLE_LOG_INVALID_TOKEN_SENTINEL",
                "APPLE_LOG_INVALID_RAW_NONCE_SENTINEL",
                None,
            )?)
            .await?;
        let invalid_body =
            assert_response_excludes(invalid, StatusCode::UNPROCESSABLE_ENTITY, &forbidden).await?;
        assert_error_code(&invalid_body, "apple_identity_token_invalid")?;

        let delete_success_id = seed_apple_deletion_user(&pool, "apple-log-delete-subject").await?;
        let delete_identity =
            std::sync::Arc::new(account_deletion_support::FakeAppleIdentityProvider::new(
                AppleIdentity {
                    provider_id: "apple-log-delete-subject".to_owned(),
                    client_id: "dev.local.jamyeapp".to_owned(),
                },
                None,
            ));
        let delete_revocation = std::sync::Arc::new(
            account_deletion_support::FakeAppleRevocationProvider::new(None),
        );
        let delete_success = account_deletion_support::test_router_with_apple(
            pool.clone(),
            Some(delete_identity),
            Some(delete_revocation),
        )?
        .oneshot(apple_delete_request(
            delete_success_id,
            "APPLE_LOG_DELETE_IDENTITY_TOKEN_SENTINEL",
            "APPLE_LOG_AUTHORIZATION_CODE_SENTINEL",
            "APPLE_LOG_DELETE_RAW_NONCE_SENTINEL",
        )?)
        .await?;
        assert_response_excludes(delete_success, StatusCode::NO_CONTENT, &forbidden).await?;

        let mismatch_id = seed_apple_deletion_user(&pool, "apple-log-subject").await?;
        let mismatch_identity =
            std::sync::Arc::new(account_deletion_support::FakeAppleIdentityProvider::new(
                AppleIdentity {
                    provider_id: "other-apple-log-subject".to_owned(),
                    client_id: "dev.local.jamyeapp".to_owned(),
                },
                None,
            ));
        let mismatch_revocation = std::sync::Arc::new(
            account_deletion_support::FakeAppleRevocationProvider::new(None),
        );
        let mismatch = account_deletion_support::test_router_with_apple(
            pool.clone(),
            Some(mismatch_identity),
            Some(mismatch_revocation),
        )?
        .oneshot(apple_delete_request(
            mismatch_id,
            "APPLE_LOG_SUBJECT_TOKEN_SENTINEL",
            "APPLE_LOG_SUBJECT_CODE_SENTINEL",
            "APPLE_LOG_SUBJECT_RAW_NONCE_SENTINEL",
        )?)
        .await?;
        let mismatch_body =
            assert_response_excludes(mismatch, StatusCode::FORBIDDEN, &forbidden).await?;
        assert_error_code(&mismatch_body, "apple_subject_mismatch")?;

        let invalid_code_id = seed_apple_deletion_user(&pool, "apple-log-code-subject").await?;
        let code_identity =
            std::sync::Arc::new(account_deletion_support::FakeAppleIdentityProvider::new(
                AppleIdentity {
                    provider_id: "apple-log-code-subject".to_owned(),
                    client_id: "dev.local.jamyeapp".to_owned(),
                },
                None,
            ));
        let code_revocation =
            std::sync::Arc::new(account_deletion_support::FakeAppleRevocationProvider::new(
                Some(AppleRevocationProviderError::InvalidAuthorizationCode),
            ));
        let invalid_code = account_deletion_support::test_router_with_apple(
            pool.clone(),
            Some(code_identity),
            Some(code_revocation),
        )?
        .oneshot(apple_delete_request(
            invalid_code_id,
            "APPLE_LOG_CODE_TOKEN_SENTINEL",
            "APPLE_LOG_INVALID_CODE_SENTINEL",
            "APPLE_LOG_CODE_RAW_NONCE_SENTINEL",
        )?)
        .await?;
        let invalid_code_body =
            assert_response_excludes(invalid_code, StatusCode::UNPROCESSABLE_ENTITY, &forbidden)
                .await?;
        assert_error_code(&invalid_code_body, "apple_authorization_code_invalid")?;

        let logs = output.snapshot()?;
        assert!(logs.contains("apple_identity_token_invalid"));
        assert!(logs.contains("apple_subject_mismatch"));
        assert!(logs.contains("apple_authorization_code_invalid"));
        for forbidden in forbidden {
            assert!(
                !logs.contains(forbidden),
                "logs leaked Apple credential material"
            );
        }

        Ok(())
    }
    .await;
    pool.close().await;
    match (result, database.dispose().await) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) => Err(error),
        (Ok(()), Err(error)) => Err(error),
        (Err(test_failure), Err(cleanup_failure)) => Err(io::Error::other(format!(
            "test failed: {test_failure}; disposable database cleanup also failed: {cleanup_failure}"
        ))
        .into()),
    }
}

fn apple_exchange_request(
    identity_token: &str,
    raw_nonce: &str,
    full_name: Option<&str>,
) -> TestResult<Request<Body>> {
    let mut body = json!({
        "identity_token": identity_token,
        "raw_nonce": raw_nonce
    });
    if let Some(full_name) = full_name {
        body["full_name"] = json!(full_name);
    }
    Ok(Request::post("/api/v1/auth/apple/exchange")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body)?))?)
}

fn apple_delete_request(
    user_id: Uuid,
    identity_token: &str,
    authorization_code: &str,
    raw_nonce: &str,
) -> TestResult<Request<Body>> {
    account_deletion_support::authenticated_request(
        "DELETE",
        "/api/v1/me",
        user_id,
        Body::from(serde_json::to_vec(&json!({
            "identity_token": identity_token,
            "authorization_code": authorization_code,
            "raw_nonce": raw_nonce
        }))?),
    )
}

async fn seed_apple_deletion_user(pool: &PgPool, provider_id: &str) -> TestResult<Uuid> {
    let user_id = Uuid::new_v4();
    sqlx::query("INSERT INTO users (id, nickname, avatar_url) VALUES ($1, $2, $3)")
        .bind(user_id)
        .bind(format!("Apple log {provider_id}"))
        .bind(format!("https://private.invalid/{user_id}"))
        .execute(pool)
        .await?;
    sqlx::query(
        "INSERT INTO auth_identities (id, user_id, provider, provider_id) \
         VALUES ($1, $2, 'apple', $3)",
    )
    .bind(Uuid::new_v4())
    .bind(user_id)
    .bind(provider_id)
    .execute(pool)
    .await?;
    Ok(user_id)
}

async fn assert_response_excludes(
    response: Response,
    expected_status: StatusCode,
    forbidden: &[&str],
) -> TestResult<String> {
    assert_eq!(response.status(), expected_status);
    let body = String::from_utf8(to_bytes(response.into_body(), 16 * 1024).await?.to_vec())?;
    for forbidden in forbidden {
        assert!(
            !body.contains(forbidden),
            "response body leaked Apple credential material"
        );
    }
    Ok(body)
}

fn assert_error_code(body: &str, code: &str) -> TestResult {
    let body: Value = serde_json::from_str(body)?;
    assert_eq!(body["error"]["code"], code);
    Ok(())
}

#[derive(Clone, Default)]
struct SharedWriter(Arc<Mutex<Vec<u8>>>);

impl SharedWriter {
    fn snapshot(&self) -> io::Result<String> {
        let bytes = self
            .0
            .lock()
            .map_err(|_| io::Error::other("auth log writer lock poisoned"))?
            .clone();
        String::from_utf8(bytes).map_err(io::Error::other)
    }
}

impl<'writer> MakeWriter<'writer> for SharedWriter {
    type Writer = SharedWriterGuard;

    fn make_writer(&'writer self) -> Self::Writer {
        SharedWriterGuard(self.0.clone())
    }
}

struct SharedWriterGuard(Arc<Mutex<Vec<u8>>>);

impl io::Write for SharedWriterGuard {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_| io::Error::other("auth log writer lock poisoned"))?
            .extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}
