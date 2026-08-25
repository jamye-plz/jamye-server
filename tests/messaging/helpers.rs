use std::{env, sync::Arc, time::Duration};

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, Response, header::AUTHORIZATION},
};
use jamye_server::{
    adapters::postgres::{
        dev_fixtures::PostgresDevFixtureStore, messaging::PostgresMessagingRepository,
        transactions::SqlxTransactionManager,
    },
    application::{
        auth::{AccessIdentity, AccessTokenVerifier, AuthenticationError},
        messaging::MessagingService,
    },
    dev_fixtures::{DevFixtureGuard, DevTokenCodec, SeededFixture},
    transport::http::{
        auth::AuthVerifierState,
        dev_fixtures::{DevFixtureHttpState, router as dev_fixture_router},
        messaging::{MessagingHttpState, router as messaging_router},
    },
};
use serde_json::Value;
use sqlx::{AssertSqlSafe, Connection, PgConnection, PgPool, postgres::PgPoolOptions};
use tower::ServiceExt;
use uuid::Uuid;

use crate::postgres_support::{TestDatabase, TestResult};

pub struct TestApp {
    database: TestDatabase,
    pub pool: PgPool,
    pub fixture: SeededFixture,
    fixture_state: DevFixtureHttpState,
    pub router: Router,
}

impl TestApp {
    pub async fn new() -> TestResult<Self> {
        let database = TestDatabase::migrated().await?;
        let pool = database.pool()?;
        let guard = DevFixtureGuard::from_env()?;
        let codec = DevTokenCodec::ephemeral(guard);
        let fixture_state =
            DevFixtureHttpState::new(Arc::new(PostgresDevFixtureStore::new(pool.clone())), codec);
        let fixture = seed_fixture(&fixture_state).await?;
        let router = router_for_pool(pool.clone(), fixture_state.auth_state());
        Ok(Self {
            database,
            pool,
            fixture,
            fixture_state,
            router,
        })
    }

    pub async fn seed_another(&self) -> TestResult<SeededFixture> {
        seed_fixture(&self.fixture_state).await
    }

    pub async fn send(
        &self,
        token: Option<&str>,
        chatroom_id: Uuid,
        payload: Value,
        idempotency_key: Option<&str>,
    ) -> TestResult<Response<Body>> {
        send_to(&self.router, token, chatroom_id, payload, idempotency_key).await
    }

    pub async fn events(
        &self,
        token: Option<&str>,
        conversation_id: Uuid,
        after: Option<&str>,
        limit: u32,
        version: Option<&str>,
    ) -> TestResult<Response<Body>> {
        events_from(&self.router, token, conversation_id, after, limit, version).await
    }

    pub async fn dispose(self) -> TestResult {
        self.pool.close().await;
        self.database.dispose().await
    }

    pub fn restartable_router(&self) -> TestResult<(Router, PgPool)> {
        let connect_options = self.pool.connect_options();
        let database_name = connect_options
            .get_database()
            .ok_or_else(|| {
                std::io::Error::other("disposable database URL omitted its database name")
            })?
            .to_owned();
        let mut parsed = url::Url::parse(
            &env::var("DATABASE_URL")
                .map_err(|_| std::io::Error::other("DATABASE_URL is required"))?,
        )?;
        parsed.set_path(&format!("/{database_name}"));
        let pool = PgPoolOptions::new()
            .max_connections(2)
            .acquire_timeout(Duration::from_millis(500))
            .connect_lazy(parsed.as_str())?;
        let router = router_for_pool(pool.clone(), self.fixture_state.auth_state());
        Ok((router, pool))
    }

    pub async fn dispose_after_postgres_restart(self, recovery_pool: PgPool) -> TestResult {
        let database_name = sqlx::query_scalar::<_, String>("SELECT current_database()")
            .fetch_one(&recovery_pool)
            .await?;
        let database_identifier = quoted_disposable_database(&database_name)?;

        recovery_pool.close().await;
        self.pool.close().await;
        drop(self.database);

        let admin_url = guarded_admin_url()?;
        let mut admin = PgConnection::connect(admin_url.as_str()).await?;
        sqlx::query(AssertSqlSafe(format!(
            "DROP DATABASE {database_identifier} WITH (FORCE)"
        )))
        .execute(&mut admin)
        .await?;
        admin.close().await?;
        Ok(())
    }
}

pub fn router_for_pool(pool: PgPool, auth: AuthVerifierState) -> Router {
    let transactions = Arc::new(SqlxTransactionManager::new(pool.clone()));
    let repository = Arc::new(PostgresMessagingRepository::new(pool));
    let service = Arc::new(MessagingService::new(transactions, repository));
    messaging_router(MessagingHttpState::new(service, auth))
}

pub fn router_for_unavailable_database(identity: AccessIdentity) -> TestResult<Router> {
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_millis(100))
        .connect_lazy("postgres://task4a_db_user@127.0.0.1:1/task4a_unavailable_db")?;
    let auth = AuthVerifierState::new(Arc::new(StaticVerifier(identity)));
    Ok(router_for_pool(pool, auth))
}

pub async fn send_to(
    router: &Router,
    token: Option<&str>,
    chatroom_id: Uuid,
    payload: Value,
    idempotency_key: Option<&str>,
) -> TestResult<Response<Body>> {
    let mut request = Request::post(format!("/api/v1/chatrooms/{chatroom_id}/messages"))
        .header("content-type", "application/json");
    if let Some(token) = token {
        request = request.header(AUTHORIZATION, format!("Bearer {token}"));
    }
    if let Some(idempotency_key) = idempotency_key {
        request = request.header("idempotency-key", idempotency_key);
    }
    Ok(router
        .clone()
        .oneshot(request.body(Body::from(serde_json::to_vec(&payload)?))?)
        .await?)
}

pub async fn events_from(
    router: &Router,
    token: Option<&str>,
    conversation_id: Uuid,
    after: Option<&str>,
    limit: u32,
    version: Option<&str>,
) -> TestResult<Response<Body>> {
    let after = after
        .map(|cursor| format!("&after={cursor}"))
        .unwrap_or_default();
    let mut request = Request::get(format!(
        "/api/v1/conversations/{conversation_id}/events?limit={limit}{after}"
    ));
    if let Some(token) = token {
        request = request.header(AUTHORIZATION, format!("Bearer {token}"));
    }
    if let Some(version) = version {
        request = request.header("x-jamye-contract-version", version);
    }
    Ok(router.clone().oneshot(request.body(Body::empty())?).await?)
}

pub async fn json_body(response: Response<Body>) -> TestResult<Value> {
    let bytes = to_bytes(response.into_body(), 256 * 1024).await?;
    Ok(serde_json::from_slice(&bytes)?)
}

pub async fn assert_error(
    response: Response<Body>,
    status: axum::http::StatusCode,
    code: &str,
) -> TestResult {
    assert_eq!(response.status(), status);
    let body = json_body(response).await?;
    assert_eq!(body["error"]["code"], code);
    assert!(body["error"]["details"].is_null());
    Uuid::try_parse(
        body["error"]["request_id"]
            .as_str()
            .ok_or("error response omitted request_id")?,
    )?;
    assert_eq!(body.as_object().map(|object| object.len()), Some(1));
    Ok(())
}

pub async fn counts(pool: &PgPool) -> TestResult<(i64, i64, i64)> {
    let messages = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM messages")
        .fetch_one(pool)
        .await?;
    let events = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM conversation_events")
        .fetch_one(pool)
        .await?;
    let outbox = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM outbox_events")
        .fetch_one(pool)
        .await?;
    Ok((messages, events, outbox))
}

async fn seed_fixture(state: &DevFixtureHttpState) -> TestResult<SeededFixture> {
    let response = dev_fixture_router(state.clone())
        .oneshot(Request::post("/__dev/fixtures/seed").body(Body::empty())?)
        .await?;
    if response.status() != axum::http::StatusCode::CREATED {
        return Err(format!("fixture seed returned {}", response.status()).into());
    }
    let bytes = to_bytes(response.into_body(), 256 * 1024).await?;
    Ok(serde_json::from_slice(&bytes)?)
}

struct StaticVerifier(AccessIdentity);

impl AccessTokenVerifier for StaticVerifier {
    fn verify(&self, _token: &str) -> Result<AccessIdentity, AuthenticationError> {
        Ok(self.0.clone())
    }
}

pub fn test_identity(user_id: Uuid) -> AccessIdentity {
    AccessIdentity::new(user_id, Uuid::new_v4(), "task-4a-test")
}

fn guarded_admin_url() -> TestResult<url::Url> {
    let database_url =
        env::var("DATABASE_URL").map_err(|_| std::io::Error::other("DATABASE_URL is required"))?;
    let parsed = url::Url::parse(&database_url)?;
    if !matches!(parsed.scheme(), "postgres" | "postgresql")
        || !matches!(parsed.host_str(), Some("127.0.0.1" | "localhost" | "::1"))
        || parsed.path() != "/jamye_test"
    {
        return Err(std::io::Error::other(
            "recovery cleanup only accepts the loopback jamye_test database",
        )
        .into());
    }
    Ok(parsed)
}

fn quoted_disposable_database(database_name: &str) -> TestResult<String> {
    let valid = database_name.starts_with("jamye_task_test_")
        && database_name.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_'
        });
    if !valid {
        return Err(
            std::io::Error::other("refused unsafe recovery-test database identifier").into(),
        );
    }
    Ok(format!("\"{database_name}\""))
}
