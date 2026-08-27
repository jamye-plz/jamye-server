//! Private-media upload intent and finalization use cases.

use std::{fmt, sync::Arc, time::Duration};

use uuid::Uuid;

use crate::{
    domain::media::{
        MediaScope, PRESIGNED_GET_TTL_SECONDS, PRESIGNED_PUT_TTL_SECONDS,
        download_content_disposition, mint_object_key, validate_finalized_object, validate_upload,
    },
    ports::{
        media::{
            AuthorizeMediaAccessQuery, ConfirmedUploadRecord, CreateUploadIntentCommand,
            FinalizeUploadCommand, MediaAccessRecord, MediaRepository, MediaRepositoryError,
            PrepareUploadFinalizeQuery, TopicMediaBindingRecord, UploadFinalizePreparation,
            UploadFinalizeRecord, UploadIntentRecord,
        },
        object_storage::{
            InspectObjectRequest, MediaObjectStorage, PresignGetRequest, PresignPutRequest,
            PresignedGet,
        },
        rate_limit::{RateLimitOutcome, RateLimitRequest, RateLimiter},
        topics::{TopicStatus, TopicsRepository},
        transactions::{BoxTransactionHandle, TransactionManager},
    },
};

#[derive(Clone)]
pub struct MediaService {
    dependencies: MediaDependencies,
    upload_presign_rate_limit: MediaEndpointRateLimit,
}

#[derive(Clone)]
pub struct MediaDependencies {
    pub transactions: Arc<dyn TransactionManager>,
    pub repository: Arc<dyn MediaRepository>,
    pub object_storage: Arc<dyn MediaObjectStorage>,
    pub rate_limiter: Arc<dyn RateLimiter>,
}

#[derive(Clone)]
pub struct MediaFinalizeService {
    dependencies: MediaFinalizeDependencies,
}

#[derive(Clone)]
pub struct MediaFinalizeDependencies {
    pub transactions: Arc<dyn TransactionManager>,
    pub repository: Arc<dyn MediaRepository>,
    pub object_storage: Arc<dyn MediaObjectStorage>,
    pub topics: Arc<dyn TopicsRepository>,
}

#[derive(Clone)]
pub struct MediaAccessService {
    dependencies: MediaAccessDependencies,
}

#[derive(Clone)]
pub struct MediaAccessDependencies {
    pub repository: Arc<dyn MediaRepository>,
    pub object_storage: Arc<dyn MediaObjectStorage>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MediaEndpointRateLimit {
    pub limit: u32,
    pub window: Duration,
}

impl MediaService {
    pub fn new(
        dependencies: MediaDependencies,
        upload_presign_rate_limit: MediaEndpointRateLimit,
    ) -> Result<Self, MediaError> {
        if upload_presign_rate_limit.limit == 0 || upload_presign_rate_limit.window.is_zero() {
            return Err(MediaError::InvalidConfiguration);
        }
        Ok(Self {
            dependencies,
            upload_presign_rate_limit,
        })
    }

    pub async fn create_upload_intent(
        &self,
        actor_id: Uuid,
        input: UploadIntentCreateInput,
    ) -> Result<UploadIntentWithPresignedPut, MediaError> {
        let validated = validate_upload(
            input.scope,
            &input.content_type,
            input.byte_size,
            input.filename.as_deref(),
        )
        .map_err(|_| MediaError::RequestValidation)?;
        self.check_upload_rate_limit(actor_id, input.scope, input.target_id)
            .await?;

        let upload_id = Uuid::new_v4();
        let command = CreateUploadIntentCommand {
            id: upload_id,
            user_id: actor_id,
            scope: input.scope,
            target_id: input.target_id,
            object_key: mint_object_key(input.scope, input.target_id, upload_id),
            kind: validated.kind,
            content_type: validated.content_type,
            byte_size: validated.byte_size,
            filename: validated.filename,
            expires_in: Duration::from_secs(PRESIGNED_PUT_TTL_SECONDS),
        };
        let mut transaction = self.begin().await?;
        let result = self
            .create_upload_intent_in_transaction(transaction.as_mut(), &command)
            .await;
        self.finish(transaction, result).await
    }

    async fn check_upload_rate_limit(
        &self,
        actor_id: Uuid,
        scope: MediaScope,
        target_id: Uuid,
    ) -> Result<(), MediaError> {
        let subject = format!(
            "user:{actor_id}:scope:{}:target:{target_id}",
            scope_name(scope)
        );
        match self
            .dependencies
            .rate_limiter
            .check(&RateLimitRequest {
                endpoint: "media_upload_presign",
                subject,
                limit: self.upload_presign_rate_limit.limit,
                window: self.upload_presign_rate_limit.window,
            })
            .await
            .map_err(|_| MediaError::RateLimitUnavailable)?
        {
            RateLimitOutcome::Allowed => Ok(()),
            RateLimitOutcome::Denied { retry_after } => {
                Err(MediaError::RateLimited { retry_after })
            }
        }
    }

    async fn create_upload_intent_in_transaction(
        &self,
        transaction: &mut dyn crate::ports::transactions::TransactionHandle,
        command: &CreateUploadIntentCommand,
    ) -> Result<UploadIntentWithPresignedPut, MediaError> {
        let upload = self
            .dependencies
            .repository
            .create_upload_intent(transaction, command)
            .await
            .map_err(MediaError::from)?;
        let request = presign_request(&upload, command.expires_in);
        let put = self
            .dependencies
            .object_storage
            .presign_put(&request)
            .await
            .map_err(|_| MediaError::ObjectStorageDegraded)?;
        Ok(UploadIntentWithPresignedPut { upload, put })
    }

    async fn begin(&self) -> Result<BoxTransactionHandle, MediaError> {
        self.dependencies
            .transactions
            .begin()
            .await
            .map_err(|_| MediaError::DatabaseUnavailable)
    }

    async fn finish<T>(
        &self,
        transaction: BoxTransactionHandle,
        result: Result<T, MediaError>,
    ) -> Result<T, MediaError> {
        match result {
            Ok(value) => {
                self.dependencies
                    .transactions
                    .commit(transaction)
                    .await
                    .map_err(|_| MediaError::DatabaseUnavailable)?;
                Ok(value)
            }
            Err(error) => {
                self.dependencies
                    .transactions
                    .rollback(transaction)
                    .await
                    .map_err(|_| MediaError::DatabaseUnavailable)?;
                Err(error)
            }
        }
    }
}

impl MediaFinalizeService {
    pub fn new(dependencies: MediaFinalizeDependencies) -> Self {
        Self { dependencies }
    }

    pub async fn finalize_upload(
        &self,
        actor_id: Uuid,
        upload_id: Uuid,
        input: UploadFinalizeInput,
    ) -> Result<UploadFinalizeResult, MediaError> {
        let preparation = self
            .dependencies
            .repository
            .prepare_upload_finalize(&PrepareUploadFinalizeQuery {
                actor_id,
                upload_id,
                width: input.width,
                height: input.height,
            })
            .await
            .map_err(MediaError::from)?;
        let upload = match preparation {
            UploadFinalizePreparation::Existing(record) => return finalize_result(record),
            UploadFinalizePreparation::Pending(upload) => upload,
        };

        let expected = validate_upload(
            upload.scope,
            &upload.content_type,
            upload.byte_size,
            upload.filename.as_deref(),
        )
        .map_err(|_| MediaError::FinalizeValidation)?;
        if expected.kind != upload.kind {
            return Err(MediaError::FinalizeValidation);
        }
        let inspected = self
            .dependencies
            .object_storage
            .inspect_object(&InspectObjectRequest {
                object_key: upload.object_key.clone(),
                kind: expected.kind,
            })
            .await
            .map_err(|_| MediaError::ObjectStorageDegraded)?;
        let finalized = validate_finalized_object(&expected, &inspected)
            .map_err(|_| MediaError::FinalizeValidation)?;
        let command = match upload.scope {
            MediaScope::Chat => FinalizeUploadCommand::Chat {
                actor_id,
                upload_id,
                finalized,
            },
            MediaScope::Topic => FinalizeUploadCommand::Topic {
                actor_id,
                upload_id,
                topic_media_id: Uuid::new_v4(),
                width: input.width,
                height: input.height,
                finalized,
            },
        };

        let mut transaction = self.begin().await?;
        let result = self
            .finalize_in_transaction(transaction.as_mut(), upload.target_id, &command)
            .await;
        self.finish(transaction, result).await
    }

    async fn finalize_in_transaction(
        &self,
        transaction: &mut dyn crate::ports::transactions::TransactionHandle,
        target_id: Uuid,
        command: &FinalizeUploadCommand,
    ) -> Result<UploadFinalizeResult, MediaError> {
        let record = self
            .dependencies
            .repository
            .finalize_upload(transaction, command)
            .await
            .map_err(MediaError::from)?;
        match (command, record) {
            (FinalizeUploadCommand::Chat { .. }, UploadFinalizeRecord::Chat { upload }) => {
                Ok(UploadFinalizeResult::Chat { upload })
            }
            (
                FinalizeUploadCommand::Topic { .. },
                UploadFinalizeRecord::Topic {
                    upload,
                    topic_media,
                },
            ) => {
                let topic_status = self
                    .dependencies
                    .topics
                    .promote_enriched(transaction, target_id)
                    .await
                    .map_err(|_| MediaError::DatabaseUnavailable)?;
                if topic_status != TopicStatus::Enriched {
                    return Err(MediaError::DatabaseUnavailable);
                }
                Ok(UploadFinalizeResult::Topic {
                    upload,
                    topic_media,
                    topic_status,
                })
            }
            _ => Err(MediaError::DatabaseUnavailable),
        }
    }

    async fn begin(&self) -> Result<BoxTransactionHandle, MediaError> {
        self.dependencies
            .transactions
            .begin()
            .await
            .map_err(|_| MediaError::DatabaseUnavailable)
    }

    async fn finish<T>(
        &self,
        transaction: BoxTransactionHandle,
        result: Result<T, MediaError>,
    ) -> Result<T, MediaError> {
        match result {
            Ok(value) => {
                self.dependencies
                    .transactions
                    .commit(transaction)
                    .await
                    .map_err(|_| MediaError::DatabaseUnavailable)?;
                Ok(value)
            }
            Err(error) => {
                self.dependencies
                    .transactions
                    .rollback(transaction)
                    .await
                    .map_err(|_| MediaError::DatabaseUnavailable)?;
                Err(error)
            }
        }
    }
}

impl MediaAccessService {
    pub fn new(dependencies: MediaAccessDependencies) -> Self {
        Self { dependencies }
    }

    pub async fn view_url(
        &self,
        actor_id: Uuid,
        media_id: Uuid,
    ) -> Result<MediaAccessUrl, MediaError> {
        let media = self.authorize(actor_id, media_id).await?;
        let get = self.presign(&media, None).await?;
        Ok(MediaAccessUrl {
            id: media.id,
            media_upload_id: media.media_upload_id,
            url: get.url,
            content_type: media.content_type,
            byte_size: media.byte_size,
            width: media.width,
            height: media.height,
            duration_seconds: media.duration_seconds,
            filename: media.filename,
            expires_in: get.expires_in,
        })
    }

    pub async fn download_url(
        &self,
        actor_id: Uuid,
        media_id: Uuid,
    ) -> Result<PresignedGet, MediaError> {
        let media = self.authorize(actor_id, media_id).await?;
        let disposition =
            download_content_disposition(media.id, &media.content_type, media.filename.as_deref());
        self.presign(&media, Some(disposition)).await
    }

    async fn authorize(
        &self,
        actor_id: Uuid,
        media_id: Uuid,
    ) -> Result<MediaAccessRecord, MediaError> {
        self.dependencies
            .repository
            .authorize_media_access(&AuthorizeMediaAccessQuery { actor_id, media_id })
            .await
            .map_err(MediaError::from)
    }

    async fn presign(
        &self,
        media: &MediaAccessRecord,
        response_content_disposition: Option<String>,
    ) -> Result<PresignedGet, MediaError> {
        self.dependencies
            .object_storage
            .presign_get(&PresignGetRequest {
                object_key: media.object_key.clone(),
                response_content_disposition,
                expires_in: Duration::from_secs(PRESIGNED_GET_TTL_SECONDS),
            })
            .await
            .map_err(|_| MediaError::ObjectStorageDegraded)
    }
}

fn finalize_result(record: UploadFinalizeRecord) -> Result<UploadFinalizeResult, MediaError> {
    match record {
        UploadFinalizeRecord::Chat { upload } => Ok(UploadFinalizeResult::Chat { upload }),
        UploadFinalizeRecord::Topic {
            upload,
            topic_media,
        } => Ok(UploadFinalizeResult::Topic {
            upload,
            topic_media,
            topic_status: TopicStatus::Enriched,
        }),
    }
}

fn presign_request(upload: &UploadIntentRecord, expires_in: Duration) -> PresignPutRequest {
    PresignPutRequest {
        object_key: upload.object_key.clone(),
        content_type: upload.content_type.clone(),
        byte_size: upload.byte_size,
        expires_in,
    }
}

fn scope_name(scope: MediaScope) -> &'static str {
    match scope {
        MediaScope::Chat => "chat",
        MediaScope::Topic => "topic",
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UploadIntentCreateInput {
    pub scope: crate::domain::media::MediaScope,
    pub target_id: Uuid,
    pub content_type: String,
    pub byte_size: u64,
    pub filename: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UploadIntentWithPresignedPut {
    pub upload: crate::ports::media::UploadIntentRecord,
    pub put: crate::ports::object_storage::PresignedPut,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct UploadFinalizeInput {
    pub width: Option<u32>,
    pub height: Option<u32>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UploadFinalizeResult {
    Chat {
        upload: ConfirmedUploadRecord,
    },
    Topic {
        upload: ConfirmedUploadRecord,
        topic_media: TopicMediaBindingRecord,
        topic_status: TopicStatus,
    },
}

/// Public-safe metadata plus one short viewing URL. The private object key is omitted.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MediaAccessUrl {
    pub id: Uuid,
    pub media_upload_id: Uuid,
    pub url: String,
    pub content_type: String,
    pub byte_size: u64,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub duration_seconds: Option<u64>,
    pub filename: Option<String>,
    pub expires_in: Duration,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MediaError {
    RequestValidation,
    RateLimited { retry_after: Duration },
    RateLimitUnavailable,
    TargetNotAccessible,
    FinalizeConflict,
    FinalizeValidation,
    DatabaseUnavailable,
    ObjectStorageDegraded,
    InvalidConfiguration,
}

impl fmt::Display for MediaError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("media operation failed")
    }
}

impl std::error::Error for MediaError {}

impl From<MediaRepositoryError> for MediaError {
    fn from(error: MediaRepositoryError) -> Self {
        match error {
            MediaRepositoryError::TargetNotAccessible => Self::TargetNotAccessible,
            MediaRepositoryError::FinalizeConflict => Self::FinalizeConflict,
            MediaRepositoryError::InvalidData | MediaRepositoryError::Unavailable => {
                Self::DatabaseUnavailable
            }
        }
    }
}
