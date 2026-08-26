use std::{error::Error, fs, io};

#[cfg(feature = "dev-fixtures")]
mod support;

#[cfg(feature = "dev-fixtures")]
use std::sync::Arc;

#[cfg(feature = "dev-fixtures")]
use axum::{
    Json, Router,
    body::{Body, to_bytes},
    http::{Request, Response, StatusCode, header::AUTHORIZATION},
    routing::get,
};
#[cfg(feature = "dev-fixtures")]
use jamye_server::{
    adapters::postgres::dev_fixtures::PostgresDevFixtureStore,
    application::auth::AccessIdentity,
    dev_fixtures::{
        AUDIENCE, DevAccessClaims, DevFixtureGuard, DevTokenCodec, ISSUER, SeededFixture,
    },
    transport::http::{
        auth::{AuthVerifierState, AuthenticatedAccess},
        dev_fixtures::{DevFixtureHttpState, router as dev_fixture_router},
    },
};
#[cfg(feature = "dev-fixtures")]
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
#[cfg(feature = "dev-fixtures")]
use serde_json::Value;
#[cfg(feature = "dev-fixtures")]
use sqlx::PgPool;
#[cfg(feature = "dev-fixtures")]
use time::OffsetDateTime;
#[cfg(feature = "dev-fixtures")]
use tower::ServiceExt;
#[cfg(feature = "dev-fixtures")]
use uuid::Uuid;

type TestResult<T = ()> = Result<T, Box<dyn Error + Send + Sync>>;

const DEV_FIXTURE_ROOT: &str = "src/dev_fixtures/mod.rs";
const DEV_FIXTURE_HTTP: &str = "src/transport/http/dev_fixtures/mod.rs";
const APPLICATION_AUTH_ROOT: &str = "src/application/auth/mod.rs";
const HTTP_AUTH_ROOT: &str = "src/transport/http/auth/mod.rs";

#[test]
fn dev_fixture_surface_is_feature_and_environment_guarded() -> TestResult {
    let dev_fixture_root = required_source(DEV_FIXTURE_ROOT)?;
    let dev_fixture_http = required_source(DEV_FIXTURE_HTTP)?;
    required_source(APPLICATION_AUTH_ROOT)?;
    required_source(HTTP_AUTH_ROOT)?;

    let manifest = fs::read_to_string("Cargo.toml")?;
    assert!(manifest.contains("default = []"));
    assert!(manifest.contains("dev-fixtures = []"));
    assert!(manifest.contains("jsonwebtoken = { version = \"11.0.0\""));
    assert!(manifest.contains("default-features = false"));
    assert!(manifest.contains("features = [\"aws_lc_rs\"]"));
    assert!(!manifest.contains("features = [\"rust_crypto\"]"));
    assert!(!manifest.contains("optional = true"));

    let library_root = fs::read_to_string("src/lib.rs")?;
    assert!(library_root.contains("#[cfg(feature = \"dev-fixtures\")]"));
    assert!(library_root.contains("pub mod dev_fixtures;"));

    let application_root = fs::read_to_string("src/application/mod.rs")?;
    assert!(application_root.contains("pub mod auth;"));

    let http_root = fs::read_to_string("src/transport/http/mod.rs")?;
    assert!(http_root.contains("pub mod auth;"));
    assert!(http_root.contains("#[cfg(feature = \"dev-fixtures\")]"));
    assert!(http_root.contains("pub mod dev_fixtures;"));

    for required in [
        "JAMYE_ENABLE_DEV_FIXTURES",
        "JAMYE_ENVIRONMENT",
        "jamye-dev",
        "jamye-api",
    ] {
        assert!(
            dev_fixture_root.contains(required),
            "dev fixture implementation is missing guard/claim marker: {required}"
        );
    }
    assert!(dev_fixture_http.contains("pub fn router"));
    assert!(dev_fixture_http.contains("/__dev/fixtures/seed"));

    for production_root in ["src/bin/api.rs", "src/transport/http/composition.rs"] {
        let source = fs::read_to_string(production_root)?;
        assert!(
            !source.contains("dev_fixtures"),
            "default production root unexpectedly references dev fixtures: {production_root}"
        );
    }

    Ok(())
}

#[cfg(feature = "dev-fixtures")]
#[tokio::test]
async fn shared_helpers_seed_owner_fixture_without_http() -> TestResult {
    let database = support::TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let fixture = support::insert_owner_fixture(&pool).await?;

    let stored = sqlx::query_as::<_, (Uuid, Uuid, Uuid, String, Uuid, String)>(
        "SELECT g.owner_id, m.group_id, m.user_id, m.role, c.group_id, c.type \
         FROM groups g \
         JOIN memberships m ON m.group_id = g.id \
         JOIN chatrooms c ON c.group_id = g.id \
         WHERE g.id = $1 AND m.id = $2 AND c.id = $3",
    )
    .bind(fixture.group_id)
    .bind(fixture.membership_id)
    .bind(fixture.chatroom_id)
    .fetch_one(&pool)
    .await?;

    assert_eq!(stored.0, fixture.user_id);
    assert_eq!(stored.1, fixture.group_id);
    assert_eq!(stored.2, fixture.user_id);
    assert_eq!(stored.3, "owner");
    assert_eq!(stored.4, fixture.group_id);
    assert_eq!(stored.5, "main");

    pool.close().await;
    database.dispose().await
}

#[cfg(feature = "dev-fixtures")]
#[tokio::test]
async fn real_seed_endpoint_is_atomic_and_issues_a_verified_identity() -> TestResult {
    let database = support::TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let guard = DevFixtureGuard::from_env()?;
    let secret = ephemeral_secret();
    let codec = DevTokenCodec::from_secret(guard, &secret)?;
    let store = Arc::new(PostgresDevFixtureStore::new(pool.clone()));
    let state = DevFixtureHttpState::new(store, codec);
    let auth_state = state.auth_state();

    let response = dev_fixture_router(state)
        .oneshot(Request::post("/__dev/fixtures/seed").body(Body::empty())?)
        .await?;
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let fixture: SeededFixture = serde_json::from_slice(&body)?;

    assert_seeded_rows(&pool, &fixture).await?;
    assert_short_lived_claims(&fixture, &secret)?;

    let protected = Router::new()
        .route("/protected", get(protected_identity))
        .with_state(auth_state);
    let response = protected
        .oneshot(
            Request::get("/protected")
                .header(AUTHORIZATION, format!("Bearer {}", fixture.access_token))
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let identity: AccessIdentity = serde_json::from_slice(&body)?;
    assert_eq!(identity.user_id, fixture.user_id);
    assert_eq!(identity.issuer, ISSUER);

    pool.close().await;
    database.dispose().await
}

#[cfg(feature = "dev-fixtures")]
#[tokio::test]
async fn failed_seed_rolls_back_every_fixture_row() -> TestResult {
    let database = support::TestDatabase::migrated().await?;
    let pool = database.pool()?;
    sqlx::query(
        "CREATE FUNCTION reject_dev_fixture_group() RETURNS trigger AS $$ \
         BEGIN RAISE EXCEPTION 'forced task-3c rollback'; END; \
         $$ LANGUAGE plpgsql",
    )
    .execute(&pool)
    .await?;
    sqlx::query(
        "CREATE TRIGGER reject_dev_fixture_group \
         BEFORE INSERT ON groups \
         FOR EACH ROW EXECUTE FUNCTION reject_dev_fixture_group()",
    )
    .execute(&pool)
    .await?;

    let guard = DevFixtureGuard::from_env()?;
    let codec = DevTokenCodec::from_secret(guard, ephemeral_secret())?;
    let state =
        DevFixtureHttpState::new(Arc::new(PostgresDevFixtureStore::new(pool.clone())), codec);
    let response = dev_fixture_router(state)
        .oneshot(Request::post("/__dev/fixtures/seed").body(Body::empty())?)
        .await?;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let envelope: Value = serde_json::from_slice(&body)?;
    assert_eq!(
        envelope["error"]["code"].as_str(),
        Some("database_unavailable")
    );

    for (table, query) in [
        ("users", "SELECT COUNT(*) FROM users"),
        ("groups", "SELECT COUNT(*) FROM groups"),
        ("memberships", "SELECT COUNT(*) FROM memberships"),
        ("chatrooms", "SELECT COUNT(*) FROM chatrooms"),
    ] {
        let count = sqlx::query_scalar::<_, i64>(query).fetch_one(&pool).await?;
        assert_eq!(count, 0, "failed seed left rows in {table}");
    }

    pool.close().await;
    database.dispose().await
}

#[cfg(feature = "dev-fixtures")]
#[tokio::test]
async fn bearer_extractor_rejects_every_invalid_dev_claim_class() -> TestResult {
    let guard = DevFixtureGuard::from_env()?;
    let secret = ephemeral_secret();
    let codec = DevTokenCodec::from_secret(guard, &secret)?;
    let router = Router::new()
        .route("/protected", get(protected_identity))
        .with_state(AuthVerifierState::new(Arc::new(codec)));
    let now = u64::try_from(OffsetDateTime::now_utc().unix_timestamp())?;
    let valid = claims(
        Uuid::new_v4().to_string(),
        Uuid::new_v4().to_string(),
        now + 300,
    );

    let cases = [
        None,
        Some("Basic credential".to_owned()),
        Some("Bearer not.a.jwt".to_owned()),
        Some(format!(
            "Bearer {}",
            signed_token(
                &DevAccessClaims {
                    exp: now - 1,
                    ..valid.clone()
                },
                &secret
            )?
        )),
        Some(format!(
            "Bearer {}",
            signed_token(
                &DevAccessClaims {
                    aud: "wrong-audience".to_owned(),
                    ..valid.clone()
                },
                &secret,
            )?
        )),
        Some(format!(
            "Bearer {}",
            signed_token(
                &DevAccessClaims {
                    iss: "wrong-issuer".to_owned(),
                    ..valid.clone()
                },
                &secret,
            )?
        )),
        Some(format!(
            "Bearer {}",
            signed_token(
                &DevAccessClaims {
                    sub: "not-a-uuid".to_owned(),
                    ..valid.clone()
                },
                &secret,
            )?
        )),
        Some(format!(
            "Bearer {}",
            signed_token(
                &DevAccessClaims {
                    sid: "not-a-uuid".to_owned(),
                    ..valid.clone()
                },
                &secret,
            )?
        )),
        Some(format!(
            "Bearer {}",
            signed_token(&valid, &ephemeral_secret())?
        )),
    ];

    for authorization in cases {
        let request = auth_request(authorization.as_deref())?;
        let response = router.clone().oneshot(request).await?;
        assert_authentication_required(response).await?;
    }
    Ok(())
}

#[cfg(feature = "dev-fixtures")]
async fn protected_identity(
    AuthenticatedAccess(identity): AuthenticatedAccess,
) -> Json<AccessIdentity> {
    Json(identity)
}

#[cfg(feature = "dev-fixtures")]
async fn assert_seeded_rows(pool: &PgPool, fixture: &SeededFixture) -> TestResult {
    let stored = sqlx::query_as::<_, (Uuid, Uuid, Uuid, String, Uuid, String)>(
        "SELECT g.owner_id, m.group_id, m.user_id, m.role, c.group_id, c.type \
         FROM groups g \
         JOIN memberships m ON m.group_id = g.id \
         JOIN chatrooms c ON c.group_id = g.id \
         WHERE g.id = $1 AND m.id = $2 AND c.id = $3",
    )
    .bind(fixture.group_id)
    .bind(fixture.membership_id)
    .bind(fixture.chatroom_id)
    .fetch_one(pool)
    .await?;
    assert_eq!(stored.0, fixture.user_id);
    assert_eq!(stored.1, fixture.group_id);
    assert_eq!(stored.2, fixture.user_id);
    assert_eq!(stored.3, "owner");
    assert_eq!(stored.4, fixture.group_id);
    assert_eq!(stored.5, "main");
    Ok(())
}

#[cfg(feature = "dev-fixtures")]
fn assert_short_lived_claims(fixture: &SeededFixture, secret: &[u8]) -> TestResult {
    let token = decode::<DevAccessClaims>(
        &fixture.access_token,
        &DecodingKey::from_secret(secret),
        &validation(),
    )?;
    let now = u64::try_from(OffsetDateTime::now_utc().unix_timestamp())?;
    assert_eq!(token.claims.sub, fixture.user_id.to_string());
    assert!(Uuid::try_parse(&token.claims.sid).is_ok());
    assert_eq!(token.claims.iss, ISSUER);
    assert_eq!(token.claims.aud, AUDIENCE);
    assert!(token.claims.exp > now);
    assert!(token.claims.exp <= now + 300);
    Ok(())
}

#[cfg(feature = "dev-fixtures")]
fn claims(sub: String, sid: String, exp: u64) -> DevAccessClaims {
    DevAccessClaims {
        sub,
        sid,
        iss: ISSUER.to_owned(),
        aud: AUDIENCE.to_owned(),
        exp,
    }
}

#[cfg(feature = "dev-fixtures")]
fn signed_token(claims: &DevAccessClaims, secret: &[u8]) -> TestResult<String> {
    Ok(encode(
        &Header::new(Algorithm::HS256),
        claims,
        &EncodingKey::from_secret(secret),
    )?)
}

#[cfg(feature = "dev-fixtures")]
fn validation() -> Validation {
    let mut validation = Validation::new(Algorithm::HS256);
    validation.leeway = 0;
    validation.set_audience(&[AUDIENCE]);
    validation.set_issuer(&[ISSUER]);
    validation.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
    validation
}

#[cfg(feature = "dev-fixtures")]
fn ephemeral_secret() -> Vec<u8> {
    let mut secret = Vec::with_capacity(32);
    secret.extend_from_slice(Uuid::new_v4().as_bytes());
    secret.extend_from_slice(Uuid::new_v4().as_bytes());
    secret
}

#[cfg(feature = "dev-fixtures")]
fn auth_request(authorization: Option<&str>) -> TestResult<Request<Body>> {
    let mut request = Request::get("/protected");
    if let Some(authorization) = authorization {
        request = request.header(AUTHORIZATION, authorization);
    }
    Ok(request.body(Body::empty())?)
}

#[cfg(feature = "dev-fixtures")]
async fn assert_authentication_required(response: Response<Body>) -> TestResult {
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let envelope: Value = serde_json::from_slice(&body)?;
    assert_eq!(envelope["error"]["code"], "authentication_required");
    assert!(envelope["error"]["details"].is_null());
    let request_id = envelope["error"]["request_id"]
        .as_str()
        .ok_or_else(|| io::Error::other("authentication error omitted request_id"))?;
    Uuid::try_parse(request_id)?;
    Ok(())
}

fn required_source(path: &str) -> TestResult<String> {
    fs::read_to_string(path).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            io::Error::other(format!(
                "RED: {path} is absent; task-3c must add the guarded dev identity and seed surface"
            ))
            .into()
        } else {
            error.into()
        }
    })
}
