//! Feature-local private object-storage configuration.

use std::{fmt, net::Ipv4Addr};

use url::{Host, Url};

use super::{AppEnvironment, ConfigError};

const ENDPOINT_KEY: &str = "JAMYE_OBJECT_STORAGE_ENDPOINT";
const PUBLIC_ENDPOINT_KEY: &str = "JAMYE_OBJECT_STORAGE_PUBLIC_ENDPOINT";
const REGION_KEY: &str = "JAMYE_OBJECT_STORAGE_REGION";
const BUCKET_KEY: &str = "JAMYE_OBJECT_STORAGE_BUCKET";
const ACCESS_KEY_ID_KEY: &str = "JAMYE_OBJECT_STORAGE_ACCESS_KEY_ID";
const SECRET_ACCESS_KEY_KEY: &str = "JAMYE_OBJECT_STORAGE_SECRET_ACCESS_KEY";

#[derive(Clone, Default)]
pub struct ObjectStorageConfigInput {
    pub endpoint: Option<String>,
    pub public_endpoint: Option<String>,
    pub region: Option<String>,
    pub bucket: Option<String>,
    pub access_key_id: Option<String>,
    pub secret_access_key: Option<String>,
}

/// Validated settings used to construct internal and public-presign S3 clients.
#[derive(Clone)]
pub struct ObjectStorageConfig {
    endpoint: Url,
    public_endpoint: Url,
    region: String,
    bucket: String,
    access_key_id: SensitiveValue,
    secret_access_key: SensitiveValue,
}

impl ObjectStorageConfig {
    pub fn resolve(
        environment: AppEnvironment,
        input: ObjectStorageConfigInput,
    ) -> Result<Option<Self>, ConfigError> {
        if environment != AppEnvironment::Production && all_absent(&input) {
            return Ok(None);
        }

        let endpoint = parse_endpoint(ENDPOINT_KEY, required_value(ENDPOINT_KEY, input.endpoint)?)?;
        let public_endpoint = parse_endpoint(
            PUBLIC_ENDPOINT_KEY,
            required_value(PUBLIC_ENDPOINT_KEY, input.public_endpoint)?,
        )?;
        if environment == AppEnvironment::Production {
            validate_production_public_endpoint(&public_endpoint)?;
        }
        let region = validate_region(required_value(REGION_KEY, input.region)?)?;
        let bucket = validate_bucket(required_value(BUCKET_KEY, input.bucket)?)?;
        let access_key_id = SensitiveValue(validate_credential(
            ACCESS_KEY_ID_KEY,
            required_value(ACCESS_KEY_ID_KEY, input.access_key_id)?,
            128,
        )?);
        let secret_access_key = SensitiveValue(validate_credential(
            SECRET_ACCESS_KEY_KEY,
            required_value(SECRET_ACCESS_KEY_KEY, input.secret_access_key)?,
            256,
        )?);

        Ok(Some(Self {
            endpoint,
            public_endpoint,
            region,
            bucket,
            access_key_id,
            secret_access_key,
        }))
    }

    pub fn endpoint(&self) -> &str {
        self.endpoint.as_str()
    }

    pub fn public_endpoint(&self) -> &str {
        self.public_endpoint.as_str()
    }

    pub fn region(&self) -> &str {
        &self.region
    }

    pub fn bucket(&self) -> &str {
        &self.bucket
    }

    pub(crate) fn access_key_id(&self) -> &str {
        self.access_key_id.expose()
    }

    pub(crate) fn secret_access_key(&self) -> &str {
        self.secret_access_key.expose()
    }
}

impl fmt::Debug for ObjectStorageConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ObjectStorageConfig")
            .field("endpoint", &self.endpoint)
            .field("public_endpoint", &self.public_endpoint)
            .field("region", &self.region)
            .field("bucket", &self.bucket)
            .field("access_key_id", &self.access_key_id)
            .field("secret_access_key", &self.secret_access_key)
            .finish()
    }
}

#[derive(Clone)]
struct SensitiveValue(String);

impl SensitiveValue {
    fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SensitiveValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[REDACTED]")
    }
}

fn all_absent(input: &ObjectStorageConfigInput) -> bool {
    [
        input.endpoint.as_deref(),
        input.public_endpoint.as_deref(),
        input.region.as_deref(),
        input.bucket.as_deref(),
        input.access_key_id.as_deref(),
        input.secret_access_key.as_deref(),
    ]
    .into_iter()
    .all(|value| value.is_none_or(|value| value.trim().is_empty()))
}

fn required_value(key: &'static str, value: Option<String>) -> Result<String, ConfigError> {
    value
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| ConfigError::new(key, "is required"))
}

fn parse_endpoint(key: &'static str, value: String) -> Result<Url, ConfigError> {
    let endpoint = Url::parse(&value).map_err(|_| ConfigError::new(key, "must be a valid URL"))?;
    if !matches!(endpoint.scheme(), "http" | "https") || endpoint.host().is_none() {
        return Err(ConfigError::new(key, "uses an unsupported URL form"));
    }
    let has_user_info = !endpoint.username().is_empty() || endpoint.password().is_some();
    let has_suffix_data = endpoint.query().is_some() || endpoint.fragment().is_some();
    if has_user_info || has_suffix_data || endpoint.path() != "/" {
        return Err(ConfigError::new(
            key,
            "must be an origin without credentials, path, query, or fragment",
        ));
    }
    Ok(endpoint)
}

fn validate_production_public_endpoint(endpoint: &Url) -> Result<(), ConfigError> {
    if endpoint.scheme() != "https" {
        return Err(ConfigError::new(
            PUBLIC_ENDPOINT_KEY,
            "must use HTTPS in production",
        ));
    }
    let external_domain = match endpoint.host() {
        Some(Host::Domain(domain)) => {
            let domain = domain.to_ascii_lowercase();
            domain.contains('.')
                && domain != "localhost"
                && !domain.ends_with(".localhost")
                && !domain.ends_with(".local")
                && !domain.ends_with(".internal")
        }
        Some(Host::Ipv4(_)) | Some(Host::Ipv6(_)) | None => false,
    };
    if !external_domain {
        return Err(ConfigError::new(
            PUBLIC_ENDPOINT_KEY,
            "must use an external DNS host in production",
        ));
    }
    Ok(())
}

fn validate_region(value: String) -> Result<String, ConfigError> {
    let valid = value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-');
    if !valid {
        return Err(ConfigError::new(
            REGION_KEY,
            "must contain only ASCII letters, digits, or hyphens",
        ));
    }
    Ok(value)
}

fn validate_bucket(value: String) -> Result<String, ConfigError> {
    let bytes = value.as_bytes();
    let valid_length = (3..=63).contains(&bytes.len());
    let valid_edges = bytes
        .first()
        .zip(bytes.last())
        .is_some_and(|(first, last)| first.is_ascii_alphanumeric() && last.is_ascii_alphanumeric());
    let valid_bytes = bytes.iter().copied().all(|byte| {
        byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'-')
    });
    let valid_separators = !value.contains("..") && !value.contains(".-") && !value.contains("-.");
    let not_ip_address = value.parse::<Ipv4Addr>().is_err();
    if !(valid_length && valid_edges && valid_bytes && valid_separators && not_ip_address) {
        return Err(ConfigError::new(
            BUCKET_KEY,
            "is not a valid private bucket name",
        ));
    }
    Ok(value)
}

fn validate_credential(
    key: &'static str,
    value: String,
    maximum_length: usize,
) -> Result<String, ConfigError> {
    if value.len() > maximum_length || value.chars().any(char::is_control) {
        return Err(ConfigError::new(key, "uses an unsupported credential form"));
    }
    Ok(value)
}
