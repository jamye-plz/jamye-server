use std::io;

use jamye_server::config::{
    AppEnvironment,
    object_storage::{ObjectStorageConfig, ObjectStorageConfigInput},
};

use crate::TestResult;

const ACCESS_KEY: &str = "task-8-access-key";
const SECRET_KEY: &str = "task-8-secret-key";

#[test]
fn production_accepts_internal_loopback_but_requires_external_https_presigning() -> TestResult {
    let config = ObjectStorageConfig::resolve(
        AppEnvironment::Production,
        complete_input("https://media.example.com"),
    )?
    .ok_or_else(|| io::Error::other("configured production storage is unexpectedly absent"))?;

    assert_eq!(config.endpoint(), "http://127.0.0.1:9000/");
    assert_eq!(config.public_endpoint(), "https://media.example.com/");
    assert_eq!(config.region(), "us-east-1");
    assert_eq!(config.bucket(), "jamye-private-media");
    let debug = format!("{config:?}");
    assert!(!debug.contains(ACCESS_KEY));
    assert!(!debug.contains(SECRET_KEY));
    assert!(debug.contains("[REDACTED]"));
    Ok(())
}

#[test]
fn production_requires_every_object_storage_value() {
    let required_keys = [
        "JAMYE_OBJECT_STORAGE_ENDPOINT",
        "JAMYE_OBJECT_STORAGE_PUBLIC_ENDPOINT",
        "JAMYE_OBJECT_STORAGE_REGION",
        "JAMYE_OBJECT_STORAGE_BUCKET",
        "JAMYE_OBJECT_STORAGE_ACCESS_KEY_ID",
        "JAMYE_OBJECT_STORAGE_SECRET_ACCESS_KEY",
    ];

    for key in required_keys {
        let mut input = complete_input("https://media.example.com");
        clear(&mut input, key);
        let error = ObjectStorageConfig::resolve(AppEnvironment::Production, input).err();
        assert_eq!(error.as_ref().map(|error| error.key()), Some(key));
    }
}

#[test]
fn production_rejects_unsafe_public_presign_endpoints() {
    for public_endpoint in [
        "http://media.example.com",
        "https://127.0.0.1:9000",
        "https://localhost:9000",
        "https://minio:9000",
        "https://host.containers.internal:9000",
    ] {
        let error = ObjectStorageConfig::resolve(
            AppEnvironment::Production,
            complete_input(public_endpoint),
        )
        .err();
        assert_eq!(
            error.as_ref().map(|error| error.key()),
            Some("JAMYE_OBJECT_STORAGE_PUBLIC_ENDPOINT")
        );
    }
}

#[test]
fn test_mode_accepts_explicit_loopback_http_and_redacts_credentials() -> TestResult {
    let config = ObjectStorageConfig::resolve(
        AppEnvironment::Test,
        complete_input("http://127.0.0.1:9000"),
    )?
    .ok_or_else(|| io::Error::other("configured test storage is unexpectedly absent"))?;

    assert_eq!(config.public_endpoint(), "http://127.0.0.1:9000/");
    let debug = format!("{config:?}");
    assert!(!debug.contains(ACCESS_KEY));
    assert!(!debug.contains(SECRET_KEY));
    assert!(debug.contains("[REDACTED]"));
    Ok(())
}

#[test]
fn nonproduction_all_absent_is_storage_degraded_without_process_failure() -> TestResult {
    assert!(
        ObjectStorageConfig::resolve(
            AppEnvironment::Development,
            ObjectStorageConfigInput::default()
        )?
        .is_none()
    );
    Ok(())
}

#[test]
fn partial_nonproduction_configuration_is_rejected() {
    let input = ObjectStorageConfigInput {
        endpoint: Some("http://127.0.0.1:9000".to_owned()),
        ..ObjectStorageConfigInput::default()
    };
    let error = ObjectStorageConfig::resolve(AppEnvironment::Test, input).err();

    assert_eq!(
        error.as_ref().map(|error| error.key()),
        Some("JAMYE_OBJECT_STORAGE_PUBLIC_ENDPOINT")
    );
}

#[test]
fn invalid_bucket_name_is_rejected_without_echoing_it() {
    let mut input = complete_input("https://media.example.com");
    let invalid_bucket = "Private_Bucket_With_Uppercase";
    input.bucket = Some(invalid_bucket.to_owned());
    let error = ObjectStorageConfig::resolve(AppEnvironment::Production, input).err();

    assert_eq!(
        error.as_ref().map(|error| error.key()),
        Some("JAMYE_OBJECT_STORAGE_BUCKET")
    );
    assert!(!format!("{error:?}").contains(invalid_bucket));
}

fn complete_input(public_endpoint: &str) -> ObjectStorageConfigInput {
    ObjectStorageConfigInput {
        endpoint: Some("http://127.0.0.1:9000".to_owned()),
        public_endpoint: Some(public_endpoint.to_owned()),
        region: Some("us-east-1".to_owned()),
        bucket: Some("jamye-private-media".to_owned()),
        access_key_id: Some(ACCESS_KEY.to_owned()),
        secret_access_key: Some(SECRET_KEY.to_owned()),
    }
}

fn clear(input: &mut ObjectStorageConfigInput, key: &str) {
    match key {
        "JAMYE_OBJECT_STORAGE_ENDPOINT" => input.endpoint = None,
        "JAMYE_OBJECT_STORAGE_PUBLIC_ENDPOINT" => input.public_endpoint = None,
        "JAMYE_OBJECT_STORAGE_REGION" => input.region = None,
        "JAMYE_OBJECT_STORAGE_BUCKET" => input.bucket = None,
        "JAMYE_OBJECT_STORAGE_ACCESS_KEY_ID" => input.access_key_id = None,
        "JAMYE_OBJECT_STORAGE_SECRET_ACCESS_KEY" => input.secret_access_key = None,
        _ => {}
    }
}
