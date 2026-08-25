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

#[cfg(feature = "dev-fixtures")]
use crate::{
    adapters::{
        postgres::{
            dev_fixtures::PostgresDevFixtureStore, messaging::PostgresMessagingRepository,
            realtime::PostgresRealtimeRepository, runtime_pool,
            transactions::SqlxTransactionManager,
        },
        redis::realtime::{OsTicketCredentialSource, RedisRealtimeAdapter},
    },
    application::{
        messaging::MessagingService,
        realtime::{RealtimeTicketService, SystemClock},
    },
    dev_fixtures::DevFixtureGuard,
    transport::{
        http::{
            dev_fixtures::{DevFixtureHttpState, router as dev_fixture_router},
            messaging::{MessagingHttpState, router as messaging_router},
            realtime::{RealtimeHttpState, router as realtime_router},
        },
        realtime::LocalRealtimeHub,
    },
};

pub fn router(config: &AppConfig) -> Result<Router, CompositionError> {
    let timeout = config.readiness_timeout();
    let postgres: Arc<dyn DependencyProbe> = Arc::new(
        PostgresHealthProbe::connect_lazy(config.database_url(), timeout)
            .map_err(|_| CompositionError::Postgres)?,
    );
    let redis = redis_probe(config, timeout)?;
    let minio = minio_probe(config, timeout)?;
    let readiness = ReadinessService::new(postgres, redis, minio);
    #[cfg(feature = "dev-fixtures")]
    if let Ok(guard) = DevFixtureGuard::from_env() {
        return c1_dev_router(config, readiness, guard);
    }
    Ok(router_with_readiness(readiness))
}

pub fn router_with_readiness(readiness: ReadinessService) -> Router {
    with_platform_layers(health::router(HealthState::new(readiness)))
}

fn with_platform_layers(router: Router) -> Router {
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
    router.layer(middleware)
}

#[cfg(feature = "dev-fixtures")]
fn c1_dev_router(
    config: &AppConfig,
    readiness: ReadinessService,
    guard: DevFixtureGuard,
) -> Result<Router, CompositionError> {
    let pool = runtime_pool(config.database_url(), config.readiness_timeout())
        .map_err(|_| CompositionError::Postgres)?;
    let redis_url = config
        .redis_url()
        .ok_or(CompositionError::RedisNotConfigured)?;
    let redis =
        Arc::new(RedisRealtimeAdapter::new(redis_url).map_err(|_| CompositionError::Redis)?);
    let fixture = DevFixtureHttpState::new(
        Arc::new(PostgresDevFixtureStore::new(pool.clone())),
        crate::dev_fixtures::DevTokenCodec::ephemeral(guard),
    );
    let auth = fixture.auth_state();
    let transactions = Arc::new(SqlxTransactionManager::new(pool.clone()));
    let messaging_repository = Arc::new(PostgresMessagingRepository::new(pool.clone()));
    let messaging = Arc::new(MessagingService::new(transactions, messaging_repository));
    let realtime_repository = Arc::new(PostgresRealtimeRepository::new(pool));
    let tickets = Arc::new(RealtimeTicketService::new(
        redis.clone(),
        Arc::new(OsTicketCredentialSource),
        Arc::new(SystemClock),
    ));
    let hub = LocalRealtimeHub::default();
    spawn_redis_forwarder(redis, hub.clone());

    let application = health::router(HealthState::new(readiness))
        .merge(dev_fixture_router(fixture))
        .merge(messaging_router(MessagingHttpState::new(
            messaging,
            auth.clone(),
        )))
        .merge(realtime_router(RealtimeHttpState::new(
            tickets,
            hub,
            realtime_repository,
            auth,
        )));
    Ok(with_platform_layers(application))
}

#[cfg(feature = "dev-fixtures")]
fn spawn_redis_forwarder(redis: Arc<RedisRealtimeAdapter>, hub: LocalRealtimeHub) {
    tokio::spawn(async move {
        loop {
            let mut subscriber = match redis.event_subscriber().await {
                Ok(subscriber) => subscriber,
                Err(_) => {
                    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                    continue;
                }
            };
            loop {
                match subscriber.next_event().await {
                    Ok(Some(event)) => {
                        let conversation_id = event.conversation_id;
                        if let Ok(payload) = serde_json::to_string(&event) {
                            hub.publish(conversation_id, payload).await;
                        }
                    }
                    Ok(None) | Err(_) => break,
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
    });
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
    RedisNotConfigured,
    Minio,
}

impl fmt::Display for CompositionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let component = match self {
            Self::Postgres => "PostgreSQL",
            Self::Redis | Self::RedisNotConfigured => "Redis",
            Self::Minio => "MinIO",
        };
        write!(
            formatter,
            "failed to initialize {component} readiness probe"
        )
    }
}

impl Error for CompositionError {}
