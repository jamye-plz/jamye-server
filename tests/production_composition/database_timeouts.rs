use std::{io, time::Duration};

use jamye_server::adapters::postgres::runtime_pool;

use crate::{TestResult, postgres_support::TestDatabase};

#[tokio::test]
async fn runtime_pool_applies_postgres_session_timeouts() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = runtime_pool(database.database_url(), Duration::from_secs(1))?;

    let result: TestResult = async {
        let mut connection = pool.acquire().await?;
        let lock_timeout: String = sqlx::query_scalar("SHOW lock_timeout")
            .fetch_one(&mut *connection)
            .await?;
        let idle_in_transaction_session_timeout: String =
            sqlx::query_scalar("SHOW idle_in_transaction_session_timeout")
                .fetch_one(&mut *connection)
                .await?;
        let statement_timeout: String = sqlx::query_scalar("SHOW statement_timeout")
            .fetch_one(&mut *connection)
            .await?;

        assert_eq!(postgres_duration(&lock_timeout)?, Duration::from_secs(5));
        assert_eq!(
            postgres_duration(&idle_in_transaction_session_timeout)?,
            Duration::from_secs(30)
        );
        assert_eq!(
            postgres_duration(&statement_timeout)?,
            Duration::from_secs(120)
        );
        Ok(())
    }
    .await;

    pool.close().await;
    let cleanup = database.dispose().await;
    result.and(cleanup)
}

fn postgres_duration(value: &str) -> TestResult<Duration> {
    let (amount, multiplier) = if let Some(amount) = value.strip_suffix("ms") {
        (amount, 1_u64)
    } else if let Some(amount) = value.strip_suffix('s') {
        (amount, 1_000)
    } else if let Some(amount) = value.strip_suffix("min") {
        (amount, 60_000)
    } else if let Some(amount) = value.strip_suffix('h') {
        (amount, 3_600_000)
    } else {
        return Err(io::Error::other(format!("unsupported PostgreSQL duration: {value}")).into());
    };
    let milliseconds = amount
        .parse::<u64>()?
        .checked_mul(multiplier)
        .ok_or_else(|| io::Error::other(format!("PostgreSQL duration overflow: {value}")))?;
    Ok(Duration::from_millis(milliseconds))
}
