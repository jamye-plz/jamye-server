use std::{
    io,
    sync::{Arc, Mutex},
};

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header::AUTHORIZATION},
};
use jamye_server::{
    adapters::oauth::OsCredentialSource,
    application::auth::ExchangeInput,
    platform::logging::build_json_subscriber,
    ports::auth::CredentialSource,
    transport::http::auth::{AuthHttpState, router as auth_router},
};
use serde_json::{Value, json};
use tower::ServiceExt;
use tracing_subscriber::fmt::MakeWriter;

use crate::{
    TestResult,
    auth_helpers::{KAKAO_REDIRECT, TEST_VERIFIER, authorize, harness},
    postgres_support::TestDatabase,
};

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
        .await?;
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
