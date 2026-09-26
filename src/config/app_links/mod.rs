//! Validated app-link association and invite landing configuration.

use std::{env, error::Error, fmt};

use url::Url;

const DEFAULT_AASA_APP_IDS: &str = "6ZH8V43A7D.dev.local.jamyeapp";
const DEFAULT_ANDROID_PACKAGE: &str = "dev.local.jamyeapp";
const DEFAULT_ANDROID_SHA256_CERT_FINGERPRINTS: &str = "FA:C6:17:45:DC:09:03:78:6F:B9:ED:E6:2A:96:2B:39:9F:73:48:F0:BB:6F:89:9B:83:32:66:75:91:03:3B:9C";

#[derive(Clone, Default)]
pub struct AppLinksConfigInput {
    pub aasa_app_ids: Option<String>,
    pub android_package: Option<String>,
    pub android_sha256_cert_fingerprints: Option<String>,
    pub app_store_url: Option<String>,
    pub play_store_url: Option<String>,
}

impl AppLinksConfigInput {
    pub fn from_env() -> Self {
        Self {
            aasa_app_ids: read("JAMYE_APP_LINKS_AASA_APP_IDS"),
            android_package: read("JAMYE_APP_LINKS_ANDROID_PACKAGE"),
            android_sha256_cert_fingerprints: read(
                "JAMYE_APP_LINKS_ANDROID_SHA256_CERT_FINGERPRINTS",
            ),
            app_store_url: read("JAMYE_APP_STORE_URL"),
            play_store_url: read("JAMYE_PLAY_STORE_URL"),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppLinksConfig {
    aasa_app_ids: Vec<String>,
    android_package: String,
    android_sha256_cert_fingerprints: Vec<String>,
    app_store_url: Option<String>,
    play_store_url: Option<String>,
}

impl AppLinksConfig {
    pub fn from_env() -> Result<Self, AppLinksConfigError> {
        Self::try_from(AppLinksConfigInput::from_env())
    }

    pub fn aasa_app_ids(&self) -> &[String] {
        &self.aasa_app_ids
    }

    pub fn android_package(&self) -> &str {
        &self.android_package
    }

    pub fn android_sha256_cert_fingerprints(&self) -> &[String] {
        &self.android_sha256_cert_fingerprints
    }

    pub fn app_store_url(&self) -> Option<&str> {
        self.app_store_url.as_deref()
    }

    pub fn play_store_url(&self) -> Option<&str> {
        self.play_store_url.as_deref()
    }
}

impl TryFrom<AppLinksConfigInput> for AppLinksConfig {
    type Error = AppLinksConfigError;

    fn try_from(input: AppLinksConfigInput) -> Result<Self, Self::Error> {
        let aasa_app_ids = parse_list(
            "JAMYE_APP_LINKS_AASA_APP_IDS",
            input
                .aasa_app_ids
                .as_deref()
                .unwrap_or(DEFAULT_AASA_APP_IDS),
            valid_aasa_app_id,
            "must be comma-separated TeamID.bundle identifiers",
        )?;
        let android_package = input
            .android_package
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_ANDROID_PACKAGE.to_owned());
        if !valid_android_package(&android_package) {
            return Err(AppLinksConfigError::new(
                "JAMYE_APP_LINKS_ANDROID_PACKAGE",
                "must be a valid Android package name",
            ));
        }
        let android_sha256_cert_fingerprints = parse_list(
            "JAMYE_APP_LINKS_ANDROID_SHA256_CERT_FINGERPRINTS",
            input
                .android_sha256_cert_fingerprints
                .as_deref()
                .unwrap_or(DEFAULT_ANDROID_SHA256_CERT_FINGERPRINTS),
            valid_sha256_fingerprint,
            "must be comma-separated SHA-256 certificate fingerprints",
        )?
        .into_iter()
        .map(|value| value.to_ascii_uppercase())
        .collect::<Vec<_>>();
        let app_store_url = optional_https_url("JAMYE_APP_STORE_URL", input.app_store_url)?;
        let play_store_url = optional_https_url("JAMYE_PLAY_STORE_URL", input.play_store_url)?;
        Ok(Self {
            aasa_app_ids,
            android_package,
            android_sha256_cert_fingerprints,
            app_store_url,
            play_store_url,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppLinksConfigError {
    key: &'static str,
    reason: &'static str,
}

impl AppLinksConfigError {
    fn new(key: &'static str, reason: &'static str) -> Self {
        Self { key, reason }
    }

    pub fn key(&self) -> &'static str {
        self.key
    }
}

impl fmt::Display for AppLinksConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "invalid configuration for {}: {}",
            self.key, self.reason
        )
    }
}

impl Error for AppLinksConfigError {}

fn parse_list(
    key: &'static str,
    value: &str,
    validator: fn(&str) -> bool,
    reason: &'static str,
) -> Result<Vec<String>, AppLinksConfigError> {
    let items = value
        .split(',')
        .map(str::trim)
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if items.is_empty() || items.iter().any(|item| item.is_empty() || !validator(item)) {
        return Err(AppLinksConfigError::new(key, reason));
    }
    let mut unique = items.clone();
    unique.sort();
    unique.dedup();
    if unique.len() != items.len() {
        return Err(AppLinksConfigError::new(key, "must not contain duplicates"));
    }
    Ok(items)
}

fn optional_https_url(
    key: &'static str,
    value: Option<String>,
) -> Result<Option<String>, AppLinksConfigError> {
    let Some(value) = value.filter(|value| !value.trim().is_empty()) else {
        return Ok(None);
    };
    let parsed =
        Url::parse(&value).map_err(|_| AppLinksConfigError::new(key, "must be a valid URL"))?;
    if parsed.scheme() != "https" || parsed.host_str().is_none() {
        return Err(AppLinksConfigError::new(
            key,
            "must be an HTTPS URL with a host",
        ));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(AppLinksConfigError::new(
            key,
            "must not contain credentials",
        ));
    }
    Ok(Some(value))
}

fn valid_aasa_app_id(value: &str) -> bool {
    let Some((team_id, bundle_id)) = value.split_once('.') else {
        return false;
    };
    team_id.len() == 10
        && team_id.chars().all(|ch| ch.is_ascii_alphanumeric())
        && valid_bundle_or_package(bundle_id, true)
}

fn valid_android_package(value: &str) -> bool {
    valid_bundle_or_package(value, false)
}

fn valid_bundle_or_package(value: &str, allow_hyphen: bool) -> bool {
    let mut parts = value.split('.').peekable();
    if parts.peek().is_none() {
        return false;
    }
    let mut count = 0;
    for part in parts {
        count += 1;
        let mut chars = part.chars();
        let Some(first) = chars.next() else {
            return false;
        };
        if !(first.is_ascii_alphabetic() || first == '_') {
            return false;
        }
        if !chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || (allow_hyphen && ch == '-')) {
            return false;
        }
    }
    count >= 2
}

fn valid_sha256_fingerprint(value: &str) -> bool {
    let parts = value.split(':').collect::<Vec<_>>();
    parts.len() == 32
        && parts
            .iter()
            .all(|part| part.len() == 2 && part.chars().all(|ch| ch.is_ascii_hexdigit()))
}

fn read(key: &'static str) -> Option<String> {
    env::var(key).ok()
}
