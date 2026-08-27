use std::{
    collections::VecDeque,
    io,
    sync::{Arc, Mutex},
};

use axum::{
    Router,
    body::Body,
    extract::{Request, State},
    http::{Method, StatusCode},
    response::Response,
};
use jamye_server::{
    adapters::object_storage::media::{
        BucketEnsureError, BucketEnsureOutcome, BucketLifecycle, S3BucketBackend,
    },
    config::{
        AppEnvironment,
        object_storage::{ObjectStorageConfig, ObjectStorageConfigInput},
    },
    ports::object_storage::ObjectStorageProviderError,
};
use tokio::{net::TcpListener, sync::oneshot, task::JoinHandle};

use crate::TestResult;

const PRIVATE_BUCKET: &str = "jamye-private-media";
const ACCESS_KEY: &str = "task-8-sdk-access-key";
const SECRET_KEY: &str = "task-8-sdk-secret-key";

#[tokio::test]
async fn sdk_head_success_is_a_signed_path_style_noop() -> TestResult {
    let server = ScriptedS3::start([StatusCode::OK]).await?;
    let lifecycle = lifecycle(server.endpoint())?;

    assert_eq!(
        lifecycle.ensure_bucket().await,
        Ok(BucketEnsureOutcome::Existing)
    );
    drop(lifecycle);
    assert_eq!(
        server.finish().await?,
        vec![ExpectedRequest::head(PRIVATE_BUCKET)]
    );
    Ok(())
}

#[tokio::test]
async fn sdk_exact_404_creates_the_private_bucket_once() -> TestResult {
    let server = ScriptedS3::start([StatusCode::NOT_FOUND, StatusCode::OK]).await?;
    let lifecycle = lifecycle(server.endpoint())?;

    assert_eq!(
        lifecycle.ensure_bucket().await,
        Ok(BucketEnsureOutcome::Created)
    );
    drop(lifecycle);
    assert_eq!(
        server.finish().await?,
        vec![
            ExpectedRequest::head(PRIVATE_BUCKET),
            ExpectedRequest::create(PRIVATE_BUCKET),
        ]
    );
    Ok(())
}

#[tokio::test]
async fn sdk_403_is_typed_access_denied_and_never_creates() -> TestResult {
    let server = ScriptedS3::start([StatusCode::FORBIDDEN]).await?;
    let lifecycle = lifecycle(server.endpoint())?;

    assert_eq!(
        lifecycle.ensure_bucket().await,
        Err(BucketEnsureError::Provider(
            ObjectStorageProviderError::AccessDenied
        ))
    );
    drop(lifecycle);
    assert_eq!(
        server.finish().await?,
        vec![ExpectedRequest::head(PRIVATE_BUCKET)]
    );
    Ok(())
}

fn lifecycle(endpoint: &str) -> TestResult<BucketLifecycle<S3BucketBackend>> {
    let config = ObjectStorageConfig::resolve(
        AppEnvironment::Test,
        ObjectStorageConfigInput {
            endpoint: Some(endpoint.to_owned()),
            public_endpoint: Some(endpoint.to_owned()),
            region: Some("us-east-1".to_owned()),
            bucket: Some(PRIVATE_BUCKET.to_owned()),
            access_key_id: Some(ACCESS_KEY.to_owned()),
            secret_access_key: Some(SECRET_KEY.to_owned()),
        },
    )?
    .ok_or_else(|| io::Error::other("complete test object-storage config was absent"))?;

    Ok(BucketLifecycle::new(
        S3BucketBackend::new(&config),
        config.bucket(),
    ))
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ExpectedRequest {
    method: Method,
    path: String,
    signed: bool,
    secret_exposed: bool,
}

impl ExpectedRequest {
    fn head(bucket: &str) -> Self {
        Self::new(Method::HEAD, bucket)
    }

    fn create(bucket: &str) -> Self {
        Self::new(Method::PUT, bucket)
    }

    fn new(method: Method, bucket: &str) -> Self {
        Self {
            method,
            path: format!("/{bucket}/"),
            signed: true,
            secret_exposed: false,
        }
    }
}

#[derive(Clone)]
struct ScriptState {
    statuses: Arc<Mutex<VecDeque<StatusCode>>>,
    requests: Arc<Mutex<Vec<ExpectedRequest>>>,
}

struct ScriptedS3 {
    endpoint: String,
    state: ScriptState,
    shutdown: oneshot::Sender<()>,
    task: JoinHandle<Result<(), io::Error>>,
}

impl ScriptedS3 {
    async fn start(statuses: impl IntoIterator<Item = StatusCode>) -> TestResult<Self> {
        let state = ScriptState {
            statuses: Arc::new(Mutex::new(statuses.into_iter().collect())),
            requests: Arc::new(Mutex::new(Vec::new())),
        };
        let app = Router::new()
            .fallback(handle_request)
            .with_state(state.clone());
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let address = listener.local_addr()?;
        let (shutdown, shutdown_receiver) = oneshot::channel();
        let task = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    let _ = shutdown_receiver.await;
                })
                .await
        });

        Ok(Self {
            endpoint: format!("http://{address}"),
            state,
            shutdown,
            task,
        })
    }

    fn endpoint(&self) -> &str {
        &self.endpoint
    }

    async fn finish(self) -> TestResult<Vec<ExpectedRequest>> {
        self.shutdown
            .send(())
            .map_err(|_| io::Error::other("scripted S3 server stopped before shutdown"))?;
        self.task.await??;

        let remaining = self
            .state
            .statuses
            .lock()
            .map_err(|_| io::Error::other("scripted S3 status mutex is poisoned"))?;
        if !remaining.is_empty() {
            return Err(io::Error::other("scripted S3 did not receive every request").into());
        }
        drop(remaining);

        self.state
            .requests
            .lock()
            .map(|requests| requests.clone())
            .map_err(|_| io::Error::other("scripted S3 request mutex is poisoned").into())
    }
}

async fn handle_request(
    State(state): State<ScriptState>,
    request: Request<Body>,
) -> Response<Body> {
    let authorization = request
        .headers()
        .get("authorization")
        .and_then(|value| value.to_str().ok());
    let observed = ExpectedRequest {
        method: request.method().clone(),
        path: request.uri().path().to_owned(),
        signed: authorization.is_some_and(|value| value.starts_with("AWS4-HMAC-SHA256 ")),
        secret_exposed: authorization.is_some_and(|value| value.contains(SECRET_KEY)),
    };

    let recorded = state
        .requests
        .lock()
        .map(|mut requests| requests.push(observed))
        .is_ok();
    let status = state
        .statuses
        .lock()
        .ok()
        .and_then(|mut statuses| statuses.pop_front());

    let status = match (recorded, status) {
        (true, Some(status)) => status,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    let mut response = Response::new(Body::empty());
    *response.status_mut() = status;
    response
}
