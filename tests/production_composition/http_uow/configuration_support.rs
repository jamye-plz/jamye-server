fn auth_config() -> TestResult<AuthConfig> {
    let config = AuthConfig::try_from(AuthConfigInput {
        kakao_enabled: Some("true".to_owned()),
        kakao_client_id: Some("task-12-kakao-client".to_owned()),
        kakao_client_secret: Some("task-12-kakao-secret".to_owned()),
        kakao_redirect_uris: Some("https://app.example.test/oauth/kakao".to_owned()),
        google_enabled: Some("true".to_owned()),
        google_client_id: Some("task-12-google-client".to_owned()),
        google_client_secret: Some("task-12-google-secret".to_owned()),
        google_redirect_uris: Some("https://app.example.test/oauth/google".to_owned()),
        provider_timeout_ms: Some("500".to_owned()),
        access_token_secret: Some(SECRET.to_owned()),
        access_token_issuer: Some(ISSUER.to_owned()),
        access_token_audience: Some(AUDIENCE.to_owned()),
        access_token_ttl_seconds: Some("900".to_owned()),
        refresh_token_ttl_seconds: Some("3600".to_owned()),
    })?;
    Ok(config)
}
fn production_router(config: &AppConfig, auth: &AuthConfig) -> TestResult<Router> {
    let rate_limits = RateLimitConfig::default();
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
        io::Error::other("Task-12 test environment uses configured object storage")
    })?;
    let router = composition::router_with_runtime(config, auth, &rate_limits, Some(&storage))?;
    Ok(router)
}
fn token(user: Uuid) -> Result<String, ProductionTokenConfigError> {
    let codec = ProductionTokenCodec::new(SECRET.as_bytes(), ISSUER, AUDIENCE)?;
    codec
        .issue(
            user,
            Uuid::new_v4(),
            OffsetDateTime::now_utc(),
            OffsetDateTime::now_utc() + Duration::minutes(5),
        )
        .map_err(|_| ProductionTokenConfigError)
}
fn require(condition: bool, message: &str) -> TestResult {
    if condition {
        Ok(())
    } else {
        Err(message.to_owned().into())
    }
}
