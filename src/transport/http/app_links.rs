//! Public app-link association and invite landing routes.

use axum::{
    Json, Router,
    extract::{Path, State},
    http::{
        HeaderValue, StatusCode,
        header::{CACHE_CONTROL, CONTENT_SECURITY_POLICY, CONTENT_TYPE, REFERRER_POLICY},
    },
    response::{Html, IntoResponse, Response},
    routing::get,
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::config::app_links::AppLinksConfig;

const X_CONTENT_TYPE_OPTIONS: &str = "x-content-type-options";
const X_ROBOTS_TAG: &str = "x-robots-tag";
const COPY_SCRIPT: &str = "(() => { const button = document.getElementById('copy-code'); const code = document.getElementById('invite-code'); if (!button || !code || !navigator.clipboard) { return; } button.addEventListener('click', async () => { try { await navigator.clipboard.writeText(code.textContent || ''); button.textContent = '복사됨'; } catch (_) { button.textContent = '복사 실패'; } }); })();";

#[derive(Clone)]
pub struct AppLinksHttpState {
    config: AppLinksConfig,
}

impl AppLinksHttpState {
    pub fn new(config: AppLinksConfig) -> Self {
        Self { config }
    }
}

pub fn router(state: AppLinksHttpState) -> Router {
    Router::new()
        .route(
            "/.well-known/apple-app-site-association",
            get(apple_app_site_association),
        )
        .route("/.well-known/assetlinks.json", get(assetlinks))
        .route("/invite/{code}", get(invite_landing))
        .with_state(state)
}

async fn apple_app_site_association(State(state): State<AppLinksHttpState>) -> Response {
    (
        [(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"))],
        Json(AasaResponse {
            applinks: AasaApplinks {
                details: vec![AasaDetail {
                    app_ids: state.config.aasa_app_ids().to_vec(),
                    components: vec![AasaComponent { path: "/invite/*" }],
                }],
            },
        }),
    )
        .into_response()
}

async fn assetlinks(State(state): State<AppLinksHttpState>) -> Response {
    (
        [(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"))],
        Json(vec![AssetLinksStatement {
            relation: vec!["delegate_permission/common.handle_all_urls"],
            target: AssetLinksTarget {
                namespace: "android_app",
                package_name: state.config.android_package(),
                sha256_cert_fingerprints: state.config.android_sha256_cert_fingerprints(),
            },
        }]),
    )
        .into_response()
}

async fn invite_landing(
    State(state): State<AppLinksHttpState>,
    Path(code): Path<String>,
) -> Response {
    if !valid_invite_code(&code) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let csp = format!(
        "default-src 'none'; style-src 'unsafe-inline'; script-src 'sha256-{}'; img-src 'self'; base-uri 'none'; form-action 'none'",
        script_hash()
    );
    let mut response = Html(invite_html(&state.config, &code)).into_response();
    let headers = response.headers_mut();
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    headers.insert(X_ROBOTS_TAG, HeaderValue::from_static("noindex"));
    if let Ok(value) = HeaderValue::from_str(&csp) {
        headers.insert(CONTENT_SECURITY_POLICY, value);
    }
    response
}

fn valid_invite_code(code: &str) -> bool {
    (16..=64).contains(&code.len())
        && code
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

fn invite_html(config: &AppLinksConfig, code: &str) -> String {
    let escaped_code = escape_html(code);
    let invite_url = format!("jamye://invite/{}", escape_attribute(code));
    let store_links = match (config.app_store_url(), config.play_store_url()) {
        (None, None) => "<p class=\"pending\">출시 준비 중</p>".to_owned(),
        (app_store, play_store) => {
            let mut links = String::new();
            if let Some(url) = app_store {
                links.push_str(&format!(
                    "<a class=\"store\" href=\"{}\" rel=\"noopener noreferrer\">App Store</a>",
                    escape_attribute(url)
                ));
            }
            if let Some(url) = play_store {
                links.push_str(&format!(
                    "<a class=\"store\" href=\"{}\" rel=\"noopener noreferrer\">Google Play</a>",
                    escape_attribute(url)
                ));
            }
            links
        }
    };
    format!(
        r#"<!doctype html>
<html lang="ko">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Jamye 초대</title>
  <style>
    :root {{ color-scheme: light dark; font-family: system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif; }}
    body {{ margin: 0; min-height: 100vh; display: grid; place-items: center; background: Canvas; color: CanvasText; }}
    main {{ width: min(100% - 32px, 420px); padding: 32px 0; }}
    h1 {{ font-size: 1.6rem; margin: 0 0 12px; }}
    p {{ line-height: 1.55; margin: 10px 0; }}
    .code {{ display: flex; align-items: center; justify-content: space-between; gap: 12px; border: 1px solid color-mix(in srgb, CanvasText 18%, transparent); border-radius: 12px; padding: 14px; margin: 20px 0; }}
    code {{ overflow-wrap: anywhere; font-size: 1rem; }}
    a, button {{ border: 0; border-radius: 999px; display: inline-flex; align-items: center; justify-content: center; min-height: 44px; padding: 0 18px; font: inherit; text-decoration: none; }}
    button {{ background: ButtonFace; color: ButtonText; }}
    .primary {{ width: 100%; background: CanvasText; color: Canvas; margin: 8px 0 12px; }}
    .stores {{ display: flex; flex-wrap: wrap; gap: 8px; margin-top: 8px; }}
    .store {{ border: 1px solid color-mix(in srgb, CanvasText 22%, transparent); color: CanvasText; }}
    .pending {{ color: color-mix(in srgb, CanvasText 68%, transparent); }}
  </style>
</head>
<body>
  <main>
    <h1>Jamye 초대</h1>
    <p>앱이 설치되어 있으면 앱에서 초대 확인 화면이 열립니다.</p>
    <div class="code"><code id="invite-code">{escaped_code}</code><button id="copy-code" type="button">복사</button></div>
    <a class="primary" href="{invite_url}">앱 열기</a>
    <div class="stores">{store_links}</div>
  </main>
  <script>{COPY_SCRIPT}</script>
</body>
</html>"#
    )
}

fn script_hash() -> String {
    let digest = Sha256::digest(COPY_SCRIPT.as_bytes());
    STANDARD.encode(digest)
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn escape_attribute(value: &str) -> String {
    escape_html(value)
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[derive(Serialize)]
struct AasaResponse {
    applinks: AasaApplinks,
}

#[derive(Serialize)]
struct AasaApplinks {
    details: Vec<AasaDetail>,
}

#[derive(Serialize)]
struct AasaDetail {
    #[serde(rename = "appIDs")]
    app_ids: Vec<String>,
    components: Vec<AasaComponent>,
}

#[derive(Serialize)]
struct AasaComponent {
    #[serde(rename = "/")]
    path: &'static str,
}

#[derive(Serialize)]
struct AssetLinksStatement<'a> {
    relation: Vec<&'static str>,
    target: AssetLinksTarget<'a>,
}

#[derive(Serialize)]
struct AssetLinksTarget<'a> {
    namespace: &'static str,
    package_name: &'a str,
    sha256_cert_fingerprints: &'a [String],
}
