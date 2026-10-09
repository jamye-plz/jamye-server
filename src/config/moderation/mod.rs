//! Feature-local moderation configuration: operator alert recipients and the content-filter
//! term list. Both are optional; a malformed value is a startup error that names the key
//! (never the value).

use std::{env, fs};

use uuid::Uuid;

use super::ConfigError;
use crate::domain::moderation::{ContentFilter, ContentFilterError, MAX_OPERATOR_ACCOUNTS};

const OPERATOR_ACCOUNT_IDS_KEY: &str = "JAMYE_OPERATOR_ACCOUNT_IDS";
const CONTENT_FILTER_TERMS_FILE_KEY: &str = "JAMYE_CONTENT_FILTER_TERMS_FILE";
const MAX_TERMS_FILE_BYTES: u64 = 256 * 1024;

/// Unvalidated values. Blank values count as unset.
#[derive(Clone, Default)]
pub struct ModerationConfigInput {
    pub operator_account_ids: Option<String>,
    pub content_filter_terms_file: Option<String>,
}

impl ModerationConfigInput {
    pub fn from_env() -> Self {
        Self {
            operator_account_ids: env::var(OPERATOR_ACCOUNT_IDS_KEY).ok(),
            content_filter_terms_file: env::var(CONTENT_FILTER_TERMS_FILE_KEY).ok(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ModerationConfig {
    operator_account_ids: Vec<Uuid>,
    content_filter: ContentFilter,
}

impl Default for ModerationConfig {
    fn default() -> Self {
        Self {
            operator_account_ids: Vec::new(),
            content_filter: ContentFilter::embedded(),
        }
    }
}

impl ModerationConfig {
    pub fn resolve(input: ModerationConfigInput) -> Result<Self, ConfigError> {
        Ok(Self {
            operator_account_ids: parse_operator_account_ids(input.operator_account_ids)?,
            content_filter: resolve_content_filter(input.content_filter_terms_file)?,
        })
    }

    /// Accounts that receive the generic operator alert after a report commits (empty = none).
    pub fn operator_account_ids(&self) -> &[Uuid] {
        &self.operator_account_ids
    }

    /// The embedded default list, or the operator's replacement.
    pub fn content_filter(&self) -> &ContentFilter {
        &self.content_filter
    }
}

fn parse_operator_account_ids(value: Option<String>) -> Result<Vec<Uuid>, ConfigError> {
    let Some(value) = value.filter(|value| !value.trim().is_empty()) else {
        return Ok(Vec::new());
    };
    let mut ids: Vec<Uuid> = Vec::new();
    for segment in value.split(',') {
        let id = Uuid::try_parse(segment.trim()).map_err(|_| {
            ConfigError::new(OPERATOR_ACCOUNT_IDS_KEY, "must be comma-separated UUIDs")
        })?;
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    if ids.len() > MAX_OPERATOR_ACCOUNTS {
        return Err(ConfigError::new(
            OPERATOR_ACCOUNT_IDS_KEY,
            "lists too many accounts",
        ));
    }
    Ok(ids)
}

fn resolve_content_filter(path: Option<String>) -> Result<ContentFilter, ConfigError> {
    let Some(path) = path
        .map(|path| path.trim().to_owned())
        .filter(|path| !path.is_empty())
    else {
        return Ok(ContentFilter::embedded());
    };
    let unreadable = || ConfigError::new(CONTENT_FILTER_TERMS_FILE_KEY, "must be a readable file");
    let metadata = fs::metadata(&path).map_err(|_| unreadable())?;
    if !metadata.is_file() || metadata.len() > MAX_TERMS_FILE_BYTES {
        return Err(unreadable());
    }
    let source = fs::read_to_string(&path).map_err(|_| unreadable())?;
    ContentFilter::from_lines(&source).map_err(|error| {
        ConfigError::new(
            CONTENT_FILTER_TERMS_FILE_KEY,
            match error {
                ContentFilterError::EmptyList => "must list at least one term",
                ContentFilterError::TooManyTerms => "lists too many terms",
                ContentFilterError::TermTooLong => "contains a term that is too long",
                ContentFilterError::TermTooShort => "contains a term shorter than 2 characters",
                ContentFilterError::ControlCharacter => "contains a control character",
            },
        )
    })
}
