use std::{sync::Arc, time::Duration};

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use jamye_server::{
    platform::readiness::{DependencyProbe, ProbeFuture, ProbeOutcome, ReadinessService},
    transport::http::composition::router_with_readiness,
};
use serde_json::json;
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use crate::{
    messaging_helpers::{TestApp, counts, json_body, send_to},
    postgres_support::TestResult,
};

#[tokio::test]
async fn media_resilience_minio_degradation_does_not_block_postgres_text_messages() -> TestResult {
    let persisted = TestApp::new().await?;
    let app = health_router(persisted.pool.clone()).merge(persisted.router.clone());

    let ready = app
        .clone()
        .oneshot(Request::get("/health/ready").body(Body::empty())?)
        .await?;
    assert_eq!(ready.status(), StatusCode::OK);
    let readiness = json_body(ready).await?;
    assert_eq!(readiness["status"], "ready");
    assert_eq!(readiness["checks"]["postgres"]["status"], "ready");
    assert_eq!(readiness["checks"]["postgres"]["required"], true);
    assert_eq!(readiness["checks"]["minio"]["status"], "degraded");
    assert_eq!(readiness["checks"]["minio"]["required"], false);

    let client_msg_id = Uuid::new_v4();
    let created = send_to(
        &app,
        Some(&persisted.fixture.access_token),
        persisted.fixture.chatroom_id,
        json!({"client_msg_id": client_msg_id, "body": "MinIO 없이 보내는 텍스트"}),
        None,
    )
    .await?;
    assert_eq!(created.status(), StatusCode::CREATED);
    let message = json_body(created).await?;
    assert_eq!(message["client_msg_id"], client_msg_id.to_string());
    assert_eq!(message["body"], "MinIO 없이 보내는 텍스트");
    assert_eq!(message["media"], json!([]));
    assert_eq!(counts(&persisted.pool).await?, (1, 1, 1));

    persisted.dispose().await
}

fn health_router(pool: PgPool) -> Router {
    router_with_readiness(ReadinessService::new(
        Arc::new(PoolProbe(pool)),
        Arc::new(FixedProbe(ProbeOutcome::Reachable)),
        Arc::new(FixedProbe(ProbeOutcome::Unreachable)),
    ))
}

struct PoolProbe(PgPool);

impl DependencyProbe for PoolProbe {
    fn check(&self) -> ProbeFuture<'_> {
        Box::pin(async move {
            let query = sqlx::query_scalar::<_, i32>("SELECT 1").fetch_one(&self.0);
            match tokio::time::timeout(Duration::from_millis(750), query).await {
                Ok(Ok(1)) => ProbeOutcome::Reachable,
                _ => ProbeOutcome::Unreachable,
            }
        })
    }
}

struct FixedProbe(ProbeOutcome);

impl DependencyProbe for FixedProbe {
    fn check(&self) -> ProbeFuture<'_> {
        Box::pin(async move { self.0 })
    }
}
