use std::{
    env, fs, io,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use jamye_server::{
    adapters::{
        oauth::OsCredentialSource,
        redis::{oauth_attempt::RedisOAuthAttemptStore, rate_limit::RedisRateLimiter},
    },
    ports::{
        auth::CredentialSource,
        oauth_attempt::{
            ConsumeAttemptOutcome, CreateAttemptOutcome, OAuthAttempt, OAuthAttemptStore,
        },
        oauth_provider::ProviderKind,
        rate_limit::{RateLimitOutcome, RateLimitRequest, RateLimiter},
    },
};
use url::Url;
use uuid::Uuid;

#[path = "support/postgres.rs"]
mod postgres_support;
use postgres_support::TestDatabase;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[tokio::test]
async fn fixed_window_is_atomic_shared_resettable_and_subject_safe() -> TestResult {
    let redis_url = guarded_redis_url()?;
    let first = Arc::new(RedisRateLimiter::new(&redis_url)?);
    let second = Arc::new(RedisRateLimiter::new(&redis_url)?);
    let subject = format!("ip:task-5:{}", Uuid::new_v4());
    let request = RateLimitRequest {
        endpoint: "oauth_authorize",
        subject: subject.clone(),
        limit: 8,
        window: Duration::from_secs(1),
    };
    assert!(!format!("{request:?}").contains(&subject));

    let mut checks = Vec::new();
    for index in 0..16 {
        let limiter: Arc<dyn RateLimiter> = if index % 2 == 0 {
            first.clone()
        } else {
            second.clone()
        };
        let request = request.clone();
        checks.push(tokio::spawn(async move { limiter.check(&request).await }));
    }

    let mut allowed = 0;
    let mut denied = 0;
    for check in checks {
        match check.await?? {
            RateLimitOutcome::Allowed => allowed += 1,
            RateLimitOutcome::Denied { retry_after } => {
                denied += 1;
                assert!(!retry_after.is_zero());
                assert!(retry_after <= Duration::from_secs(1));
            }
        }
    }
    assert_eq!(allowed, 8);
    assert_eq!(denied, 8);

    let isolated_endpoint = RateLimitRequest {
        endpoint: "oauth_exchange",
        ..request.clone()
    };
    assert_eq!(
        first.check(&isolated_endpoint).await?,
        RateLimitOutcome::Allowed
    );
    let isolated_subject = format!("ip:task-5:{}", Uuid::new_v4());
    assert_eq!(
        second
            .check(&RateLimitRequest {
                subject: isolated_subject.clone(),
                ..request.clone()
            })
            .await?,
        RateLimitOutcome::Allowed
    );

    let mut connection = redis::Client::open(redis_url.as_str())?
        .get_multiplexed_async_connection()
        .await?;
    let keys = redis::cmd("KEYS")
        .arg("jamye:rate-limit:v1:*")
        .query_async::<Vec<String>>(&mut connection)
        .await?;
    assert!(keys.iter().all(|key| !key.contains(&subject)));
    assert!(keys.iter().all(|key| !key.contains(&isolated_subject)));

    tokio::time::sleep(Duration::from_millis(1_100)).await;
    assert_eq!(first.check(&request).await?, RateLimitOutcome::Allowed);
    Ok(())
}

#[tokio::test]
async fn oauth_attempt_uses_only_the_state_digest_and_getdel_is_one_time() -> TestResult {
    let redis_url = guarded_redis_url()?;
    let store = RedisOAuthAttemptStore::new(&redis_url)?;
    let credential = OsCredentialSource.generate()?;
    let raw_state = credential.raw.expose().to_owned();
    let attempt = OAuthAttempt {
        provider: ProviderKind::Kakao,
        redirect_uri: "jamye://oauth/kakao".to_owned(),
        code_challenge: "c".repeat(43),
        nonce: "n".repeat(43),
    };
    assert_eq!(
        store
            .create(&credential.digest, &attempt, Duration::from_secs(60))
            .await?,
        CreateAttemptOutcome::Created
    );

    let digest = encode_hex(credential.digest.as_bytes());
    let raw_key = format!("jamye:oauth-attempt:v1:{raw_state}");
    let digest_key = format!("jamye:oauth-attempt:v1:{digest}");
    let mut connection = redis::Client::open(redis_url.as_str())?
        .get_multiplexed_async_connection()
        .await?;
    let raw_key_exists = redis::cmd("EXISTS")
        .arg(raw_key)
        .query_async::<i64>(&mut connection)
        .await?;
    let digest_payload = redis::cmd("GET")
        .arg(digest_key)
        .query_async::<Option<String>>(&mut connection)
        .await?;
    assert_eq!(raw_key_exists, 0);
    let digest_payload = digest_payload
        .ok_or_else(|| io::Error::other("digest-keyed OAuth attempt was not stored"))?;
    assert!(!digest_payload.contains(&raw_state));

    let first = store.consume(&credential.digest);
    let second = store.consume(&credential.digest);
    let (first, second) = tokio::join!(first, second);
    let outcomes = [first?, second?];
    let consumed = match (&outcomes[0], &outcomes[1]) {
        (ConsumeAttemptOutcome::Found(found), ConsumeAttemptOutcome::Missing)
        | (ConsumeAttemptOutcome::Missing, ConsumeAttemptOutcome::Found(found)) => found,
        _ => {
            return Err(io::Error::other(
                "OAuth attempt was not consumed exactly once with GETDEL",
            )
            .into());
        }
    };
    assert_eq!(consumed, &attempt);
    Ok(())
}

#[tokio::test]
#[ignore = "the task-5 Redis recovery card coordinates the guarded container lifecycle"]
async fn redis_stop_restart_recovers_same_limiter_without_touching_refresh_authority() -> TestResult
{
    let coordination_dir = recovery_coordination_dir()?;
    let redis_url = guarded_redis_url()?;
    let limiter = RedisRateLimiter::new(&redis_url)?;
    let request = RateLimitRequest {
        endpoint: "auth_refresh",
        subject: format!("refresh:task-5-recovery:{}", Uuid::new_v4()),
        limit: 2,
        window: Duration::from_secs(60),
    };
    assert_eq!(limiter.check(&request).await?, RateLimitOutcome::Allowed);

    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let user_id = Uuid::new_v4();
    let session_id = Uuid::new_v4();
    let token_hash = vec![7_u8; 32];
    sqlx::query("INSERT INTO users (id, nickname) VALUES ($1, $2)")
        .bind(user_id)
        .bind("Redis recovery fixture")
        .execute(&pool)
        .await?;
    sqlx::query(
        "INSERT INTO auth_identities (id, user_id, provider, provider_id) \
         VALUES ($1, $2, 'kakao', $3)",
    )
    .bind(Uuid::new_v4())
    .bind(user_id)
    .bind(format!("recovery-{user_id}"))
    .execute(&pool)
    .await?;
    sqlx::query(
        "INSERT INTO refresh_sessions \
         (id, user_id, family_id, token_hash, expires_at) \
         VALUES ($1, $2, $3, $4, clock_timestamp() + interval '1 hour')",
    )
    .bind(session_id)
    .bind(user_id)
    .bind(Uuid::new_v4())
    .bind(&token_hash)
    .execute(&pool)
    .await?;

    write_marker(&coordination_dir, "ready-to-stop")?;
    wait_for_marker(&coordination_dir, "redis-stopped").await?;
    assert!(limiter.check(&request).await.is_err());
    assert_eq!(stored_hash(&pool, session_id).await?, token_hash);

    write_marker(&coordination_dir, "ready-to-start")?;
    wait_for_marker(&coordination_dir, "redis-started").await?;
    assert!(matches!(
        limiter.check(&request).await?,
        RateLimitOutcome::Allowed | RateLimitOutcome::Denied { .. }
    ));
    assert_eq!(stored_hash(&pool, session_id).await?, token_hash);

    pool.close().await;
    database.dispose().await
}

fn guarded_redis_url() -> TestResult<String> {
    if env::var("JAMYE_ENVIRONMENT").as_deref() != Ok("test") {
        return Err(
            io::Error::other("Redis integration tests require JAMYE_ENVIRONMENT=test").into(),
        );
    }
    let redis_url = env::var("REDIS_URL")
        .map_err(|_| io::Error::other("REDIS_URL is required for Redis integration tests"))?;
    let parsed = Url::parse(&redis_url)?;
    if parsed.scheme() != "redis"
        || !matches!(parsed.host_str(), Some("127.0.0.1" | "localhost" | "::1"))
    {
        return Err(io::Error::other(
            "Redis integration tests accept only a loopback redis:// URL",
        )
        .into());
    }
    Ok(redis_url)
}

fn recovery_coordination_dir() -> TestResult<PathBuf> {
    let path = env::var("JAMYE_TASK5_RECOVERY_COORD_DIR").map_err(|_| {
        io::Error::other("JAMYE_TASK5_RECOVERY_COORD_DIR is required for the ignored recovery test")
    })?;
    let path = PathBuf::from(path);
    if !path.is_absolute() || !path.is_dir() {
        return Err(io::Error::other(
            "task-5 recovery coordination path must be an existing absolute directory",
        )
        .into());
    }
    Ok(path)
}

fn write_marker(directory: &Path, name: &str) -> TestResult {
    fs::write(directory.join(name), b"ready\n")?;
    Ok(())
}

async fn wait_for_marker(directory: &Path, name: &str) -> TestResult {
    let marker = directory.join(name);
    for _ in 0..600 {
        if marker.is_file() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err(io::Error::other(format!(
        "timed out waiting for recovery marker {}",
        marker.display()
    ))
    .into())
}

async fn stored_hash(pool: &sqlx::PgPool, session_id: Uuid) -> TestResult<Vec<u8>> {
    Ok(
        sqlx::query_scalar("SELECT token_hash FROM refresh_sessions WHERE id = $1")
            .bind(session_id)
            .fetch_one(pool)
            .await?,
    )
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
