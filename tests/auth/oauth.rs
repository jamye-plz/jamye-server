use std::{collections::BTreeSet, io, sync::Arc, time::Duration};

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header::RETRY_AFTER},
};
use jamye_server::{
    adapters::oauth::{
        GOOGLE_AUTHORIZE_URL, GOOGLE_IDENTITY_URL, GOOGLE_ISSUER, GOOGLE_JWKS_URL,
        GOOGLE_TOKEN_URL, GoogleOAuthProvider, KAKAO_AUTHORIZE_URL, KAKAO_IDENTITY_URL,
        KAKAO_TOKEN_URL, KakaoOAuthProvider, OAuthClientConfig, ProductionTokenCodec,
    },
    application::auth::{AccessTokenVerifier, AuthError, AuthorizeInput, ExchangeInput},
    config::{
        auth::{AuthConfig, AuthConfigInput, OAUTH_ATTEMPT_TTL},
        rate_limit::{RateLimitConfig, RateLimitConfigInput},
    },
    ports::{
        auth::AccessTokenIssuer,
        oauth_provider::{AuthorizationRequest, OAuthProvider, OAuthProviderError},
        rate_limit::{
            RateLimitError, RateLimitFuture, RateLimitOutcome, RateLimitRequest, RateLimiter,
        },
    },
    transport::http::auth::{AuthHttpState, router as auth_router},
};
use serde_json::{Value, json};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use tower::ServiceExt;
use url::Url;
use uuid::Uuid;

use crate::{
    TestResult,
    auth_helpers::{
        GOOGLE_REDIRECT, KAKAO_REDIRECT, TEST_VERIFIER, authorize, harness,
        harness_with_provider_error, harness_with_provider_identity, harness_with_rate_limiter,
    },
    postgres_support::TestDatabase,
};

#[test]
fn concrete_provider_origins_are_fixed_to_the_selected_literal_allowlist() -> TestResult {
    assert_eq!(
        KAKAO_AUTHORIZE_URL,
        "https://kauth.kakao.com/oauth/authorize"
    );
    assert_eq!(KAKAO_TOKEN_URL, "https://kauth.kakao.com/oauth/token");
    assert_eq!(KAKAO_IDENTITY_URL, "https://kapi.kakao.com/v2/user/me");
    assert_eq!(GOOGLE_ISSUER, "https://accounts.google.com");
    assert_eq!(
        GOOGLE_AUTHORIZE_URL,
        "https://accounts.google.com/o/oauth2/v2/auth"
    );
    assert_eq!(GOOGLE_TOKEN_URL, "https://oauth2.googleapis.com/token");
    assert_eq!(
        GOOGLE_IDENTITY_URL,
        "https://openidconnect.googleapis.com/v1/userinfo"
    );
    assert_eq!(
        GOOGLE_JWKS_URL,
        "https://www.googleapis.com/oauth2/v3/certs"
    );

    let request = AuthorizationRequest {
        redirect_uri: KAKAO_REDIRECT.to_owned(),
        state: "s".repeat(43),
        code_challenge: "c".repeat(43),
        nonce: "n".repeat(43),
    };
    let kakao = KakaoOAuthProvider::new(OAuthClientConfig::new(
        "kakao-client",
        "kakao-secret",
        Duration::from_secs(2),
    )?)?;
    let kakao_url = Url::parse(&kakao.authorization_url(&request)?)?;
    assert_eq!(
        kakao_url.origin().ascii_serialization(),
        "https://kauth.kakao.com"
    );
    let kakao_query = kakao_url.query_pairs().collect::<Vec<_>>();
    assert!(kakao_query.contains(&("code_challenge_method".into(), "S256".into())));

    let google = GoogleOAuthProvider::new(OAuthClientConfig::new(
        "google-client",
        "google-secret",
        Duration::from_secs(2),
    )?)?;
    let google_url = Url::parse(&google.authorization_url(&AuthorizationRequest {
        redirect_uri: GOOGLE_REDIRECT.to_owned(),
        ..request
    })?)?;
    assert_eq!(
        google_url.origin().ascii_serialization(),
        "https://accounts.google.com"
    );
    let google_query = google_url.query_pairs().collect::<Vec<_>>();
    assert!(google_query.contains(&("scope".into(), "openid profile".into())));
    assert!(google_query.contains(&("nonce".into(), "n".repeat(43).into())));
    Ok(())
}

#[tokio::test]
async fn unsupported_and_disabled_providers_have_zero_side_effects() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let fixture = harness(pool.clone(), None)?;
    let challenge = jamye_server::ports::auth::CredentialSource::pkce_s256(
        &jamye_server::adapters::oauth::OsCredentialSource,
        TEST_VERIFIER,
    )?;
    let input = || AuthorizeInput {
        redirect_uri: KAKAO_REDIRECT.to_owned(),
        code_challenge: challenge.clone(),
        code_challenge_method: "S256".to_owned(),
    };
    assert_eq!(
        fixture
            .service
            .authorize("apple", input(), "ip:fixture")
            .await,
        Err(AuthError::OAuthProviderNotSupported)
    );
    assert_eq!(
        fixture
            .service
            .authorize(
                "google",
                AuthorizeInput {
                    redirect_uri: GOOGLE_REDIRECT.to_owned(),
                    ..input()
                },
                "ip:fixture",
            )
            .await,
        Err(AuthError::OAuthProviderNotAvailable)
    );
    assert_eq!(fixture.attempts.len()?, 0);
    assert_eq!(fixture.provider.exchange_calls(), 0);
    let rows: i64 = sqlx::query_scalar(
        "SELECT (SELECT count(*) FROM users) + \
                (SELECT count(*) FROM auth_identities) + \
                (SELECT count(*) FROM refresh_sessions)",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(rows, 0);
    pool.close().await;
    database.dispose().await
}

#[tokio::test]
async fn unavailable_shared_rate_limit_fails_before_every_auth_mutation() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let fixture = harness_with_rate_limiter(pool.clone(), None, Arc::new(UnavailableRateLimiter))?;
    let challenge = jamye_server::ports::auth::CredentialSource::pkce_s256(
        &jamye_server::adapters::oauth::OsCredentialSource,
        TEST_VERIFIER,
    )?;

    assert_eq!(
        fixture
            .service
            .authorize(
                "kakao",
                AuthorizeInput {
                    redirect_uri: KAKAO_REDIRECT.to_owned(),
                    code_challenge: challenge.clone(),
                    code_challenge_method: "S256".to_owned(),
                },
                "ip:fixture",
            )
            .await,
        Err(AuthError::RateLimitUnavailable)
    );
    let router = auth_router(AuthHttpState::new(
        fixture.service.clone(),
        fixture.codec.clone(),
    ));
    let response = router
        .oneshot(authorize_request(&challenge, "S256", KAKAO_REDIRECT)?)
        .await?;
    assert_error_code(
        response,
        StatusCode::SERVICE_UNAVAILABLE,
        "rate_limit_unavailable",
    )
    .await?;
    assert_eq!(fixture.attempts.len()?, 0);
    assert_eq!(fixture.provider.exchange_calls(), 0);
    let rows: i64 = sqlx::query_scalar(
        "SELECT (SELECT count(*) FROM users) + \
                (SELECT count(*) FROM auth_identities) + \
                (SELECT count(*) FROM refresh_sessions)",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(rows, 0);

    pool.close().await;
    database.dispose().await
}

#[tokio::test]
async fn authorize_http_separates_shape_semantics_and_stable_rate_limit() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let fixture = harness_with_rate_limiter(
        pool.clone(),
        None,
        Arc::new(DenyRateLimiter(Duration::from_secs(3))),
    )?;
    let router = auth_router(AuthHttpState::new(
        fixture.service.clone(),
        fixture.codec.clone(),
    ));
    let challenge = jamye_server::ports::auth::CredentialSource::pkce_s256(
        &jamye_server::adapters::oauth::OsCredentialSource,
        TEST_VERIFIER,
    )?;

    let malformed = authorize_request(&challenge[..42], "S256", KAKAO_REDIRECT)?;
    let malformed = router.clone().oneshot(malformed).await?;
    assert_error_code(
        malformed,
        StatusCode::UNPROCESSABLE_ENTITY,
        "request_validation_failed",
    )
    .await?;

    let wrong_method = authorize_request(&challenge, "plain", KAKAO_REDIRECT)?;
    let wrong_method = router.clone().oneshot(wrong_method).await?;
    assert_error_code(
        wrong_method,
        StatusCode::UNPROCESSABLE_ENTITY,
        "oauth_authorize_invalid",
    )
    .await?;

    let wrong_redirect = authorize_request(&challenge, "S256", "jamye://oauth/other")?;
    let wrong_redirect = router.clone().oneshot(wrong_redirect).await?;
    assert_error_code(
        wrong_redirect,
        StatusCode::UNPROCESSABLE_ENTITY,
        "oauth_authorize_invalid",
    )
    .await?;

    let limited = router
        .oneshot(authorize_request(&challenge, "S256", KAKAO_REDIRECT)?)
        .await?;
    assert_eq!(limited.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        limited
            .headers()
            .get(RETRY_AFTER)
            .and_then(|value| value.to_str().ok()),
        Some("3")
    );
    assert_error_code(
        limited,
        StatusCode::TOO_MANY_REQUESTS,
        "rate_limit_exceeded",
    )
    .await?;
    assert_eq!(fixture.attempts.len()?, 0);
    assert_eq!(fixture.provider.exchange_calls(), 0);

    pool.close().await;
    database.dispose().await
}

#[tokio::test]
async fn auth_http_happy_path_returns_exact_mobile_token_pairs_and_stores_only_digest() -> TestResult
{
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let fixture = harness(pool.clone(), None)?;
    let router = auth_router(AuthHttpState::new(
        fixture.service.clone(),
        fixture.codec.clone(),
    ));
    let challenge = jamye_server::ports::auth::CredentialSource::pkce_s256(
        &jamye_server::adapters::oauth::OsCredentialSource,
        TEST_VERIFIER,
    )?;

    let authorize_response = router
        .clone()
        .oneshot(authorize_request(&challenge, "S256", KAKAO_REDIRECT)?)
        .await?;
    assert_eq!(authorize_response.status(), StatusCode::OK);
    let authorize_body: Value =
        serde_json::from_slice(&to_bytes(authorize_response.into_body(), 4096).await?)?;
    assert_eq!(authorize_body["expires_in_seconds"], 600);
    let state = authorize_body["state"]
        .as_str()
        .ok_or_else(|| io::Error::other("authorize response omitted state"))?;
    assert_eq!(state.len(), 43);
    let authorization_url = Url::parse(
        authorize_body["authorization_url"]
            .as_str()
            .ok_or_else(|| io::Error::other("authorize response omitted provider URL"))?,
    )?;
    assert!(
        authorization_url
            .query_pairs()
            .any(|(key, value)| { key == "state" && value == state })
    );

    let exchange_response = router
        .clone()
        .oneshot(
            Request::post("/api/v1/auth/oauth/kakao/exchange")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&json!({
                    "authorization_code": "provider-code",
                    "state": state,
                    "code_verifier": TEST_VERIFIER,
                    "redirect_uri": KAKAO_REDIRECT
                }))?))?,
        )
        .await?;
    assert_eq!(exchange_response.status(), StatusCode::OK);
    let exchange_body: Value =
        serde_json::from_slice(&to_bytes(exchange_response.into_body(), 8192).await?)?;
    let first_refresh = assert_exact_token_pair(&exchange_body)?;
    let digest = jamye_server::ports::auth::CredentialSource::digest(
        &jamye_server::adapters::oauth::OsCredentialSource,
        &first_refresh,
    )?;
    let stored_hash: Vec<u8> = sqlx::query_scalar(
        "SELECT token_hash FROM refresh_sessions WHERE parent_session_id IS NULL",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(stored_hash.as_slice(), digest.as_bytes().as_slice());
    assert_ne!(stored_hash.as_slice(), first_refresh.as_bytes());

    let refresh_response = router
        .oneshot(
            Request::post("/api/v1/auth/refresh")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&json!({
                    "refresh_token": first_refresh.clone()
                }))?))?,
        )
        .await?;
    assert_eq!(refresh_response.status(), StatusCode::OK);
    let refresh_body: Value =
        serde_json::from_slice(&to_bytes(refresh_response.into_body(), 8192).await?)?;
    let rotated_refresh = assert_exact_token_pair(&refresh_body)?;
    assert_ne!(rotated_refresh, first_refresh);

    pool.close().await;
    database.dispose().await
}

#[tokio::test]
async fn attempt_is_consumed_before_pkce_failure_and_never_reaches_provider() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let fixture = harness(pool.clone(), None)?;
    let state = authorize(&fixture.service).await?;
    let input = |verifier: &str| ExchangeInput {
        authorization_code: "provider-code".to_owned(),
        state: state.clone(),
        code_verifier: verifier.to_owned(),
        redirect_uri: KAKAO_REDIRECT.to_owned(),
    };
    let wrong_verifier = "Z".repeat(43);
    assert_eq!(
        fixture
            .service
            .exchange("kakao", input(&wrong_verifier), "ip:fixture")
            .await,
        Err(AuthError::OAuthExchangeInvalid)
    );
    assert_eq!(
        fixture
            .service
            .exchange("kakao", input(TEST_VERIFIER), "ip:fixture")
            .await,
        Err(AuthError::OAuthExchangeInvalid)
    );
    assert_eq!(fixture.attempts.len()?, 0);
    assert_eq!(fixture.provider.exchange_calls(), 0);
    pool.close().await;
    database.dispose().await
}

#[tokio::test]
async fn provider_failure_keeps_the_consumed_attempt_and_zero_identity_mutation() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let fixture = harness_with_provider_error(pool.clone(), OAuthProviderError::Unavailable)?;
    let state = authorize(&fixture.service).await?;

    assert_eq!(
        fixture
            .service
            .exchange(
                "kakao",
                ExchangeInput {
                    authorization_code: "provider-timeout-code".to_owned(),
                    state: state.clone(),
                    code_verifier: TEST_VERIFIER.to_owned(),
                    redirect_uri: KAKAO_REDIRECT.to_owned(),
                },
                "ip:fixture",
            )
            .await,
        Err(AuthError::OAuthProviderUnavailable)
    );
    assert_eq!(fixture.attempts.len()?, 0);
    assert_eq!(fixture.provider.exchange_calls(), 1);
    assert_eq!(
        fixture
            .service
            .exchange(
                "kakao",
                ExchangeInput {
                    authorization_code: "provider-timeout-code".to_owned(),
                    state,
                    code_verifier: TEST_VERIFIER.to_owned(),
                    redirect_uri: KAKAO_REDIRECT.to_owned(),
                },
                "ip:fixture",
            )
            .await,
        Err(AuthError::OAuthExchangeInvalid)
    );
    assert_eq!(fixture.provider.exchange_calls(), 1);
    let rows: i64 = sqlx::query_scalar(
        "SELECT (SELECT count(*) FROM users) + \
                (SELECT count(*) FROM auth_identities) + \
                (SELECT count(*) FROM refresh_sessions)",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(rows, 0);

    pool.close().await;
    database.dispose().await
}

#[tokio::test]
async fn malformed_provider_identity_is_rejected_before_database_mutation() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let fixture = harness_with_provider_identity(
        pool.clone(),
        jamye_server::ports::oauth_provider::ProviderIdentity {
            provider_id: "p".repeat(129),
            nickname: "malformed".to_owned(),
            avatar_url: None,
        },
    )?;
    let state = authorize(&fixture.service).await?;
    assert_eq!(
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
            .await,
        Err(AuthError::OAuthProviderUnavailable)
    );
    let rows: i64 = sqlx::query_scalar(
        "SELECT (SELECT count(*) FROM users) + \
                (SELECT count(*) FROM auth_identities) + \
                (SELECT count(*) FROM refresh_sessions)",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(rows, 0);

    pool.close().await;
    database.dispose().await
}

#[test]
fn production_bearer_verifies_exact_issuer_audience_and_expiry() -> TestResult {
    let codec = ProductionTokenCodec::new(
        b"task-5-production-token-secret-32-bytes",
        "https://api.jamye.test",
        "jamye-mobile",
    )?;
    let wrong_issuer = ProductionTokenCodec::new(
        b"task-5-production-token-secret-32-bytes",
        "https://other.jamye.test",
        "jamye-mobile",
    )?;
    let wrong_audience = ProductionTokenCodec::new(
        b"task-5-production-token-secret-32-bytes",
        "https://api.jamye.test",
        "other-client",
    )?;
    let now = OffsetDateTime::now_utc();
    let token = codec.issue(
        Uuid::new_v4(),
        Uuid::new_v4(),
        now,
        now + time::Duration::minutes(5),
    )?;
    assert!(codec.verify(&token).is_ok());
    assert!(wrong_issuer.verify(&token).is_err());
    assert!(wrong_audience.verify(&token).is_err());
    let expired = codec.issue(
        Uuid::new_v4(),
        Uuid::new_v4(),
        now - time::Duration::minutes(10),
        now - time::Duration::minutes(5),
    )?;
    assert!(codec.verify(&expired).is_err());
    assert!(
        ProductionTokenCodec::new(
            b"task-5-production-token-secret-32-bytes",
            "jamye-dev",
            "jamye-mobile"
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn enabled_provider_configuration_is_complete_exact_and_secret_safe() -> TestResult {
    assert_eq!(OAUTH_ATTEMPT_TTL, Duration::from_secs(600));
    let config = AuthConfig::try_from(AuthConfigInput {
        kakao_enabled: Some("true".to_owned()),
        kakao_client_id: Some("client-id".to_owned()),
        kakao_client_secret: Some("client-secret".to_owned()),
        kakao_redirect_uris: Some(KAKAO_REDIRECT.to_owned()),
        google_enabled: Some("false".to_owned()),
        access_token_secret: Some("a-secret-with-at-least-thirty-two-bytes".to_owned()),
        access_token_issuer: Some("https://api.jamye.test".to_owned()),
        access_token_audience: Some("jamye-mobile".to_owned()),
        ..AuthConfigInput::default()
    })?;
    assert!(config.kakao.enabled);
    assert!(!config.google.enabled);
    assert_eq!(config.kakao.redirect_uris, vec![KAKAO_REDIRECT]);
    assert_eq!(format!("{:?}", config.access_token_secret), "[REDACTED]");

    let incomplete = AuthConfig::try_from(AuthConfigInput {
        kakao_enabled: Some("true".to_owned()),
        kakao_client_id: Some("client-id".to_owned()),
        kakao_redirect_uris: Some(KAKAO_REDIRECT.to_owned()),
        access_token_secret: Some("a-secret-with-at-least-thirty-two-bytes".to_owned()),
        access_token_issuer: Some("https://api.jamye.test".to_owned()),
        access_token_audience: Some("jamye-mobile".to_owned()),
        ..AuthConfigInput::default()
    });
    assert!(incomplete.is_err());
    let wildcard = AuthConfig::try_from(AuthConfigInput {
        kakao_enabled: Some("true".to_owned()),
        kakao_client_id: Some("client-id".to_owned()),
        kakao_client_secret: Some("client-secret".to_owned()),
        kakao_redirect_uris: Some("jamye://oauth/*".to_owned()),
        access_token_secret: Some("a-secret-with-at-least-thirty-two-bytes".to_owned()),
        access_token_issuer: Some("https://api.jamye.test".to_owned()),
        access_token_audience: Some("jamye-mobile".to_owned()),
        ..AuthConfigInput::default()
    });
    assert!(wildcard.is_err());
    Ok(())
}

#[test]
fn rate_limit_configuration_has_conservative_defaults_and_rejects_invalid_overrides() -> TestResult
{
    let defaults = RateLimitConfig::default();
    assert_eq!(defaults.auth.authorize.limit, 10);
    assert_eq!(defaults.auth.exchange.limit, 20);
    assert_eq!(defaults.auth.refresh.limit, 30);
    assert_eq!(defaults.auth.logout.limit, 30);
    assert_eq!(defaults.auth.authorize.window, Duration::from_secs(60));

    let overridden = RateLimitConfig::try_from(RateLimitConfigInput {
        authorize_limit: Some("7".to_owned()),
        authorize_window_seconds: Some("90".to_owned()),
        refresh_limit: Some("45".to_owned()),
        ..RateLimitConfigInput::default()
    })?;
    assert_eq!(overridden.auth.authorize.limit, 7);
    assert_eq!(overridden.auth.authorize.window, Duration::from_secs(90));
    assert_eq!(overridden.auth.refresh.limit, 45);
    assert_eq!(overridden.auth.exchange.limit, 20);

    let invalid = RateLimitConfig::try_from(RateLimitConfigInput {
        logout_limit: Some("0".to_owned()),
        ..RateLimitConfigInput::default()
    });
    assert_eq!(
        invalid.as_ref().map_err(|error| error.key()),
        Err("JAMYE_RATE_LIMIT_AUTH_LOGOUT_LIMIT")
    );
    Ok(())
}

struct UnavailableRateLimiter;

impl RateLimiter for UnavailableRateLimiter {
    fn check<'a>(&'a self, _request: &'a RateLimitRequest) -> RateLimitFuture<'a> {
        Box::pin(async { Err(RateLimitError) })
    }
}

struct DenyRateLimiter(Duration);

impl RateLimiter for DenyRateLimiter {
    fn check<'a>(&'a self, _request: &'a RateLimitRequest) -> RateLimitFuture<'a> {
        Box::pin(async move {
            Ok(RateLimitOutcome::Denied {
                retry_after: self.0,
            })
        })
    }
}

fn authorize_request(
    code_challenge: &str,
    code_challenge_method: &str,
    redirect_uri: &str,
) -> TestResult<Request<Body>> {
    Ok(Request::post("/api/v1/auth/oauth/kakao/authorize")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&json!({
            "redirect_uri": redirect_uri,
            "code_challenge": code_challenge,
            "code_challenge_method": code_challenge_method
        }))?))?)
}

async fn assert_error_code(
    response: axum::response::Response,
    expected_status: StatusCode,
    expected_code: &str,
) -> TestResult {
    assert_eq!(response.status(), expected_status);
    let body: Value = serde_json::from_slice(&to_bytes(response.into_body(), 4096).await?)?;
    assert_eq!(body["error"]["code"], expected_code);
    assert!(body["error"]["details"].is_null());
    Ok(())
}

fn assert_exact_token_pair(body: &Value) -> TestResult<String> {
    let object = body
        .as_object()
        .ok_or_else(|| io::Error::other("token response must be an object"))?;
    let actual = object.keys().map(String::as_str).collect::<BTreeSet<_>>();
    let expected = [
        "access_token",
        "access_token_expires_at",
        "refresh_token",
        "refresh_token_expires_at",
        "token_type",
    ]
    .into_iter()
    .collect::<BTreeSet<_>>();
    assert_eq!(actual, expected);
    assert_eq!(body["token_type"], "Bearer");
    assert!(
        body["access_token"]
            .as_str()
            .is_some_and(|token| !token.is_empty())
    );
    let refresh_token = body["refresh_token"]
        .as_str()
        .ok_or_else(|| io::Error::other("token response omitted refresh token"))?;
    assert_eq!(refresh_token.len(), 43);
    let access_expiry = OffsetDateTime::parse(
        body["access_token_expires_at"]
            .as_str()
            .ok_or_else(|| io::Error::other("token response omitted access expiry"))?,
        &Rfc3339,
    )?;
    let refresh_expiry = OffsetDateTime::parse(
        body["refresh_token_expires_at"]
            .as_str()
            .ok_or_else(|| io::Error::other("token response omitted refresh expiry"))?,
        &Rfc3339,
    )?;
    assert!(refresh_expiry > access_expiry);
    Ok(refresh_token.to_owned())
}
