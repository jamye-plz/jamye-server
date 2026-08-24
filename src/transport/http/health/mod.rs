//! Liveness and readiness HTTP endpoints.

use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
};
use schemars::JsonSchema;
use serde::Serialize;
use utoipa::ToSchema;

use crate::platform::readiness::{ProbeOutcome, ReadinessReport, ReadinessService};

#[derive(Clone)]
pub struct HealthState {
    readiness: ReadinessService,
}

impl HealthState {
    pub fn new(readiness: ReadinessService) -> Self {
        Self { readiness }
    }
}

pub fn router(state: HealthState) -> Router {
    Router::new()
        .route("/health/live", get(live))
        .route("/health/ready", get(ready))
        .with_state(state)
}

#[utoipa::path(
    get,
    path = "/health/live",
    responses((status = 200, description = "Process is alive", body = LivenessResponse))
)]
async fn live() -> Json<LivenessResponse> {
    Json(LivenessResponse {
        status: LivenessStatus::Live,
    })
}

#[utoipa::path(
    get,
    path = "/health/ready",
    responses(
        (status = 200, description = "Required dependencies are ready", body = ReadinessResponse),
        (status = 503, description = "PostgreSQL is unavailable", body = ReadinessResponse)
    )
)]
async fn ready(State(state): State<HealthState>) -> Response {
    let report = state.readiness.check().await;
    let status = if report.is_ready() {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (status, Json(ReadinessResponse::from(report))).into_response()
}

#[derive(Clone, Copy, Debug, JsonSchema, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum LivenessStatus {
    Live,
}

#[derive(Clone, Copy, Debug, JsonSchema, Serialize, ToSchema)]
pub struct LivenessResponse {
    pub status: LivenessStatus,
}

#[derive(Clone, Copy, Debug, JsonSchema, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReadinessStatus {
    Ready,
    NotReady,
}

#[derive(Clone, Copy, Debug, JsonSchema, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum DependencyStatus {
    Ready,
    Unavailable,
    Degraded,
}

#[derive(Clone, Copy, Debug, JsonSchema, Serialize, ToSchema)]
pub struct DependencyCheck {
    pub status: DependencyStatus,
    pub required: bool,
}

#[derive(Clone, Copy, Debug, JsonSchema, Serialize, ToSchema)]
pub struct DependencyChecks {
    pub postgres: DependencyCheck,
    pub redis: DependencyCheck,
    pub minio: DependencyCheck,
}

#[derive(Clone, Copy, Debug, JsonSchema, Serialize, ToSchema)]
pub struct ReadinessResponse {
    pub status: ReadinessStatus,
    pub checks: DependencyChecks,
}

impl From<ReadinessReport> for ReadinessResponse {
    fn from(report: ReadinessReport) -> Self {
        Self {
            status: if report.is_ready() {
                ReadinessStatus::Ready
            } else {
                ReadinessStatus::NotReady
            },
            checks: DependencyChecks {
                postgres: required_check(report.postgres),
                redis: degradable_check(report.redis),
                minio: degradable_check(report.minio),
            },
        }
    }
}

fn required_check(outcome: ProbeOutcome) -> DependencyCheck {
    DependencyCheck {
        status: match outcome {
            ProbeOutcome::Reachable => DependencyStatus::Ready,
            ProbeOutcome::Unreachable => DependencyStatus::Unavailable,
        },
        required: true,
    }
}

fn degradable_check(outcome: ProbeOutcome) -> DependencyCheck {
    DependencyCheck {
        status: match outcome {
            ProbeOutcome::Reachable => DependencyStatus::Ready,
            ProbeOutcome::Unreachable => DependencyStatus::Degraded,
        },
        required: false,
    }
}
