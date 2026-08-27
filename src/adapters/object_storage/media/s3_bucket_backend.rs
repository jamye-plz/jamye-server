use aws_sdk_s3::{
    Client,
    config::{Credentials, Region},
    error::{ProvideErrorMetadata, SdkError},
    operation::head_bucket::HeadBucketError as S3HeadBucketError,
};

use crate::{
    config::object_storage::ObjectStorageConfig,
    ports::object_storage::{
        BucketCreateFuture, BucketHeadError, BucketHeadFuture, BucketLifecycleBackend,
        ObjectStorageProviderError,
    },
};

/// AWS S3-compatible bucket operations backed by explicit feature-local settings.
#[derive(Clone)]
pub struct S3BucketBackend {
    client: Client,
}

impl S3BucketBackend {
    pub fn new(config: &ObjectStorageConfig) -> Self {
        Self {
            client: s3_client(config, config.endpoint()),
        }
    }
}

impl BucketLifecycleBackend for S3BucketBackend {
    fn head_bucket<'a>(&'a self, bucket: &'a str) -> BucketHeadFuture<'a> {
        Box::pin(async move {
            self.client
                .head_bucket()
                .bucket(bucket)
                .send()
                .await
                .map(|_| ())
                .map_err(classify_head_error)
        })
    }

    fn create_bucket<'a>(&'a self, bucket: &'a str) -> BucketCreateFuture<'a> {
        Box::pin(async move {
            self.client
                .create_bucket()
                .bucket(bucket)
                .send()
                .await
                .map(|_| ())
                .map_err(|error| classify_provider_error(&error))
        })
    }
}

fn classify_head_error(error: SdkError<S3HeadBucketError>) -> BucketHeadError {
    let status = response_status(&error);
    let code = error
        .as_service_error()
        .and_then(ProvideErrorMetadata::code);

    if status == Some(404) || code == Some("NoSuchBucket") {
        BucketHeadError::Missing
    } else {
        BucketHeadError::Provider(classify_provider_error(&error))
    }
}

pub(super) fn s3_client(config: &ObjectStorageConfig, endpoint: &str) -> Client {
    let credentials = Credentials::new(
        config.access_key_id().to_owned(),
        config.secret_access_key().to_owned(),
        None,
        None,
        "jamye-object-storage",
    );
    let sdk_config = aws_sdk_s3::Config::builder()
        .behavior_version_latest()
        .region(Region::new(config.region().to_owned()))
        .credentials_provider(credentials)
        .endpoint_url(endpoint)
        .force_path_style(true)
        .build();

    Client::from_conf(sdk_config)
}

pub(super) fn classify_provider_error<Error>(error: &SdkError<Error>) -> ObjectStorageProviderError
where
    Error: ProvideErrorMetadata,
{
    let status = response_status(error);
    let code = error
        .as_service_error()
        .and_then(ProvideErrorMetadata::code);

    if status == Some(403) || code == Some("AccessDenied") {
        ObjectStorageProviderError::AccessDenied
    } else {
        match error {
            SdkError::TimeoutError(_) | SdkError::DispatchFailure(_) => {
                ObjectStorageProviderError::Unavailable
            }
            _ => ObjectStorageProviderError::UnexpectedResponse,
        }
    }
}

fn response_status<Error>(error: &SdkError<Error>) -> Option<u16> {
    error
        .raw_response()
        .map(|response| response.status().as_u16())
}
