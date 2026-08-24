//! Explicit M0 API composition root.

use std::{error::Error, fmt, iter::once, sync::Arc};

use axum::{
    Router,
    body::Body,
    extract::{MatchedPath, Request},
    http::{HeaderName, header::AUTHORIZATION},
    middleware,
};
use tower::ServiceBuilder;
use tower_http::{
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, RequestId, SetRequestIdLayer},
    sensitive_headers::SetSensitiveRequestHeadersLayer,
    trace::{DefaultOnResponse, TraceLayer},
};
use tracing::{Level, Span};

use crate::{
    adapters::{
        object_storage::health::MinioHealthProbe, postgres::health::PostgresHealthProbe,
        redis::health::RedisHealthProbe,
    },
    config::AppConfig,
    platform::{
        readiness::{DependencyProbe, ReadinessService, UnconfiguredProbe},
        request_id::{REQUEST_ID_HEADER, strip_external_request_id},
    },
    transport::http::health::{self, HealthState},
};

pub fn router(config: &AppConfig) -> Result<Router, CompositionError> {
    let timeout = config.readiness_timeout();
    let postgres: Arc<dyn DependencyProbe> = Arc::new(
        PostgresHealthProbe::connect_lazy(config.database_url(), timeout)
            .map_err(|_| CompositionError::Postgres)?,
    );
    let redis = redis_probe(config, timeout)?;
    let minio = minio_probe(config, timeout)?;
    Ok(router_with_readiness(ReadinessService::new(
        postgres, redis, minio,
    )))
}

pub fn router_with_readiness(readiness: ReadinessService) -> Router {
    let request_id_header = HeaderName::from_static(REQUEST_ID_HEADER);
    let middleware = ServiceBuilder::new()
        .layer(middleware::from_fn(strip_external_request_id))
        .layer(SetSensitiveRequestHeadersLayer::new(once(AUTHORIZATION)))
        .layer(SetRequestIdLayer::new(
            request_id_header.clone(),
            MakeRequestUuid,
        ))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(request_span)
                .on_response(DefaultOnResponse::new().level(Level::INFO)),
        )
        .layer(PropagateRequestIdLayer::new(request_id_header));
    health::router(HealthState::new(readiness)).layer(middleware)
}

fn redis_probe(
    config: &AppConfig,
    timeout: std::time::Duration,
) -> Result<Arc<dyn DependencyProbe>, CompositionError> {
    match config.redis_url() {
        Some(url) => RedisHealthProbe::new(url, timeout)
            .map(|probe| Arc::new(probe) as Arc<dyn DependencyProbe>)
            .map_err(|_| CompositionError::Redis),
        None => Ok(Arc::new(UnconfiguredProbe)),
    }
}

fn minio_probe(
    config: &AppConfig,
    timeout: std::time::Duration,
) -> Result<Arc<dyn DependencyProbe>, CompositionError> {
    match config.minio_health_url() {
        Some(url) => MinioHealthProbe::new(url, timeout)
            .map(|probe| Arc::new(probe) as Arc<dyn DependencyProbe>)
            .map_err(|_| CompositionError::Minio),
        None => Ok(Arc::new(UnconfiguredProbe)),
    }
}

fn request_span(request: &Request<Body>) -> Span {
    let request_id = request
        .extensions()
        .get::<RequestId>()
        .and_then(|request_id| request_id.header_value().to_str().ok())
        .unwrap_or("missing");
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map(MatchedPath::as_str)
        .unwrap_or("unmatched");
    tracing::info_span!(
        "http.request",
        request_id = %request_id,
        method = %request.method(),
        route
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompositionError {
    Postgres,
    Redis,
    Minio,
}

impl fmt::Display for CompositionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let component = match self {
            Self::Postgres => "PostgreSQL",
            Self::Redis => "Redis",
            Self::Minio => "MinIO",
        };
        write!(
            formatter,
            "failed to initialize {component} readiness probe"
        )
    }
}

impl Error for CompositionError {}
