#![allow(
    dead_code,
    reason = "each admin CLI test module uses a different subset of the shared harness"
)]

//! Admin CLI harness: the production `compose` over a disposable PostgreSQL database, plus
//! row-level fixtures the CLI reads.

use jamye_server::{
    config::{AppConfig, ConfigInput},
    transport::admin::{
        cli::{self, Parsed},
        runtime::{AdminServices, CommandError, compose, execute},
    },
};
use serde_json::Value;
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{TestResult, postgres_support::TestDatabase};

pub struct AdminHarness {
    pub database: TestDatabase,
    pub pool: PgPool,
    pub services: AdminServices,
}

impl AdminHarness {
    pub async fn new() -> TestResult<Self> {
        let database = TestDatabase::migrated().await?;
        let pool = database.pool()?;
        let config = AppConfig::try_from(ConfigInput {
            environment: Some("test".to_owned()),
            database_url: Some(database.database_url().to_owned()),
            ..ConfigInput::default()
        })?;
        let services = compose(&config)?;
        Ok(Self {
            database,
            pool,
            services,
        })
    }

    /// Parses `args` like the binary does and executes the command.
    pub async fn run(&self, args: &[&str]) -> Result<String, CommandError> {
        match cli::parse(args.iter().copied()) {
            Ok(Parsed::Help) => Ok(cli::USAGE.to_owned()),
            Ok(Parsed::Run(command)) => execute(&self.services, command).await,
            Err(error) => Err(CommandError::Usage(error.to_string())),
        }
    }

    pub async fn finish(self, result: TestResult) -> TestResult {
        self.pool.close().await;
        let cleanup = self.database.dispose().await;
        match (result, cleanup) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(failure), Ok(())) => Err(failure),
            (Ok(()), Err(cleanup_failure)) => Err(cleanup_failure),
            (Err(failure), Err(cleanup_failure)) => Err(format!(
                "admin CLI test failed: {failure}; database cleanup also failed: {cleanup_failure}"
            )
            .into()),
        }
    }
}

/// What a report row is inserted with.
pub struct ReportFixture {
    pub reporter_id: Uuid,
    pub group_id: Uuid,
    pub message_id: Option<Uuid>,
    pub user_id: Option<Uuid>,
    pub reason: &'static str,
    pub snapshot: Option<Value>,
    pub status: &'static str,
    pub created_at: OffsetDateTime,
    pub handled_at: Option<OffsetDateTime>,
}

impl ReportFixture {
    pub fn message(reporter_id: Uuid, group_id: Uuid, message_id: Uuid, text: &str) -> Self {
        Self {
            reporter_id,
            group_id,
            message_id: Some(message_id),
            user_id: None,
            reason: "harassment",
            snapshot: Some(serde_json::json!({
                "text": text,
                "content_type": "text",
                "media": [],
            })),
            status: "open",
            created_at: OffsetDateTime::now_utc(),
            handled_at: None,
        }
    }

    pub fn user(reporter_id: Uuid, group_id: Uuid, user_id: Uuid) -> Self {
        Self {
            reporter_id,
            group_id,
            message_id: None,
            user_id: Some(user_id),
            reason: "spam",
            snapshot: None,
            status: "open",
            created_at: OffsetDateTime::now_utc(),
            handled_at: None,
        }
    }

    /// A handled report: `status` with `handled_at` that many days in the past.
    pub fn handled(mut self, status: &'static str, days_ago: i64) -> Self {
        let at = OffsetDateTime::now_utc() - time::Duration::days(days_ago);
        self.status = status;
        self.created_at = at - time::Duration::hours(1);
        self.handled_at = Some(at);
        self
    }

    pub fn created_days_ago(mut self, days_ago: i64) -> Self {
        self.created_at = OffsetDateTime::now_utc() - time::Duration::days(days_ago);
        self
    }

    pub async fn insert(self, pool: &PgPool) -> TestResult<Uuid> {
        let id = Uuid::new_v4();
        let target_type = if self.message_id.is_some() {
            "message"
        } else {
            "user"
        };
        sqlx::query(
            "INSERT INTO reports \
                 (id, reporter_id, target_type, target_message_id, target_user_id, \
                  target_group_id, reason, message_snapshot, status, handled_at, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
        )
        .bind(id)
        .bind(self.reporter_id)
        .bind(target_type)
        .bind(self.message_id)
        .bind(self.user_id)
        .bind(self.group_id)
        .bind(self.reason)
        .bind(&self.snapshot)
        .bind(self.status)
        .bind(self.handled_at)
        .bind(self.created_at)
        .execute(pool)
        .await?;
        Ok(id)
    }
}

pub async fn report_status(pool: &PgPool, report_id: Uuid) -> TestResult<(String, bool)> {
    Ok(sqlx::query_as::<_, (String, bool)>(
        "SELECT status, handled_at IS NOT NULL FROM reports WHERE id = $1",
    )
    .bind(report_id)
    .fetch_one(pool)
    .await?)
}

pub async fn report_exists(pool: &PgPool, report_id: Uuid) -> TestResult<bool> {
    Ok(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM reports WHERE id = $1")
            .bind(report_id)
            .fetch_one(pool)
            .await?
            == 1,
    )
}

/// Inserts a refresh session with a token hash that is unique per call (the column is unique).
pub async fn insert_session(pool: &PgPool, user_id: Uuid) -> TestResult<Uuid> {
    let session_id = Uuid::new_v4();
    let mut token_hash = Vec::with_capacity(32);
    token_hash.extend_from_slice(session_id.as_bytes());
    token_hash.extend_from_slice(Uuid::new_v4().as_bytes());
    sqlx::query(
        "INSERT INTO refresh_sessions (id, user_id, family_id, token_hash, expires_at) \
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(session_id)
    .bind(user_id)
    .bind(Uuid::new_v4())
    .bind(token_hash)
    .bind(OffsetDateTime::now_utc() + time::Duration::days(30))
    .execute(pool)
    .await?;
    Ok(session_id)
}

/// Asserts the failure is an operational failure (exit status 1) whose message contains `needle`.
pub fn assert_failure(result: Result<String, CommandError>, needle: &str) -> TestResult {
    match result {
        Err(error @ CommandError::Failure(_)) => {
            assert_eq!(error.exit_code(), 1);
            assert!(
                error.message().contains(needle),
                "failure `{}` does not mention `{needle}`",
                error.message()
            );
            Ok(())
        }
        other => Err(format!("expected an operational failure, got {other:?}").into()),
    }
}

/// Asserts the failure is a usage error (exit status 2).
pub fn assert_usage(result: Result<String, CommandError>) -> TestResult {
    match result {
        Err(error @ CommandError::Usage(_)) => {
            assert_eq!(error.exit_code(), 2);
            Ok(())
        }
        other => Err(format!("expected a usage error, got {other:?}").into()),
    }
}
