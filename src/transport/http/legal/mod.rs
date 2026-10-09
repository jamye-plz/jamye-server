//! Public legal pages: `/privacy`, `/terms`, `/account-deletion` and `/support`.
//!
//! The pages are rendered once at startup from `docs/legal/*.ko.html` and the validated
//! operator values, then served as pre-rendered bytes. They are static documents, so there
//! is no service or repository layer and no OpenAPI contract entry (like the app-link routes).

mod render;

use std::sync::Arc;

use axum::{
    Router,
    body::Bytes,
    extract::State,
    http::{
        HeaderValue,
        header::{CACHE_CONTROL, CONTENT_SECURITY_POLICY, CONTENT_TYPE, REFERRER_POLICY},
    },
    response::{Html, IntoResponse, Response},
    routing::get,
};

pub use render::{LegalPage, LegalPages, LegalRenderError, render_body};

use crate::config::legal::LegalConfig;

const X_CONTENT_TYPE_OPTIONS: &str = "x-content-type-options";
const CONTENT_SECURITY_POLICY_VALUE: &str = "default-src 'none'; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

#[derive(Clone)]
pub struct LegalHttpState {
    pages: Arc<LegalPages>,
}

impl LegalHttpState {
    /// Renders every page; fails when a template placeholder cannot be resolved.
    pub fn new(config: &LegalConfig) -> Result<Self, LegalRenderError> {
        Ok(Self {
            pages: Arc::new(LegalPages::render(config)?),
        })
    }
}

pub fn router(state: LegalHttpState) -> Router {
    Router::new()
        .route("/privacy", get(privacy))
        .route("/terms", get(terms))
        .route("/account-deletion", get(account_deletion))
        .route("/support", get(support))
        .with_state(state)
}

async fn privacy(State(state): State<LegalHttpState>) -> Response {
    respond(state.pages.page(LegalPage::Privacy))
}

async fn terms(State(state): State<LegalHttpState>) -> Response {
    respond(state.pages.page(LegalPage::Terms))
}

async fn account_deletion(State(state): State<LegalHttpState>) -> Response {
    respond(state.pages.page(LegalPage::AccountDeletion))
}

async fn support(State(state): State<LegalHttpState>) -> Response {
    respond(state.pages.page(LegalPage::Support))
}

fn respond(body: Bytes) -> Response {
    let mut response = Html(body).into_response();
    let headers = response.headers_mut();
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    headers.insert(
        CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=300"),
    );
    headers.insert(REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    headers.insert(
        CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CONTENT_SECURITY_POLICY_VALUE),
    );
    response
}
