//! Feature-local public legal-page operator configuration.
//!
//! The legal pages (`/privacy`, `/terms`, `/account-deletion`, `/support`) are mounted
//! only when the operator name and the contact email are both configured. Nothing set keeps
//! the routes unmounted so the code can deploy before the values exist; a partial or invalid
//! set is a startup error that names the key (never the value).

use std::env;

use super::ConfigError;

const OPERATOR_NAME_KEY: &str = "JAMYE_LEGAL_OPERATOR_NAME";
const CONTACT_EMAIL_KEY: &str = "JAMYE_LEGAL_CONTACT_EMAIL";
const OPERATOR_ADDRESS_KEY: &str = "JAMYE_LEGAL_OPERATOR_ADDRESS";
const MAX_OPERATOR_NAME_CHARS: usize = 100;
const MAX_CONTACT_EMAIL_CHARS: usize = 254;
const MAX_OPERATOR_ADDRESS_CHARS: usize = 200;
const PARTIAL_SET_REASON: &str = "is required when any JAMYE_LEGAL_* value is set";

/// Unvalidated legal-page values. Blank values count as unset.
#[derive(Clone, Default)]
pub struct LegalConfigInput {
    pub operator_name: Option<String>,
    pub contact_email: Option<String>,
    pub operator_address: Option<String>,
}

impl LegalConfigInput {
    pub fn from_env() -> Self {
        Self {
            operator_name: read(OPERATOR_NAME_KEY),
            contact_email: read(CONTACT_EMAIL_KEY),
            operator_address: read(OPERATOR_ADDRESS_KEY),
        }
    }
}

/// Validated, trimmed operator values. Rendering HTML-escapes them; validation additionally
/// rejects control characters and template delimiters so a value can never form a placeholder.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LegalConfig {
    operator_name: String,
    contact_email: String,
    operator_address: Option<String>,
}

impl LegalConfig {
    /// Loads the process environment. `Ok(None)` means the legal pages stay unmounted.
    pub fn from_env() -> Result<Option<Self>, ConfigError> {
        Self::resolve(LegalConfigInput::from_env())
    }

    pub fn resolve(input: LegalConfigInput) -> Result<Option<Self>, ConfigError> {
        let name = present(input.operator_name);
        let email = present(input.contact_email);
        let address = present(input.operator_address);
        if name.is_none() && email.is_none() && address.is_none() {
            return Ok(None);
        }
        let name = name.ok_or_else(|| ConfigError::new(OPERATOR_NAME_KEY, PARTIAL_SET_REASON))?;
        let email = email.ok_or_else(|| ConfigError::new(CONTACT_EMAIL_KEY, PARTIAL_SET_REASON))?;
        let operator_name = validate_text(OPERATOR_NAME_KEY, name, MAX_OPERATOR_NAME_CHARS)?;
        let contact_email = validate_email(email)?;
        let operator_address = address
            .map(|value| validate_text(OPERATOR_ADDRESS_KEY, value, MAX_OPERATOR_ADDRESS_CHARS))
            .transpose()?;
        Ok(Some(Self {
            operator_name,
            contact_email,
            operator_address,
        }))
    }

    pub fn operator_name(&self) -> &str {
        &self.operator_name
    }

    pub fn contact_email(&self) -> &str {
        &self.contact_email
    }

    pub fn operator_address(&self) -> Option<&str> {
        self.operator_address.as_deref()
    }
}

fn read(key: &'static str) -> Option<String> {
    env::var(key).ok()
}

fn present(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn validate_text(
    key: &'static str,
    value: String,
    max_chars: usize,
) -> Result<String, ConfigError> {
    if value.chars().count() > max_chars {
        return Err(ConfigError::new(key, "is too long"));
    }
    if value.chars().any(char::is_control) {
        return Err(ConfigError::new(key, "must not contain control characters"));
    }
    if value.contains("{{") || value.contains("}}") {
        return Err(ConfigError::new(
            key,
            "must not contain template delimiters",
        ));
    }
    Ok(value)
}

fn validate_email(value: String) -> Result<String, ConfigError> {
    let value = validate_text(CONTACT_EMAIL_KEY, value, MAX_CONTACT_EMAIL_CHARS)?;
    if !valid_email(&value) {
        return Err(ConfigError::new(
            CONTACT_EMAIL_KEY,
            "must be a plain ASCII email address",
        ));
    }
    Ok(value)
}

fn valid_email(value: &str) -> bool {
    let Some((local, domain)) = value.split_once('@') else {
        return false;
    };
    valid_local_part(local) && valid_domain(domain)
}

fn valid_local_part(local: &str) -> bool {
    (1..=64).contains(&local.len())
        && !local.starts_with('.')
        && !local.ends_with('.')
        && !local.contains("..")
        && local.bytes().all(valid_local_byte)
}

fn valid_local_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'+' | b'-')
}

fn valid_domain(domain: &str) -> bool {
    let labels = domain.split('.').collect::<Vec<_>>();
    let Some(top_level) = labels.last() else {
        return false;
    };
    labels.len() >= 2
        && labels.iter().all(|label| valid_domain_label(label))
        && top_level.len() >= 2
}

fn valid_domain_label(label: &str) -> bool {
    (1..=63).contains(&label.len())
        && !label.starts_with('-')
        && !label.ends_with('-')
        && label
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}
