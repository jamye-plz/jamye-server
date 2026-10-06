//! Avatar upload intent, finalize, and public read use cases.

use std::{fmt, sync::Arc, time::Duration};

use uuid::Uuid;

use crate::{
    application::media::MediaEndpointRateLimit,
    domain::{
        avatar::{
            AVATAR_CONTENT_TYPE, AVATAR_PUT_TTL_SECONDS, MAX_AVATAR_BYTES, avatar_public_url,
            has_jpeg_signature, mint_avatar_object_key, validate_avatar_upload,
        },
        media::MediaKind,
    },
    ports::{
        auth::UserProfile,
        avatar::{
            ActivateAvatarCommand, AvatarFinalizePreparation, AvatarRepository,
            AvatarRepositoryError, CreateAvatarIntentCommand, PrepareAvatarFinalizeQuery,
        },
        object_storage::{
            InspectObjectRequest, MediaObjectStorage, ObjectStorageProviderError,
            PresignPutRequest, PresignedPut,
        },
        rate_limit::{RateLimitOutcome, RateLimitRequest, RateLimiter},
        transactions::{BoxTransactionHandle, TransactionHandle, TransactionManager},
    },
};

/// Fixed-window limit for the unauthenticated public avatar read.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AvatarEndpointRateLimit {
    pub limit: u32,
    pub window: Duration,
}

#[derive(Clone)]
pub struct AvatarDependencies {
    pub transactions: Arc<dyn TransactionManager>,
    pub repository: Arc<dyn AvatarRepository>,
    pub object_storage: Arc<dyn MediaObjectStorage>,
    pub rate_limiter: Arc<dyn RateLimiter>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AvatarSettings {
    /// Validated https origin without a trailing slash.
    pub public_base_url: String,
    /// Reuses the media presign values; the counter key is separate.
    pub upload_rate_limit: MediaEndpointRateLimit,
    pub public_read_rate_limit: AvatarEndpointRateLimit,
}

#[derive(Clone)]
pub struct AvatarService {
    dependencies: AvatarDependencies,
    settings: AvatarSettings,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AvatarUploadInput {
    pub content_type: String,
    pub byte_size: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AvatarUploadIntent {
    pub upload_id: Uuid,
    pub put: PresignedPut,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AvatarImage {
    pub bytes: Vec<u8>,
}

impl AvatarService {
    pub fn new(
        dependencies: AvatarDependencies,
        settings: AvatarSettings,
    ) -> Result<Self, AvatarError> {
        let upload = settings.upload_rate_limit;
        let read = settings.public_read_rate_limit;
        if upload.limit == 0
            || upload.window.is_zero()
            || read.limit == 0
            || read.window.is_zero()
            || settings.public_base_url.is_empty()
        {
            return Err(AvatarError::InvalidConfiguration);
        }
        Ok(Self {
            dependencies,
            settings,
        })
    }

    /// U4: validate, rate-limit, supersede the previous pending upload, and presign a PUT.
    pub async fn create_upload(
        &self,
        actor_id: Uuid,
        input: AvatarUploadInput,
    ) -> Result<AvatarUploadIntent, AvatarError> {
        validate_avatar_upload(&input.content_type, input.byte_size)
            .map_err(|_| AvatarError::RequestValidation)?;
        // Same limiter and values as chat media presign; a separate counter subject.
        self.check_rate_limit(
            "media_upload_presign",
            format!("user:{actor_id}:avatar"),
            self.settings.upload_rate_limit.limit,
            self.settings.upload_rate_limit.window,
        )
        .await?;

        let upload_id = Uuid::new_v4();
        let command = CreateAvatarIntentCommand {
            id: upload_id,
            user_id: actor_id,
            object_key: mint_avatar_object_key(actor_id, upload_id),
            byte_size: input.byte_size,
            expires_in: Duration::from_secs(AVATAR_PUT_TTL_SECONDS),
        };
        let mut transaction = self.begin().await?;
        let result = self
            .create_in_transaction(transaction.as_mut(), &command)
            .await;
        self.finish(transaction, result).await
    }

    async fn create_in_transaction(
        &self,
        transaction: &mut dyn TransactionHandle,
        command: &CreateAvatarIntentCommand,
    ) -> Result<AvatarUploadIntent, AvatarError> {
        let record = self
            .dependencies
            .repository
            .create_intent(transaction, command)
            .await
            .map_err(|error| match error {
                AvatarRepositoryError::UploadNotFound => AvatarError::AccountNotActive,
                other => AvatarError::from(other),
            })?;
        let put = self
            .dependencies
            .object_storage
            .presign_put(&PresignPutRequest {
                object_key: record.object_key,
                content_type: AVATAR_CONTENT_TYPE.to_owned(),
                byte_size: record.byte_size,
                expires_in: command.expires_in,
            })
            .await
            .map_err(|_| AvatarError::ObjectStorageDegraded)?;
        Ok(AvatarUploadIntent {
            upload_id: record.id,
            put,
        })
    }

    /// U5: verify ownership, state, and the uploaded object, then activate atomically.
    pub async fn finalize(
        &self,
        actor_id: Uuid,
        upload_id: Uuid,
    ) -> Result<UserProfile, AvatarError> {
        let preparation = self
            .dependencies
            .repository
            .prepare_finalize(&PrepareAvatarFinalizeQuery {
                actor_id,
                upload_id,
            })
            .await
            .map_err(AvatarError::from)?;
        let pending = match preparation {
            AvatarFinalizePreparation::AlreadyActive(profile) => return Ok(profile),
            AvatarFinalizePreparation::Pending(pending) => pending,
        };

        let inspected = self
            .dependencies
            .object_storage
            .inspect_object(&InspectObjectRequest {
                object_key: pending.object_key.clone(),
                kind: MediaKind::Image,
            })
            .await
            .map_err(provider_error_for_upload)?;
        if inspected.content_type.as_deref() != Some(AVATAR_CONTENT_TYPE)
            || inspected.byte_size != Some(pending.byte_size)
        {
            return Err(AvatarError::ObjectInvalid);
        }
        let bytes = self
            .dependencies
            .object_storage
            .get_object(&pending.object_key, MAX_AVATAR_BYTES)
            .await
            .map_err(provider_error_for_upload)?;
        if u64::try_from(bytes.len()).ok() != Some(pending.byte_size) || !has_jpeg_signature(&bytes)
        {
            return Err(AvatarError::ObjectInvalid);
        }

        let command = ActivateAvatarCommand {
            actor_id,
            upload_id,
            public_url: avatar_public_url(&self.settings.public_base_url, upload_id),
        };
        let mut transaction = self.begin().await?;
        let result = self
            .dependencies
            .repository
            .activate(transaction.as_mut(), &command)
            .await
            .map_err(AvatarError::from);
        self.finish(transaction, result).await
    }

    /// U6: rate-limit by peer, then stream the active avatar. Anything that is not an
    /// active upload id (including a malformed id) is one indistinguishable not-found.
    pub async fn read_public(
        &self,
        rate_key: &str,
        avatar_id: &str,
    ) -> Result<AvatarImage, AvatarError> {
        self.check_rate_limit(
            "avatar_public_read",
            rate_key.to_owned(),
            self.settings.public_read_rate_limit.limit,
            self.settings.public_read_rate_limit.window,
        )
        .await?;
        let avatar_id = Uuid::try_parse(avatar_id).map_err(|_| AvatarError::AvatarNotFound)?;
        let record = self
            .dependencies
            .repository
            .find_active(avatar_id)
            .await
            .map_err(AvatarError::from)?
            .ok_or(AvatarError::AvatarNotFound)?;
        let bytes = self
            .dependencies
            .object_storage
            .get_object(&record.object_key, MAX_AVATAR_BYTES)
            .await
            .map_err(|_| AvatarError::ObjectStorageDegraded)?;
        // The presigned PUT stays valid after finalize, so the object may have been replaced.
        // Re-check the signature on the bytes about to be served; a mismatch is the same
        // not-found as any other non-avatar id.
        if !has_jpeg_signature(&bytes) {
            return Err(AvatarError::AvatarNotFound);
        }
        Ok(AvatarImage { bytes })
    }

    async fn check_rate_limit(
        &self,
        endpoint: &'static str,
        subject: String,
        limit: u32,
        window: Duration,
    ) -> Result<(), AvatarError> {
        match self
            .dependencies
            .rate_limiter
            .check(&RateLimitRequest {
                endpoint,
                subject,
                limit,
                window,
            })
            .await
            .map_err(|_| AvatarError::RateLimitUnavailable)?
        {
            RateLimitOutcome::Allowed => Ok(()),
            RateLimitOutcome::Denied { retry_after } => {
                Err(AvatarError::RateLimited { retry_after })
            }
        }
    }

    async fn begin(&self) -> Result<BoxTransactionHandle, AvatarError> {
        self.dependencies
            .transactions
            .begin()
            .await
            .map_err(|_| AvatarError::DatabaseUnavailable)
    }

    async fn finish<T>(
        &self,
        transaction: BoxTransactionHandle,
        result: Result<T, AvatarError>,
    ) -> Result<T, AvatarError> {
        match result {
            Ok(value) => {
                self.dependencies
                    .transactions
                    .commit(transaction)
                    .await
                    .map_err(|_| AvatarError::DatabaseUnavailable)?;
                Ok(value)
            }
            Err(error) => {
                self.dependencies
                    .transactions
                    .rollback(transaction)
                    .await
                    .map_err(|_| AvatarError::DatabaseUnavailable)?;
                Err(error)
            }
        }
    }
}

/// A missing object or an oversized one is the uploader's problem (422); an outage is not.
fn provider_error_for_upload(error: ObjectStorageProviderError) -> AvatarError {
    match error {
        ObjectStorageProviderError::UnexpectedResponse => AvatarError::ObjectInvalid,
        ObjectStorageProviderError::AccessDenied | ObjectStorageProviderError::Unavailable => {
            AvatarError::ObjectStorageDegraded
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AvatarError {
    RequestValidation,
    RateLimited { retry_after: Duration },
    RateLimitUnavailable,
    AccountNotActive,
    UploadNotFound,
    UploadNotPending,
    ObjectInvalid,
    AvatarNotFound,
    DatabaseUnavailable,
    ObjectStorageDegraded,
    InvalidConfiguration,
}

impl fmt::Display for AvatarError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("avatar operation failed")
    }
}

impl std::error::Error for AvatarError {}

impl From<AvatarRepositoryError> for AvatarError {
    fn from(error: AvatarRepositoryError) -> Self {
        match error {
            AvatarRepositoryError::UploadNotFound => Self::UploadNotFound,
            AvatarRepositoryError::UploadNotPending => Self::UploadNotPending,
            AvatarRepositoryError::InvalidData | AvatarRepositoryError::Unavailable => {
                Self::DatabaseUnavailable
            }
        }
    }
}
