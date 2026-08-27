//! Replaceable private object-storage operations.

use std::{fmt, future::Future, pin::Pin};

use crate::domain::media::{InspectedObject, MediaKind};

pub type BucketHeadFuture<'a> =
    Pin<Box<dyn Future<Output = Result<(), BucketHeadError>> + Send + 'a>>;
pub type BucketCreateFuture<'a> =
    Pin<Box<dyn Future<Output = Result<(), ObjectStorageProviderError>> + Send + 'a>>;
pub type MediaObjectStorageFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, ObjectStorageProviderError>> + Send + 'a>>;

/// Minimal provider boundary used by the API-owned bucket lifecycle.
pub trait BucketLifecycleBackend: Send + Sync {
    fn head_bucket<'a>(&'a self, bucket: &'a str) -> BucketHeadFuture<'a>;

    fn create_bucket<'a>(&'a self, bucket: &'a str) -> BucketCreateFuture<'a>;
}

/// Private-object operations used by media application services.
pub trait MediaObjectStorage: Send + Sync {
    fn presign_put<'a>(
        &'a self,
        request: &'a PresignPutRequest,
    ) -> MediaObjectStorageFuture<'a, PresignedPut>;

    /// Inspect authoritative provider metadata and, for audio, the actual container duration.
    fn inspect_object<'a>(
        &'a self,
        request: &'a InspectObjectRequest,
    ) -> MediaObjectStorageFuture<'a, InspectedObject>;

    /// Presign a short private-object GET using only server-authorized inputs.
    fn presign_get<'a>(
        &'a self,
        _request: &'a PresignGetRequest,
    ) -> MediaObjectStorageFuture<'a, PresignedGet> {
        Box::pin(async { Err(ObjectStorageProviderError::Unavailable) })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PresignPutRequest {
    pub object_key: String,
    pub content_type: String,
    pub byte_size: u64,
    pub expires_in: std::time::Duration,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PresignedPut {
    pub url: String,
    pub expires_in: std::time::Duration,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InspectObjectRequest {
    pub object_key: String,
    pub kind: MediaKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PresignGetRequest {
    pub object_key: String,
    pub response_content_disposition: Option<String>,
    pub expires_in: std::time::Duration,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PresignedGet {
    pub url: String,
    pub expires_in: std::time::Duration,
}

/// A bucket probe distinguishes the one creation case from provider failures.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BucketHeadError {
    Missing,
    Provider(ObjectStorageProviderError),
}

/// Provider failures stay typed internally and map to storage-only degradation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ObjectStorageProviderError {
    AccessDenied,
    Unavailable,
    UnexpectedResponse,
}

impl fmt::Display for ObjectStorageProviderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("object-storage provider operation failed")
    }
}

impl std::error::Error for ObjectStorageProviderError {}
