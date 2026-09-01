fn worker_config() -> TestResult<AppConfig> {
    let config = AppConfig::try_from(ConfigInput {
        redis_url: Some("redis://127.0.0.1/".to_owned()),
        ..config_input("postgres://127.0.0.1/jamye_test")
    })?;
    Ok(config)
}
fn app_config(database_url: &str) -> TestResult<AppConfig> {
    let config = AppConfig::try_from(config_input(database_url))?;
    Ok(config)
}

fn config_input(database_url: &str) -> ConfigInput {
    ConfigInput {
        environment: Some("test".to_owned()),
        readiness_timeout_ms: Some("50".to_owned()),
        database_url: Some(database_url.to_owned()),
        redis_url: Some("redis://127.0.0.1/".to_owned()),
        ..ConfigInput::default()
    }
}

fn validated_auth_config() -> TestResult<AuthConfig> {
    let config = AuthConfig::try_from(auth_config_input())?;
    Ok(config)
}

fn auth_config_input() -> AuthConfigInput {
    AuthConfigInput {
        kakao_enabled: Some("true".to_owned()),
        kakao_client_id: Some("task-12-kakao-client".to_owned()),
        kakao_client_secret: Some("task-12-kakao-secret".to_owned()),
        kakao_redirect_uris: Some("https://app.example.test/oauth/kakao".to_owned()),
        google_enabled: Some("true".to_owned()),
        google_client_id: Some("task-12-google-client".to_owned()),
        google_client_secret: Some("task-12-google-secret".to_owned()),
        google_redirect_uris: Some("https://app.example.test/oauth/google".to_owned()),
        provider_timeout_ms: Some("500".to_owned()),
        access_token_secret: Some(AUTH_SECRET.to_owned()),
        access_token_issuer: Some("jamye-task-12-test".to_owned()),
        access_token_audience: Some("jamye-task-12-client".to_owned()),
        access_token_ttl_seconds: Some("900".to_owned()),
        refresh_token_ttl_seconds: Some("3600".to_owned()),
    }
}

fn push_config() -> TestResult<PushConfig> {
    let config = PushConfig::resolve(AppEnvironment::Test, PushConfigInput::default())?;
    Ok(config)
}

fn production_router(config: &AppConfig, auth: &AuthConfig) -> TestResult<axum::Router> {
    let rate_limits = RateLimitConfig::default();
    let storage = object_storage_config()?;
    let router = composition::router_with_runtime(config, auth, &rate_limits, Some(&storage))?;
    Ok(router)
}

fn object_storage_config() -> TestResult<ObjectStorageConfig> {
    let storage = ObjectStorageConfig::resolve(
        AppEnvironment::Test,
        ObjectStorageConfigInput {
            endpoint: Some("http://127.0.0.1:9000".to_owned()),
            public_endpoint: Some("http://127.0.0.1:9000".to_owned()),
            region: Some("us-east-1".to_owned()),
            bucket: Some("jamye-private-bucket".to_owned()),
            access_key_id: Some("task-12-media-access-key".to_owned()),
            secret_access_key: Some("task-12-media-secret".to_owned()),
        },
    )?;
    let storage = storage.ok_or_else(|| {
        std::io::Error::other("Task-12 test environment uses configured object storage")
    })?;
    Ok(storage)
}

fn cleanup_config() -> TestResult<AccountDeletionConfig> {
    let config = AccountDeletionConfig::resolve(AccountDeletionConfigInput {
        access_key_id: Some("task-12-cleanup-access-key".to_owned()),
        secret_access_key: Some("task-12-cleanup-secret".to_owned()),
        ..AccountDeletionConfigInput::default()
    })?;
    Ok(config)
}
