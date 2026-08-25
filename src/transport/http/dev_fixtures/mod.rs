//! Feature-local development fixture router.

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::post,
};
use uuid::Uuid;

use crate::{
    dev_fixtures::{
        DevFixtureGuard, DevFixtureInitError, DevFixtureService, DevFixtureServiceError,
        DevFixtureStore, DevTokenCodec, SeededFixture,
    },
    transport::http::auth::{AuthVerifierState, error_response, request_id},
};

const SEED_PATH: &str = "/__dev/fixtures/seed";

#[derive(Clone)]
pub struct DevFixtureHttpState {
    service: Arc<DevFixtureService>,
}

impl DevFixtureHttpState {
    pub fn from_env(store: Arc<dyn DevFixtureStore>) -> Result<Self, DevFixtureInitError> {
        let guard = DevFixtureGuard::from_env()?;
        let codec = DevTokenCodec::ephemeral(guard);
        Ok(Self::new(store, codec))
    }

    pub fn new(store: Arc<dyn DevFixtureStore>, codec: DevTokenCodec) -> Self {
        Self {
            service: Arc::new(DevFixtureService::new(store, codec)),
        }
    }

    pub fn auth_state(&self) -> AuthVerifierState {
        AuthVerifierState::new(self.service.verifier())
    }
}

pub fn router(state: DevFixtureHttpState) -> Router {
    Router::new()
        .route(SEED_PATH, post(seed_fixture))
        .with_state(state)
}

async fn seed_fixture(
    State(state): State<DevFixtureHttpState>,
    request: Request,
) -> Result<(StatusCode, Json<SeededFixture>), DevFixtureHttpError> {
    let (parts, _) = request.into_parts();
    let request_id = request_id(&parts);
    state
        .service
        .seed()
        .await
        .map(|fixture| (StatusCode::CREATED, Json(fixture)))
        .map_err(|error| DevFixtureHttpError { error, request_id })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DevFixtureHttpError {
    error: DevFixtureServiceError,
    request_id: Uuid,
}

impl IntoResponse for DevFixtureHttpError {
    fn into_response(self) -> Response {
        let (status, code, message) = match self.error {
            DevFixtureServiceError::Store => (
                StatusCode::SERVICE_UNAVAILABLE,
                "database_unavailable",
                "데이터베이스를 사용할 수 없습니다.",
            ),
            DevFixtureServiceError::Token => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "요청을 처리할 수 없습니다.",
            ),
        };
        error_response(status, code, message, self.request_id)
    }
}
