use std::{env, io, time::Duration};

use jamye_server::{
    adapters::redis::rate_limit::RedisRateLimiter,
    config::rate_limit::{RateLimitConfig, RateLimitConfigInput},
    ports::rate_limit::{RateLimitOutcome, RateLimitRequest, RateLimiter},
};
use url::Url;
use uuid::Uuid;

use crate::TestResult;

#[test]
fn invite_rate_limit_defaults_are_conservative_configurable_and_validated() -> TestResult {
    let defaults = RateLimitConfig::default();
    assert_eq!(defaults.groups.invite_issue.limit, 10);
    assert_eq!(defaults.groups.invite_issue.window, Duration::from_secs(60));
    assert_eq!(defaults.groups.invite_redeem.limit, 20);
    assert_eq!(
        defaults.groups.invite_redeem.window,
        Duration::from_secs(60)
    );

    let configured = RateLimitConfig::try_from(RateLimitConfigInput {
        invite_issue_limit: Some("3".to_owned()),
        invite_issue_window_seconds: Some("30".to_owned()),
        invite_redeem_limit: Some("7".to_owned()),
        invite_redeem_window_seconds: Some("90".to_owned()),
        ..RateLimitConfigInput::default()
    })?;
    assert_eq!(configured.groups.invite_issue.limit, 3);
    assert_eq!(
        configured.groups.invite_issue.window,
        Duration::from_secs(30)
    );
    assert_eq!(configured.groups.invite_redeem.limit, 7);
    assert_eq!(
        configured.groups.invite_redeem.window,
        Duration::from_secs(90)
    );

    let invalid = RateLimitConfig::try_from(RateLimitConfigInput {
        invite_redeem_limit: Some("0".to_owned()),
        ..RateLimitConfigInput::default()
    });
    assert_eq!(
        invalid.map_err(|error| error.key()),
        Err("JAMYE_RATE_LIMIT_INVITE_REDEEM_LIMIT")
    );
    Ok(())
}

#[tokio::test]
async fn invite_limits_share_the_versioned_redis_namespace_reset_and_isolate_subjects() -> TestResult
{
    let redis_url = guarded_redis_url()?;
    let limiter = RedisRateLimiter::new(&redis_url)?;
    let subject = format!("user:{}:ip:127.0.0.1", Uuid::new_v4());
    let issue = RateLimitRequest {
        endpoint: "invite_issue",
        subject: subject.clone(),
        limit: 1,
        window: Duration::from_secs(1),
    };
    assert!(!format!("{issue:?}").contains(&subject));
    assert_eq!(limiter.check(&issue).await?, RateLimitOutcome::Allowed);
    assert!(matches!(
        limiter.check(&issue).await?,
        RateLimitOutcome::Denied { retry_after }
            if !retry_after.is_zero() && retry_after <= Duration::from_secs(1)
    ));

    let isolated_subject = RateLimitRequest {
        subject: format!("user:{}:ip:127.0.0.2", Uuid::new_v4()),
        ..issue.clone()
    };
    assert_eq!(
        limiter.check(&isolated_subject).await?,
        RateLimitOutcome::Allowed
    );
    let isolated_endpoint = RateLimitRequest {
        endpoint: "invite_redeem",
        ..issue.clone()
    };
    assert_eq!(
        limiter.check(&isolated_endpoint).await?,
        RateLimitOutcome::Allowed
    );

    let mut connection = redis::Client::open(redis_url.as_str())?
        .get_multiplexed_async_connection()
        .await?;
    let keys = redis::cmd("KEYS")
        .arg("jamye:rate-limit:v1:invite_*")
        .query_async::<Vec<String>>(&mut connection)
        .await?;
    assert!(keys.iter().any(|key| key.contains("invite_issue")));
    assert!(keys.iter().any(|key| key.contains("invite_redeem")));
    assert!(keys.iter().all(|key| !key.contains(&subject)));

    tokio::time::sleep(Duration::from_millis(1_100)).await;
    assert_eq!(limiter.check(&issue).await?, RateLimitOutcome::Allowed);
    Ok(())
}

pub fn guarded_redis_url() -> TestResult<String> {
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
