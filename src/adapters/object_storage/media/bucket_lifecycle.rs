use std::fmt;

use crate::ports::object_storage::{
    BucketHeadError, BucketLifecycleBackend, ObjectStorageProviderError,
};

#[derive(Clone)]
pub struct BucketLifecycle<Backend> {
    backend: Backend,
    bucket: String,
}

impl<Backend> BucketLifecycle<Backend>
where
    Backend: BucketLifecycleBackend,
{
    pub fn new(backend: Backend, bucket: impl Into<String>) -> Self {
        Self {
            backend,
            bucket: bucket.into(),
        }
    }

    pub async fn ensure_bucket(&self) -> Result<BucketEnsureOutcome, BucketEnsureError> {
        match self.backend.head_bucket(&self.bucket).await {
            Ok(()) => Ok(BucketEnsureOutcome::Existing),
            Err(BucketHeadError::Missing) => self
                .backend
                .create_bucket(&self.bucket)
                .await
                .map(|()| BucketEnsureOutcome::Created)
                .map_err(BucketEnsureError::Provider),
            Err(BucketHeadError::Provider(error)) => Err(BucketEnsureError::Provider(error)),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BucketEnsureOutcome {
    Existing,
    Created,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BucketEnsureError {
    Provider(ObjectStorageProviderError),
}

impl fmt::Display for BucketEnsureError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("private object-storage bucket is unavailable")
    }
}

impl std::error::Error for BucketEnsureError {}
