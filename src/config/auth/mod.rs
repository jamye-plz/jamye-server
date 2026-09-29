//! Validated production OAuth and token configuration.

use std::{env, fmt, time::Duration};

use base64::{Engine, engine::general_purpose::STANDARD};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use serde::Serialize;
use url::Url;

pub use crate::application::auth::OAUTH_ATTEMPT_TTL;
use crate::ports::oauth_provider::ProviderKind;

pub const APPLE_ISSUER: &str = "https://appleid.apple.com";
pub const APPLE_JWKS_URL: &str = "https://appleid.apple.com/auth/keys";
pub const APPLE_TOKEN_URL: &str = "https://appleid.apple.com/auth/token";
pub const APPLE_REVOKE_URL: &str = "https://appleid.apple.com/auth/revoke";

#[derive(Clone, Default)]
pub struct AuthConfigInput {
    pub kakao_enabled: Option<String>,
    pub kakao_client_id: Option<String>,
    pub kakao_client_secret: Option<String>,
    pub kakao_redirect_uris: Option<String>,
    pub google_enabled: Option<String>,
    pub google_client_id: Option<String>,
    pub google_client_secret: Option<String>,
    pub google_redirect_uris: Option<String>,
    pub apple_signin_enabled: Option<String>,
    pub apple_audiences: Option<String>,
    pub apple_team_id: Option<String>,
    pub apple_key_id: Option<String>,
    pub apple_private_key: Option<String>,
    pub provider_timeout_ms: Option<String>,
    pub access_token_secret: Option<String>,
    pub access_token_issuer: Option<String>,
    pub access_token_audience: Option<String>,
    pub access_token_ttl_seconds: Option<String>,
    pub refresh_token_ttl_seconds: Option<String>,
}

impl AuthConfigInput {
    pub fn from_env() -> Self {
        Self {
            kakao_enabled: read("JAMYE_KAKAO_OAUTH_ENABLED"),
            kakao_client_id: read("JAMYE_KAKAO_CLIENT_ID"),
            kakao_client_secret: read("JAMYE_KAKAO_CLIENT_SECRET"),
            kakao_redirect_uris: read("JAMYE_KAKAO_REDIRECT_URIS"),
            google_enabled: read("JAMYE_GOOGLE_OAUTH_ENABLED"),
            google_client_id: read("JAMYE_GOOGLE_CLIENT_ID"),
            google_client_secret: read("JAMYE_GOOGLE_CLIENT_SECRET"),
            google_redirect_uris: read("JAMYE_GOOGLE_REDIRECT_URIS"),
            apple_signin_enabled: read("JAMYE_APPLE_SIGNIN_ENABLED"),
            apple_audiences: read("JAMYE_APPLE_AUDIENCES"),
            apple_team_id: read("JAMYE_APPLE_TEAM_ID"),
            apple_key_id: read("JAMYE_APPLE_KEY_ID"),
            apple_private_key: read("JAMYE_APPLE_PRIVATE_KEY"),
            provider_timeout_ms: read("JAMYE_OAUTH_PROVIDER_TIMEOUT_MS"),
            access_token_secret: read("JAMYE_ACCESS_TOKEN_SECRET"),
            access_token_issuer: read("JAMYE_ACCESS_TOKEN_ISSUER"),
            access_token_audience: read("JAMYE_ACCESS_TOKEN_AUDIENCE"),
            access_token_ttl_seconds: read("JAMYE_ACCESS_TOKEN_TTL_SECONDS"),
            refresh_token_ttl_seconds: read("JAMYE_REFRESH_TOKEN_TTL_SECONDS"),
        }
    }
}

#[derive(Clone)]
pub struct AuthConfig {
    pub kakao: ProviderConfig,
    pub google: ProviderConfig,
    pub apple: AppleConfig,
    pub provider_timeout: Duration,
    pub access_token_secret: SensitiveValue,
    pub access_token_issuer: String,
    pub access_token_audience: String,
    pub access_token_ttl: Duration,
    pub refresh_token_ttl: Duration,
}

impl AuthConfig {
    pub fn from_env() -> Result<Self, AuthConfigError> {
        Self::try_from(AuthConfigInput::from_env())
    }
}

impl TryFrom<AuthConfigInput> for AuthConfig {
    type Error = AuthConfigError;

    fn try_from(input: AuthConfigInput) -> Result<Self, Self::Error> {
        let kakao = provider_config(
            ProviderKind::Kakao,
            "JAMYE_KAKAO_OAUTH_ENABLED",
            input.kakao_enabled,
            "JAMYE_KAKAO_CLIENT_ID",
            input.kakao_client_id,
            "JAMYE_KAKAO_CLIENT_SECRET",
            input.kakao_client_secret,
            "JAMYE_KAKAO_REDIRECT_URIS",
            input.kakao_redirect_uris,
        )?;
        let google = provider_config(
            ProviderKind::Google,
            "JAMYE_GOOGLE_OAUTH_ENABLED",
            input.google_enabled,
            "JAMYE_GOOGLE_CLIENT_ID",
            input.google_client_id,
            "JAMYE_GOOGLE_CLIENT_SECRET",
            input.google_client_secret,
            "JAMYE_GOOGLE_REDIRECT_URIS",
            input.google_redirect_uris,
        )?;
        let apple = apple_config(
            input.apple_signin_enabled,
            input.apple_audiences,
            input.apple_team_id,
            input.apple_key_id,
            input.apple_private_key,
        )?;
        let provider_timeout = duration(
            "JAMYE_OAUTH_PROVIDER_TIMEOUT_MS",
            input.provider_timeout_ms.as_deref().unwrap_or("5000"),
            100,
            30_000,
            Duration::from_millis,
        )?;
        let access_token_secret = required("JAMYE_ACCESS_TOKEN_SECRET", input.access_token_secret)?;
        if access_token_secret.len() < 32 {
            return Err(AuthConfigError::new(
                "JAMYE_ACCESS_TOKEN_SECRET",
                "must contain at least 32 bytes",
            ));
        }
        let access_token_issuer = required("JAMYE_ACCESS_TOKEN_ISSUER", input.access_token_issuer)?;
        if access_token_issuer == "jamye-dev" {
            return Err(AuthConfigError::new(
                "JAMYE_ACCESS_TOKEN_ISSUER",
                "must not use the development issuer",
            ));
        }
        let access_token_audience =
            required("JAMYE_ACCESS_TOKEN_AUDIENCE", input.access_token_audience)?;
        let access_token_ttl = duration(
            "JAMYE_ACCESS_TOKEN_TTL_SECONDS",
            input.access_token_ttl_seconds.as_deref().unwrap_or("900"),
            60,
            3_600,
            Duration::from_secs,
        )?;
        let refresh_token_ttl = duration(
            "JAMYE_REFRESH_TOKEN_TTL_SECONDS",
            input
                .refresh_token_ttl_seconds
                .as_deref()
                .unwrap_or("2592000"),
            3_600,
            7_776_000,
            Duration::from_secs,
        )?;
        if refresh_token_ttl <= access_token_ttl {
            return Err(AuthConfigError::new(
                "JAMYE_REFRESH_TOKEN_TTL_SECONDS",
                "must exceed the access-token lifetime",
            ));
        }
        Ok(Self {
            kakao,
            google,
            apple,
            provider_timeout,
            access_token_secret: SensitiveValue(access_token_secret),
            access_token_issuer,
            access_token_audience,
            access_token_ttl,
            refresh_token_ttl,
        })
    }
}

#[derive(Clone)]
pub struct AppleConfig {
    pub enabled: bool,
    pub audiences: Vec<String>,
    pub team_id: Option<SensitiveValue>,
    pub key_id: Option<SensitiveValue>,
    pub private_key_der: Option<SensitiveBytes>,
}

#[derive(Clone)]
pub struct ProviderConfig {
    pub kind: ProviderKind,
    pub enabled: bool,
    pub client_id: Option<SensitiveValue>,
    pub client_secret: Option<SensitiveValue>,
    pub redirect_uris: Vec<String>,
}

#[derive(Clone)]
pub struct SensitiveValue(String);

impl SensitiveValue {
    pub(crate) fn expose_secret(&self) -> &str {
        &self.0
    }
}

#[derive(Clone)]
pub struct SensitiveBytes(Vec<u8>);

impl SensitiveBytes {
    pub(crate) fn expose_secret(&self) -> &[u8] {
        &self.0
    }
}

impl fmt::Debug for SensitiveBytes {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[REDACTED]")
    }
}

impl fmt::Debug for SensitiveValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[REDACTED]")
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthConfigError {
    key: &'static str,
    reason: &'static str,
}

impl AuthConfigError {
    fn new(key: &'static str, reason: &'static str) -> Self {
        Self { key, reason }
    }

    pub fn key(&self) -> &'static str {
        self.key
    }
}

impl fmt::Display for AuthConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "invalid configuration for {}: {}",
            self.key, self.reason
        )
    }
}

impl std::error::Error for AuthConfigError {}

#[allow(clippy::too_many_arguments)]
fn provider_config(
    kind: ProviderKind,
    enabled_key: &'static str,
    enabled: Option<String>,
    client_id_key: &'static str,
    client_id: Option<String>,
    client_secret_key: &'static str,
    client_secret: Option<String>,
    redirect_key: &'static str,
    redirect_uris: Option<String>,
) -> Result<ProviderConfig, AuthConfigError> {
    let enabled = boolean(enabled_key, enabled.as_deref().unwrap_or("false"))?;
    if !enabled {
        return Ok(ProviderConfig {
            kind,
            enabled,
            client_id: None,
            client_secret: None,
            redirect_uris: Vec::new(),
        });
    }
    let client_id = SensitiveValue(required(client_id_key, client_id)?);
    let client_secret = SensitiveValue(required(client_secret_key, client_secret)?);
    let redirect_uris = parse_redirects(redirect_key, redirect_uris)?;
    Ok(ProviderConfig {
        kind,
        enabled,
        client_id: Some(client_id),
        client_secret: Some(client_secret),
        redirect_uris,
    })
}

fn apple_config(
    enabled: Option<String>,
    audiences: Option<String>,
    team_id: Option<String>,
    key_id: Option<String>,
    private_key: Option<String>,
) -> Result<AppleConfig, AuthConfigError> {
    let enabled = boolean(
        "JAMYE_APPLE_SIGNIN_ENABLED",
        enabled.as_deref().unwrap_or("false"),
    )?;
    if !enabled {
        return Ok(AppleConfig {
            enabled,
            audiences: Vec::new(),
            team_id: None,
            key_id: None,
            private_key_der: None,
        });
    }
    let audiences = parse_list("JAMYE_APPLE_AUDIENCES", audiences)?;
    let team_id = SensitiveValue(required("JAMYE_APPLE_TEAM_ID", team_id)?);
    let key_id = SensitiveValue(required("JAMYE_APPLE_KEY_ID", key_id)?);
    let private_key_der = SensitiveBytes(parse_apple_private_key(private_key)?);
    validate_apple_signing_key(
        team_id.expose_secret(),
        key_id.expose_secret(),
        private_key_der.expose_secret(),
    )?;
    Ok(AppleConfig {
        enabled,
        audiences,
        team_id: Some(team_id),
        key_id: Some(key_id),
        private_key_der: Some(private_key_der),
    })
}

fn parse_list(key: &'static str, value: Option<String>) -> Result<Vec<String>, AuthConfigError> {
    let values = required(key, value)?
        .split(',')
        .map(str::trim)
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if values.is_empty() || values.iter().any(String::is_empty) {
        return Err(AuthConfigError::new(
            key,
            "must be a comma-separated non-empty list",
        ));
    }
    let mut unique = values.clone();
    unique.sort();
    unique.dedup();
    if unique.len() != values.len() {
        return Err(AuthConfigError::new(
            key,
            "must not contain duplicate values",
        ));
    }
    Ok(values)
}

fn parse_apple_private_key(value: Option<String>) -> Result<Vec<u8>, AuthConfigError> {
    let value = required("JAMYE_APPLE_PRIVATE_KEY", value)?;
    let normalized = value
        .lines()
        .map(str::trim)
        .filter(|line| !line.starts_with("-----BEGIN") && !line.starts_with("-----END"))
        .collect::<String>();
    if normalized.is_empty() {
        return Err(AuthConfigError::new(
            "JAMYE_APPLE_PRIVATE_KEY",
            "must be base64 PKCS#8 DER",
        ));
    }
    STANDARD
        .decode(normalized.as_bytes())
        .map_err(|_| AuthConfigError::new("JAMYE_APPLE_PRIVATE_KEY", "must be base64 PKCS#8 DER"))
}

fn validate_apple_signing_key(
    team_id: &str,
    key_id: &str,
    private_key_der: &[u8],
) -> Result<(), AuthConfigError> {
    let mut header = Header::new(Algorithm::ES256);
    header.kid = Some(key_id.to_owned());
    let key = EncodingKey::from_ec_der(private_key_der);
    let claims = AppleSigningProbeClaims {
        iss: team_id,
        sub: "jamye-config-validation",
        aud: APPLE_ISSUER,
        iat: 1,
        exp: 2,
    };
    encode(&header, &claims, &key).map(|_| ()).map_err(|_| {
        AuthConfigError::new("JAMYE_APPLE_PRIVATE_KEY", "must be a valid EC private key")
    })
}

fn parse_redirects(
    key: &'static str,
    value: Option<String>,
) -> Result<Vec<String>, AuthConfigError> {
    let value = required(key, value)?;
    let redirects = value
        .split(',')
        .map(str::trim)
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if redirects.is_empty()
        || redirects.iter().any(|redirect| {
            redirect.is_empty()
                || redirect.contains('*')
                || Url::parse(redirect).is_err()
                || redirect.contains('#')
        })
    {
        return Err(AuthConfigError::new(
            key,
            "must be a comma-separated exact URI allowlist without wildcards or fragments",
        ));
    }
    let mut unique = redirects.clone();
    unique.sort();
    unique.dedup();
    if unique.len() != redirects.len() {
        return Err(AuthConfigError::new(key, "must not contain duplicate URIs"));
    }
    Ok(redirects)
}

#[derive(Serialize)]
struct AppleSigningProbeClaims<'a> {
    iss: &'a str,
    sub: &'a str,
    aud: &'a str,
    iat: u64,
    exp: u64,
}

fn read(key: &'static str) -> Option<String> {
    env::var(key).ok()
}

fn required(key: &'static str, value: Option<String>) -> Result<String, AuthConfigError> {
    value
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| AuthConfigError::new(key, "is required"))
}

fn boolean(key: &'static str, value: &str) -> Result<bool, AuthConfigError> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(AuthConfigError::new(key, "must be true or false")),
    }
}

fn duration(
    key: &'static str,
    value: &str,
    minimum: u64,
    maximum: u64,
    constructor: fn(u64) -> Duration,
) -> Result<Duration, AuthConfigError> {
    let amount = value
        .parse::<u64>()
        .map_err(|_| AuthConfigError::new(key, "must be an integer"))?;
    if !(minimum..=maximum).contains(&amount) {
        return Err(AuthConfigError::new(key, "is outside the permitted range"));
    }
    Ok(constructor(amount))
}
