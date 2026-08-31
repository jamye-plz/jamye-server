//! Account-deletion-specific object cleanup provider.
//!
//! This deliberately stays separate from Task-8 media storage. It uses only
//! the internal S3 origin because cleanup keys are authorized by the worker.

use std::fmt;

use aws_sdk_s3::{
    Client,
    config::{Credentials, Region, retry::RetryConfig},
    error::{ProvideErrorMetadata, SdkError},
    operation::delete_object::DeleteObjectError,
};

use crate::{
    config::object_storage::ObjectStorageConfig,
    ports::{
        account_deletion::{AccountObjectDeletionProvider, AccountObjectDeletionProviderFuture},
        object_storage::ObjectStorageProviderError,
    },
};

#[derive(Clone)]
pub struct S3AccountObjectDeletionProvider {
    client: Client,
    bucket: String,
}

const CLEANUP_ACCESS_KEY_ID_KEY: &str = "JAMYE_ACCOUNT_OBJECT_DELETION_ACCESS_KEY_ID";
const CLEANUP_SECRET_ACCESS_KEY_KEY: &str = "JAMYE_ACCOUNT_OBJECT_DELETION_SECRET_ACCESS_KEY";

/// Credentials for the cleanup-only object-storage identity.
#[derive(Clone)]
pub struct S3AccountObjectDeletionCredentials {
    access_key_id: SensitiveCredential,
    secret_access_key: SensitiveCredential,
}

impl S3AccountObjectDeletionCredentials {
    pub fn new(
        access_key_id: impl Into<String>,
        secret_access_key: impl Into<String>,
    ) -> Result<Self, S3AccountObjectDeletionConfigError> {
        Ok(Self {
            access_key_id: SensitiveCredential(validate_credential(
                CLEANUP_ACCESS_KEY_ID_KEY,
                access_key_id.into(),
                128,
            )?),
            secret_access_key: SensitiveCredential(validate_credential(
                CLEANUP_SECRET_ACCESS_KEY_KEY,
                secret_access_key.into(),
                256,
            )?),
        })
    }
}

impl fmt::Debug for S3AccountObjectDeletionCredentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("S3AccountObjectDeletionCredentials")
            .field("access_key_id", &self.access_key_id)
            .field("secret_access_key", &self.secret_access_key)
            .finish()
    }
}

#[derive(Clone)]
struct SensitiveCredential(String);

impl SensitiveCredential {
    fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SensitiveCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[REDACTED]")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum S3AccountObjectDeletionConfigError {
    InvalidAccessKeyId,
    InvalidSecretAccessKey,
    ReusesMediaCredentials,
}

impl fmt::Display for S3AccountObjectDeletionConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("invalid account object-deletion provider configuration")
    }
}

impl std::error::Error for S3AccountObjectDeletionConfigError {}

impl S3AccountObjectDeletionProvider {
    /// Builds the internal SigV4 client without contacting object storage.
    pub fn new(
        config: &ObjectStorageConfig,
        cleanup_credentials: &S3AccountObjectDeletionCredentials,
    ) -> Result<Self, S3AccountObjectDeletionConfigError> {
        if cleanup_credentials.access_key_id.expose() == config.access_key_id()
            || cleanup_credentials.secret_access_key.expose() == config.secret_access_key()
        {
            return Err(S3AccountObjectDeletionConfigError::ReusesMediaCredentials);
        }
        let credentials = Credentials::new(
            cleanup_credentials.access_key_id.expose().to_owned(),
            cleanup_credentials.secret_access_key.expose().to_owned(),
            None,
            None,
            "jamye-account-object-cleanup",
        );
        let sdk_config = aws_sdk_s3::Config::builder()
            .behavior_version_latest()
            .region(Region::new(config.region().to_owned()))
            .credentials_provider(credentials)
            .endpoint_url(config.endpoint())
            .force_path_style(true)
            .retry_config(RetryConfig::standard().with_max_attempts(1))
            .build();

        Ok(Self {
            client: Client::from_conf(sdk_config),
            bucket: config.bucket().to_owned(),
        })
    }
}

impl fmt::Debug for S3AccountObjectDeletionProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("S3AccountObjectDeletionProvider")
            .field("client", &"[REDACTED]")
            .field("bucket", &"[REDACTED]")
            .finish()
    }
}

impl AccountObjectDeletionProvider for S3AccountObjectDeletionProvider {
    fn delete_object<'a>(&'a self, object_key: &'a str) -> AccountObjectDeletionProviderFuture<'a> {
        Box::pin(async move {
            self.client
                .delete_object()
                .bucket(&self.bucket)
                .key(object_key)
                .send()
                .await
                .map(|_| ())
                .map_err(|error| classify_delete_error(&error))
        })
    }
}

fn classify_delete_error(error: &SdkError<DeleteObjectError>) -> ObjectStorageProviderError {
    let status = error
        .raw_response()
        .map(|response| response.status().as_u16());
    let code = error
        .as_service_error()
        .and_then(ProvideErrorMetadata::code);

    if status == Some(403) || code == Some("AccessDenied") {
        ObjectStorageProviderError::AccessDenied
    } else if status.is_some_and(|status| (500..=599).contains(&status))
        || matches!(
            error,
            SdkError::TimeoutError(_) | SdkError::DispatchFailure(_)
        )
    {
        ObjectStorageProviderError::Unavailable
    } else {
        ObjectStorageProviderError::UnexpectedResponse
    }
}

fn validate_credential(
    key: &'static str,
    value: String,
    maximum_length: usize,
) -> Result<String, S3AccountObjectDeletionConfigError> {
    if value.trim().is_empty()
        || value.len() > maximum_length
        || value.chars().any(char::is_control)
    {
        return Err(if key == CLEANUP_ACCESS_KEY_ID_KEY {
            S3AccountObjectDeletionConfigError::InvalidAccessKeyId
        } else {
            S3AccountObjectDeletionConfigError::InvalidSecretAccessKey
        });
    }
    Ok(value)
}
