use std::{
    io,
    sync::{Arc, Mutex},
};

use jamye_server::{
    adapters::object_storage::media::{BucketEnsureError, BucketEnsureOutcome, BucketLifecycle},
    ports::object_storage::{
        BucketCreateFuture, BucketHeadError, BucketHeadFuture, BucketLifecycleBackend,
        ObjectStorageProviderError,
    },
};

use crate::TestResult;

const PRIVATE_BUCKET: &str = "jamye-private-media";

#[tokio::test]
async fn head_success_is_a_noop_without_create() -> TestResult {
    let backend = FakeBucketBackend::new(Ok(()), Ok(()));
    let lifecycle = BucketLifecycle::new(backend.clone(), PRIVATE_BUCKET);

    assert_eq!(
        lifecycle.ensure_bucket().await,
        Ok(BucketEnsureOutcome::Existing)
    );
    assert_eq!(backend.calls()?, vec![BucketCall::Head]);
    Ok(())
}

#[tokio::test]
async fn exact_missing_bucket_creates_once() -> TestResult {
    let backend = FakeBucketBackend::new(Err(BucketHeadError::Missing), Ok(()));
    let lifecycle = BucketLifecycle::new(backend.clone(), PRIVATE_BUCKET);

    assert_eq!(
        lifecycle.ensure_bucket().await,
        Ok(BucketEnsureOutcome::Created)
    );
    assert_eq!(backend.calls()?, vec![BucketCall::Head, BucketCall::Create]);
    Ok(())
}

#[tokio::test]
async fn non_missing_head_failure_is_preserved_and_never_creates() -> TestResult {
    let provider_error = ObjectStorageProviderError::AccessDenied;
    let backend = FakeBucketBackend::new(Err(BucketHeadError::Provider(provider_error)), Ok(()));
    let lifecycle = BucketLifecycle::new(backend.clone(), PRIVATE_BUCKET);

    assert_eq!(
        lifecycle.ensure_bucket().await,
        Err(BucketEnsureError::Provider(provider_error))
    );
    assert_eq!(backend.calls()?, vec![BucketCall::Head]);
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BucketCall {
    Head,
    Create,
}

#[derive(Clone)]
struct FakeBucketBackend {
    head_result: Result<(), BucketHeadError>,
    create_result: Result<(), ObjectStorageProviderError>,
    calls: Arc<Mutex<Vec<BucketCall>>>,
}

impl FakeBucketBackend {
    fn new(
        head_result: Result<(), BucketHeadError>,
        create_result: Result<(), ObjectStorageProviderError>,
    ) -> Self {
        Self {
            head_result,
            create_result,
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn record(&self, call: BucketCall) -> Result<(), ObjectStorageProviderError> {
        let mut calls = self
            .calls
            .lock()
            .map_err(|_| ObjectStorageProviderError::Unavailable)?;
        calls.push(call);
        Ok(())
    }

    fn calls(&self) -> TestResult<Vec<BucketCall>> {
        self.calls
            .lock()
            .map(|calls| calls.clone())
            .map_err(|_| io::Error::other("bucket call log mutex is poisoned").into())
    }
}

impl BucketLifecycleBackend for FakeBucketBackend {
    fn head_bucket<'a>(&'a self, bucket: &'a str) -> BucketHeadFuture<'a> {
        Box::pin(async move {
            if bucket != PRIVATE_BUCKET {
                return Err(BucketHeadError::Provider(
                    ObjectStorageProviderError::UnexpectedResponse,
                ));
            }
            self.record(BucketCall::Head)
                .map_err(BucketHeadError::Provider)?;
            self.head_result
        })
    }

    fn create_bucket<'a>(&'a self, bucket: &'a str) -> BucketCreateFuture<'a> {
        Box::pin(async move {
            if bucket != PRIVATE_BUCKET {
                return Err(ObjectStorageProviderError::UnexpectedResponse);
            }
            self.record(BucketCall::Create)?;
            self.create_result
        })
    }
}
