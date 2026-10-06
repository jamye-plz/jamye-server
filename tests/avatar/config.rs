use std::time::Duration;

use jamye_server::{
    application::avatar::AvatarEndpointRateLimit,
    config::{
        AppConfig, ConfigInput,
        rate_limit::{RateLimitConfig, RateLimitConfigInput},
    },
    domain::{
        avatar::{avatar_public_url, mint_avatar_object_key},
        profile::valid_avatar_url,
    },
};
use uuid::Uuid;

use crate::TestResult;

const KEY: &str = "JAMYE_AVATAR_PUBLIC_BASE_URL";

fn config_with(
    avatar_public_base_url: Option<&str>,
) -> Result<AppConfig, jamye_server::config::ConfigError> {
    AppConfig::try_from(ConfigInput {
        environment: Some("test".to_owned()),
        database_url: Some("postgres://127.0.0.1/jamye_test".to_owned()),
        avatar_public_base_url: avatar_public_base_url.map(ToOwned::to_owned),
        ..ConfigInput::default()
    })
}

#[test]
fn the_public_base_url_is_optional_and_gates_the_avatar_routes() -> TestResult {
    assert_eq!(config_with(None)?.avatar_public_base_url(), None);
    for blank in ["", "   "] {
        assert_eq!(config_with(Some(blank))?.avatar_public_base_url(), None);
    }
    for (input, expected) in [
        (
            "https://avatars.example.test",
            "https://avatars.example.test",
        ),
        (
            "https://avatars.example.test/",
            "https://avatars.example.test",
        ),
        (
            "https://Avatars.Example.test:8443",
            "https://avatars.example.test:8443",
        ),
    ] {
        assert_eq!(
            config_with(Some(input))?.avatar_public_base_url(),
            Some(expected),
            "unexpected normalization for {input}"
        );
    }
    Ok(())
}

#[test]
fn an_invalid_public_base_url_is_a_startup_failure_naming_only_the_key() -> TestResult {
    let too_long = format!("https://{}.example.test", "a".repeat(250));
    for invalid in [
        "http://avatars.example.test",
        "ftp://avatars.example.test",
        "avatars.example.test",
        "not a url",
        "https://avatars.example.test/avatars",
        "https://avatars.example.test/?a=1",
        "https://avatars.example.test/#fragment",
        "https://user:secret@avatars.example.test",
        too_long.as_str(),
    ] {
        let error = match config_with(Some(invalid)) {
            Ok(_) => return Err(format!("invalid base URL was accepted: {invalid}").into()),
            Err(error) => error,
        };
        assert_eq!(error.key(), KEY);
        assert!(
            !error.to_string().contains("secret"),
            "configuration error echoed the value"
        );
    }
    Ok(())
}

#[test]
fn a_base_of_the_maximum_length_still_yields_a_valid_avatar_url() -> TestResult {
    let label = "a".repeat(63);
    let host = format!("{label}.{label}.{label}.{}", "b".repeat(50));
    let base = format!("https://{host}");
    assert!(base.chars().count() <= 256);
    let config = config_with(Some(&base))?;
    let configured = config
        .avatar_public_base_url()
        .ok_or("configured base URL was dropped")?;
    let url = avatar_public_url(configured, Uuid::new_v4());
    assert!(url.chars().count() <= 512);
    assert!(
        valid_avatar_url(&url),
        "hosted avatar URL failed validation"
    );
    assert!(url.starts_with(configured));
    Ok(())
}

#[test]
fn the_object_key_is_private_and_the_public_url_omits_the_user_id() {
    let user_id = Uuid::new_v4();
    let upload_id = Uuid::new_v4();
    assert_eq!(
        mint_avatar_object_key(user_id, upload_id),
        format!("avatar/{user_id}/{upload_id}")
    );
    let url = avatar_public_url("https://avatars.example.test", upload_id);
    assert_eq!(
        url,
        format!("https://avatars.example.test/api/v1/avatars/{upload_id}")
    );
    assert!(!url.contains(&user_id.to_string()));
}

#[test]
fn the_public_read_limit_defaults_to_600_per_minute_and_is_configurable() -> TestResult {
    let defaults = RateLimitConfig::default();
    assert_eq!(
        defaults.avatar_public_read,
        AvatarEndpointRateLimit {
            limit: 600,
            window: Duration::from_secs(60),
        }
    );
    let from_empty = RateLimitConfig::try_from(RateLimitConfigInput::default())?;
    assert_eq!(from_empty, defaults);

    let configured = RateLimitConfig::try_from(RateLimitConfigInput {
        avatar_public_read_limit: Some("25".to_owned()),
        avatar_public_read_window_seconds: Some("30".to_owned()),
        ..RateLimitConfigInput::default()
    })?;
    assert_eq!(
        configured.avatar_public_read,
        AvatarEndpointRateLimit {
            limit: 25,
            window: Duration::from_secs(30),
        }
    );
    // U4 reuses the media presign values and gains no setting of its own.
    assert_eq!(
        configured.media_upload_presign,
        defaults.media_upload_presign
    );

    for (limit, window, key) in [
        (Some("0"), None, "JAMYE_RATE_LIMIT_AVATAR_PUBLIC_READ_LIMIT"),
        (
            Some("10001"),
            None,
            "JAMYE_RATE_LIMIT_AVATAR_PUBLIC_READ_LIMIT",
        ),
        (
            None,
            Some("0"),
            "JAMYE_RATE_LIMIT_AVATAR_PUBLIC_READ_WINDOW_SECONDS",
        ),
        (
            None,
            Some("not-a-number"),
            "JAMYE_RATE_LIMIT_AVATAR_PUBLIC_READ_WINDOW_SECONDS",
        ),
    ] {
        let error = match RateLimitConfig::try_from(RateLimitConfigInput {
            avatar_public_read_limit: limit.map(ToOwned::to_owned),
            avatar_public_read_window_seconds: window.map(ToOwned::to_owned),
            ..RateLimitConfigInput::default()
        }) {
            Ok(_) => return Err(format!("invalid limit input was accepted: {key}").into()),
            Err(error) => error,
        };
        assert_eq!(error.key(), key);
    }
    Ok(())
}
