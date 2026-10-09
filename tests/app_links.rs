//! Server task-20a invite landing copy: the user-facing product name is 잼얘좀.

use std::error::Error;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use jamye_server::{
    config::app_links::{AppLinksConfig, AppLinksConfigInput},
    transport::http::app_links::{AppLinksHttpState, router as app_links_router},
};
use tower::ServiceExt;

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

#[tokio::test]
async fn invite_landing_uses_the_korean_product_name_in_title_and_copy() -> TestResult {
    let config = AppLinksConfig::try_from(AppLinksConfigInput::default())?;
    let app = app_links_router(AppLinksHttpState::new(config));
    let response = app
        .oneshot(Request::get("/invite/Abcdefghijklmnop_123").body(Body::empty())?)
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body = String::from_utf8(to_bytes(response.into_body(), 128 * 1024).await?.to_vec())?;
    assert!(body.contains("<title>잼얘좀 초대</title>"));
    assert!(body.contains("<h1>잼얘좀 초대</h1>"));
    assert!(
        !body.contains("Jamye"),
        "user-facing copy must not use the old Latin product name"
    );
    Ok(())
}
