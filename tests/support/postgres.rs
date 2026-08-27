#![allow(
    dead_code,
    reason = "integration test crates compile different subsets of the shared PostgreSQL helpers"
)]

use std::{env, error::Error, io, path::Path};

use sqlx::{AssertSqlSafe, Connection, PgConnection, PgPool, postgres::PgPoolOptions};
use url::Url;
use uuid::Uuid;

const DISPOSABLE_DATABASE_PREFIX: &str = "jamye_task_test_";

pub type TestResult<T = ()> = Result<T, Box<dyn Error + Send + Sync>>;

pub struct TestDatabase {
    admin: PgConnection,
    database_name: String,
    database_url: String,
}

impl TestDatabase {
    pub async fn migrated() -> TestResult<Self> {
        Self::create(None).await
    }

    pub async fn migrated_to(version: i64) -> TestResult<Self> {
        Self::create(Some(version)).await
    }

    async fn create(target_version: Option<i64>) -> TestResult<Self> {
        let base_url = env::var("DATABASE_URL")
            .map_err(|_| test_error("DATABASE_URL is required for integration tests"))?;
        let mut parsed = validate_base_url(&base_url)?;
        let database_name = format!("{DISPOSABLE_DATABASE_PREFIX}{}", Uuid::new_v4().simple());
        let database_identifier = quoted_identifier(&database_name)?;
        let mut admin = PgConnection::connect(&base_url).await?;

        sqlx::query(AssertSqlSafe(format!(
            "CREATE DATABASE {database_identifier}"
        )))
        .execute(&mut admin)
        .await?;
        parsed.set_path(&format!("/{database_name}"));
        let database_url = parsed.to_string();

        let migration_result: TestResult = async {
            let migrator = sqlx::migrate::Migrator::new(Path::new("migrations")).await?;
            let mut connection = PgConnection::connect(&database_url).await?;
            match target_version {
                Some(version) => migrator.run_to(version, &mut connection).await?,
                None => migrator.run(&mut connection).await?,
            }
            connection.close().await?;
            Ok(())
        }
        .await;

        if let Err(error) = migration_result {
            let cleanup_result = sqlx::query(AssertSqlSafe(format!(
                "DROP DATABASE {database_identifier} WITH (FORCE)"
            )))
            .execute(&mut admin)
            .await;
            return match cleanup_result {
                Ok(_) => Err(error),
                Err(cleanup_error) => Err(test_error(format!(
                    "migration failed: {error}; cleanup also failed for {database_name}: {cleanup_error}"
                ))),
            };
        }

        Ok(Self {
            admin,
            database_name,
            database_url,
        })
    }

    pub async fn connection(&self) -> TestResult<PgConnection> {
        Ok(PgConnection::connect(&self.database_url).await?)
    }

    pub fn pool(&self) -> TestResult<PgPool> {
        Ok(PgPoolOptions::new()
            .max_connections(4)
            .connect_lazy(&self.database_url)?)
    }

    pub async fn dispose(mut self) -> TestResult {
        let database_identifier = quoted_identifier(&self.database_name)?;
        sqlx::query(AssertSqlSafe(format!(
            "DROP DATABASE {database_identifier} WITH (FORCE)"
        )))
        .execute(&mut self.admin)
        .await?;
        self.admin.close().await?;
        Ok(())
    }
}

fn validate_base_url(database_url: &str) -> TestResult<Url> {
    let environment = env::var("JAMYE_ENVIRONMENT")
        .map_err(|_| test_error("JAMYE_ENVIRONMENT is required for integration tests"))?;
    if environment != "test" {
        return Err(test_error(
            "integration tests refuse to run unless JAMYE_ENVIRONMENT=test",
        ));
    }

    let parsed = Url::parse(database_url)?;
    if !matches!(parsed.scheme(), "postgres" | "postgresql") {
        return Err(test_error("integration tests require a PostgreSQL URL"));
    }
    if !matches!(parsed.host_str(), Some("127.0.0.1" | "localhost" | "::1")) {
        return Err(test_error(
            "integration tests only accept a loopback PostgreSQL host",
        ));
    }
    if parsed.path() != "/jamye_test" {
        return Err(test_error(
            "integration tests only accept the disposable jamye_test database",
        ));
    }
    Ok(parsed)
}

fn quoted_identifier(identifier: &str) -> TestResult<String> {
    let valid = identifier.starts_with(DISPOSABLE_DATABASE_PREFIX)
        && identifier.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_'
        });
    if !valid {
        return Err(test_error("refused unsafe disposable database identifier"));
    }
    Ok(format!("\"{identifier}\""))
}

fn test_error(message: impl Into<String>) -> Box<dyn Error + Send + Sync> {
    Box::new(io::Error::other(message.into()))
}
