use axum::{
    body::{Body, to_bytes},
    http::{
        HeaderMap, Request, StatusCode,
        header::{CACHE_CONTROL, LOCATION, REFERRER_POLICY, X_CONTENT_TYPE_OPTIONS},
    },
};
use jamye_server::{
    application::auth::ExchangeInput,
    transport::http::auth::{AuthHttpState, router as auth_router},
};
use serde_json::Value;
use tokio::{net::TcpListener, sync::oneshot};
use tower::ServiceExt;
use url::Url;

use crate::{
    TestResult,
    auth_helpers::{KAKAO_REDIRECT, TEST_VERIFIER, authorize, harness},
    postgres_support::TestDatabase,
};

#[tokio::test]
async fn callback_bridge_redirects_only_code_and_state_to_the_fixed_provider_app_uri() -> TestResult
{
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let fixture = harness(pool.clone(), None)?;
    let router = auth_router(AuthHttpState::new(
        fixture.service.clone(),
        fixture.codec.clone(),
    ));
    let state = "s".repeat(43);

    let response = router
        .clone()
        .oneshot(Request::get(format!(
            "/api/v1/auth/oauth/google/callback?code=provider%2Bcode%2Fvalue&state={state}&scope=openid+email&authuser=0&prompt=consent&iss=https%3A%2F%2Faccounts.google.com"
        )).body(Body::empty())?)
        .await?;

    assert_eq!(response.status(), StatusCode::FOUND);
    assert_callback_headers(response.headers())?;
    let location = response
        .headers()
        .get(LOCATION)
        .ok_or("missing callback location")?
        .to_str()?;
    let location = Url::parse(location)?;
    assert_eq!(
        location.as_str().split('?').next(),
        Some("jamye://oauth/google")
    );
    assert_eq!(
        location.query_pairs().collect::<Vec<_>>(),
        vec![
            ("state".into(), state.into()),
            ("code".into(), "provider+code/value".into())
        ]
    );
    assert_eq!(fixture.attempts.len()?, 0);
    assert_eq!(fixture.provider.exchange_calls(), 0);

    pool.close().await;
    database.dispose().await
}

#[tokio::test]
async fn callback_bridge_normalizes_provider_errors_and_preserves_an_unconsumed_attempt()
-> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let fixture = harness(pool.clone(), None)?;
    let state = authorize(&fixture.service).await?;
    let router = auth_router(AuthHttpState::new(
        fixture.service.clone(),
        fixture.codec.clone(),
    ));

    let response = router
        .oneshot(Request::get(format!(
            "/api/v1/auth/oauth/kakao/callback?error=provider_outage&error_description=untrusted+provider+detail&state={state}"
        )).body(Body::empty())?)
        .await?;
    assert_eq!(response.status(), StatusCode::FOUND);
    assert_callback_headers(response.headers())?;
    let location = Url::parse(
        response
            .headers()
            .get(LOCATION)
            .ok_or("missing callback location")?
            .to_str()?,
    )?;
    assert_eq!(
        location.as_str().split('?').next(),
        Some("jamye://oauth/kakao")
    );
    assert_eq!(
        location.query_pairs().collect::<Vec<_>>(),
        vec![
            ("state".into(), state.clone().into()),
            ("error".into(), "oauth_failed".into())
        ]
    );
    assert_eq!(fixture.attempts.len()?, 1);
    assert_eq!(fixture.provider.exchange_calls(), 0);

    let denied = auth_router(AuthHttpState::new(
        fixture.service.clone(),
        fixture.codec.clone(),
    ))
    .oneshot(
        Request::get(format!(
            "/api/v1/auth/oauth/kakao/callback?error=access_denied&state={state}"
        ))
        .body(Body::empty())?,
    )
    .await?;
    let denied_location = Url::parse(
        denied
            .headers()
            .get(LOCATION)
            .ok_or("missing callback location")?
            .to_str()?,
    )?;
    assert_eq!(
        denied_location.query_pairs().collect::<Vec<_>>(),
        vec![
            ("state".into(), state.clone().into()),
            ("error".into(), "access_denied".into())
        ]
    );

    fixture
        .service
        .exchange(
            "kakao",
            ExchangeInput {
                authorization_code: "provider-code".to_owned(),
                state,
                code_verifier: TEST_VERIFIER.to_owned(),
                redirect_uri: KAKAO_REDIRECT.to_owned(),
            },
            "ip:fixture",
        )
        .await?;
    assert_eq!(fixture.provider.exchange_calls(), 1);

    pool.close().await;
    database.dispose().await
}

#[tokio::test]
async fn callback_bridge_rejects_malformed_duplicate_conflicting_oversize_and_unknown_inputs()
-> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let fixture = harness(pool.clone(), None)?;
    let router = auth_router(AuthHttpState::new(
        fixture.service.clone(),
        fixture.codec.clone(),
    ));
    let state = "s".repeat(43);
    let oversized = "a".repeat(8 * 1024);
    let cases = [
        format!("/api/v1/auth/oauth/kakao/callback?code=one&state={state}&state={state}"),
        format!("/api/v1/auth/oauth/kakao/callback?code=one&error=access_denied&state={state}"),
        format!("/api/v1/auth/oauth/kakao/callback?code=%ZZ&state={state}"),
        format!(
            "/api/v1/auth/oauth/kakao/callback?code=one&state={state}&redirect_uri=https%3A%2F%2Fevil.example"
        ),
        format!("/api/v1/auth/oauth/kakao/callback?code={oversized}&state={state}"),
    ];
    for path in cases {
        let response = router
            .clone()
            .oneshot(Request::get(path).body(Body::empty())?)
            .await?;
        assert_callback_error(
            response,
            StatusCode::UNPROCESSABLE_ENTITY,
            "request_validation_failed",
        )
        .await?;
    }
    let unsupported = router
        .oneshot(
            Request::get(format!(
                "/api/v1/auth/oauth/apple/callback?code=one&state={state}"
            ))
            .body(Body::empty())?,
        )
        .await?;
    assert_callback_error(
        unsupported,
        StatusCode::NOT_FOUND,
        "oauth_provider_not_supported",
    )
    .await?;
    assert_eq!(fixture.attempts.len()?, 0);
    assert_eq!(fixture.provider.exchange_calls(), 0);

    pool.close().await;
    database.dispose().await
}

#[tokio::test]
async fn callback_bridge_smoke_uses_a_real_loopback_tcp_http_request() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let fixture = harness(pool.clone(), None)?;
    let router = auth_router(AuthHttpState::new(
        fixture.service.clone(),
        fixture.codec.clone(),
    ));
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let address = listener.local_addr()?;
    let (shutdown, shutdown_receiver) = oneshot::channel();
    let server = tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(async move {
                let _ = shutdown_receiver.await;
            })
            .await
    });
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let state = "s".repeat(43);

    let verification = async {
        let success = client
            .get(format!(
                "http://{address}/api/v1/auth/oauth/kakao/callback?code=socket%2Bcode&state={state}"
            ))
            .send()
            .await?;
        assert_eq!(success.status(), StatusCode::FOUND);
        assert_callback_headers(success.headers())?;
        let location = Url::parse(
            success
                .headers()
                .get(LOCATION)
                .ok_or("loopback callback response omitted Location")?
                .to_str()?,
        )?;
        assert_eq!(
            location.query_pairs().collect::<Vec<_>>(),
            vec![
                ("state".into(), state.clone().into()),
                ("code".into(), "socket+code".into())
            ]
        );

        let error = client
            .get(format!(
                "http://{address}/api/v1/auth/oauth/kakao/callback?code=socket-code&state={state}&state={state}"
            ))
            .send()
            .await?;
        assert_eq!(error.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_callback_headers(error.headers())?;
        assert!(error.headers().get(LOCATION).is_none());
        let error_body = error.text().await?;
        assert!(error_body.contains("request_validation_failed"));
        assert!(!error_body.contains("socket-code"));
        Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
    }
    .await;

    shutdown
        .send(())
        .map_err(|_| "loopback callback server stopped before shutdown")?;
    server.await??;
    pool.close().await;
    database.dispose().await?;
    verification?;
    assert_eq!(fixture.attempts.len()?, 0);
    assert_eq!(fixture.provider.exchange_calls(), 0);
    Ok(())
}

fn assert_callback_headers(headers: &HeaderMap) -> TestResult {
    assert_eq!(headers.get(CACHE_CONTROL), Some(&"no-store".parse()?));
    assert_eq!(headers.get(REFERRER_POLICY), Some(&"no-referrer".parse()?));
    assert_eq!(
        headers.get(X_CONTENT_TYPE_OPTIONS),
        Some(&"nosniff".parse()?)
    );
    Ok(())
}

async fn assert_callback_error(
    response: axum::response::Response,
    status: StatusCode,
    error_code: &str,
) -> TestResult {
    assert_eq!(response.status(), status);
    assert_callback_headers(response.headers())?;
    assert!(response.headers().get(LOCATION).is_none());
    let body = to_bytes(response.into_body(), 16 * 1024).await?;
    let body: Value = serde_json::from_slice(&body)?;
    assert_eq!(
        body.pointer("/error/code").and_then(Value::as_str),
        Some(error_code)
    );
    Ok(())
}
