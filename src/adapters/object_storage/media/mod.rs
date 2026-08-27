//! Private media object-storage adapter.

mod bucket_lifecycle;
mod s3_bucket_backend;
mod s3_media_object_storage;

pub use bucket_lifecycle::{BucketEnsureError, BucketEnsureOutcome, BucketLifecycle};
pub use s3_bucket_backend::S3BucketBackend;
pub use s3_media_object_storage::S3MediaObjectStorage;
