use std::time::Duration;

use jamye_server::{
    adapters::push::expo::EXPO_PUSH_SEND_URL,
    config::{
        AppEnvironment, ConfigError,
        push::{PushConfig, PushConfigInput},
    },
};

use super::TestResult;

const ACCESS_TOKEN: &str = "TASK_9_EXPO_ACCESS_TOKEN_SENTINEL";

#[test]
fn defaults_and_overrides_enforce_budget_and_redact_access_token() -> TestResult {
    let defaults = PushConfig::resolve(AppEnvironment::Test, PushConfigInput::default())?;
    assert_eq!(defaults.endpoint(), EXPO_PUSH_SEND_URL);
    assert_eq!(defaults.batch_size(), 50);
    assert_eq!(defaults.lease_duration(), Duration::from_secs(15));
    assert_eq!(defaults.provider_timeout(), Duration::from_secs(2));
    assert_eq!(defaults.lease_safety_margin(), Duration::from_secs(1));
    assert_eq!(defaults.retry_delay(), Duration::from_secs(1));
    assert_eq!(defaults.poll_interval(), Duration::from_millis(250));
    assert_eq!(defaults.max_attempts(), 8);

    let configured = PushConfig::resolve(
        AppEnvironment::Test,
        PushConfigInput {
            endpoint: Some("http://127.0.0.1:41009/--/api/v2/push/send".to_owned()),
            access_token: Some(ACCESS_TOKEN.to_owned()),
            batch_size: Some("7".to_owned()),
            lease_ms: Some("9000".to_owned()),
            provider_timeout_ms: Some("2500".to_owned()),
            lease_safety_margin_ms: Some("500".to_owned()),
            retry_delay_ms: Some("750".to_owned()),
            poll_interval_ms: Some("125".to_owned()),
            max_attempts: Some("4".to_owned()),
        },
    )?;
    assert_eq!(configured.batch_size(), 7);
    assert_eq!(configured.lease_duration(), Duration::from_millis(9000));
    assert_eq!(configured.provider_timeout(), Duration::from_millis(2500));
    assert_eq!(configured.lease_safety_margin(), Duration::from_millis(500));
    assert_eq!(configured.retry_delay(), Duration::from_millis(750));
    assert_eq!(configured.poll_interval(), Duration::from_millis(125));
    assert_eq!(configured.max_attempts(), 4);
    let debug = format!("{configured:?}");
    assert!(debug.contains("[REDACTED]"));
    assert!(!debug.contains(ACCESS_TOKEN));
    Ok(())
}

#[test]
fn invalid_values_name_only_the_rejected_key() {
    let invalid_cases = [
        (
            PushConfigInput {
                endpoint: Some("http://192.168.0.8/--/api/v2/push/send".to_owned()),
                ..PushConfigInput::default()
            },
            "JAMYE_EXPO_PUSH_SEND_URL",
        ),
        (
            PushConfigInput {
                access_token: Some("line\nbreak".to_owned()),
                ..PushConfigInput::default()
            },
            "JAMYE_EXPO_ACCESS_TOKEN",
        ),
        (
            PushConfigInput {
                batch_size: Some("0".to_owned()),
                ..PushConfigInput::default()
            },
            "JAMYE_PUSH_BATCH_SIZE",
        ),
        (
            PushConfigInput {
                lease_ms: Some("3000".to_owned()),
                provider_timeout_ms: Some("2000".to_owned()),
                lease_safety_margin_ms: Some("1000".to_owned()),
                ..PushConfigInput::default()
            },
            "JAMYE_PUSH_LEASE_MS",
        ),
        (
            PushConfigInput {
                provider_timeout_ms: Some("not-a-number".to_owned()),
                ..PushConfigInput::default()
            },
            "JAMYE_PUSH_PROVIDER_TIMEOUT_MS",
        ),
        (
            PushConfigInput {
                retry_delay_ms: Some("0".to_owned()),
                ..PushConfigInput::default()
            },
            "JAMYE_PUSH_RETRY_DELAY_MS",
        ),
        (
            PushConfigInput {
                poll_interval_ms: Some("60001".to_owned()),
                ..PushConfigInput::default()
            },
            "JAMYE_PUSH_POLL_INTERVAL_MS",
        ),
        (
            PushConfigInput {
                max_attempts: Some("0".to_owned()),
                ..PushConfigInput::default()
            },
            "JAMYE_PUSH_MAX_ATTEMPTS",
        ),
    ];

    for (input, expected_key) in invalid_cases {
        let result = PushConfig::resolve(AppEnvironment::Test, input);
        assert_eq!(
            result.as_ref().err().map(ConfigError::key),
            Some(expected_key)
        );
        let rendered = result
            .as_ref()
            .err()
            .map(ToString::to_string)
            .unwrap_or_default();
        assert!(!rendered.contains(ACCESS_TOKEN));
        assert!(!rendered.contains("192.168.0.8"));
    }

    let production_loopback = PushConfig::resolve(
        AppEnvironment::Production,
        PushConfigInput {
            endpoint: Some("http://127.0.0.1:41009/--/api/v2/push/send".to_owned()),
            ..PushConfigInput::default()
        },
    );
    assert_eq!(
        production_loopback.as_ref().err().map(ConfigError::key),
        Some("JAMYE_EXPO_PUSH_SEND_URL")
    );
}
