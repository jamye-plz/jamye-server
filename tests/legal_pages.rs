//! Server task-20a public legal pages: /privacy, /terms, /account-deletion, /support.

use std::error::Error;

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{HeaderMap, Method, Request, StatusCode},
};
use jamye_server::{
    config::{
        AppConfig, ConfigError, ConfigInput,
        legal::{LegalConfig, LegalConfigInput},
    },
    transport::http::legal::{LegalHttpState, render_body, router as legal_router},
};
use tower::ServiceExt;

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

const NAME_KEY: &str = "JAMYE_LEGAL_OPERATOR_NAME";
const EMAIL_KEY: &str = "JAMYE_LEGAL_CONTACT_EMAIL";
const ADDRESS_KEY: &str = "JAMYE_LEGAL_OPERATOR_ADDRESS";
const OPERATOR: &str = "Example Operator";
const CONTACT: &str = "support@example.test";
const PAGES: [(&str, &str); 4] = [
    ("/privacy", "개인정보 처리방침"),
    ("/terms", "이용약관"),
    ("/account-deletion", "계정 삭제 안내"),
    ("/support", "문의하기"),
];

fn input(name: Option<&str>, email: Option<&str>, address: Option<&str>) -> LegalConfigInput {
    LegalConfigInput {
        operator_name: name.map(ToOwned::to_owned),
        contact_email: email.map(ToOwned::to_owned),
        operator_address: address.map(ToOwned::to_owned),
    }
}

fn config_for(name: &str, email: &str, address: Option<&str>) -> TestResult<LegalConfig> {
    LegalConfig::resolve(input(Some(name), Some(email), address))?
        .ok_or_else(|| Box::<dyn Error>::from("legal config was not enabled"))
}

fn app(config: &LegalConfig) -> TestResult<Router> {
    Ok(legal_router(LegalHttpState::new(config)?))
}

async fn fetch(
    app: &Router,
    method: Method,
    path: &str,
) -> TestResult<(StatusCode, HeaderMap, String)> {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .body(Body::empty())?,
        )
        .await?;
    let status = response.status();
    let headers = response.headers().clone();
    let body = to_bytes(response.into_body(), 256 * 1024).await?;
    Ok((status, headers, String::from_utf8(body.to_vec())?))
}

fn header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

fn assert_public_headers(headers: &HeaderMap, path: &str) {
    assert_eq!(
        header(headers, "content-type"),
        Some("text/html; charset=utf-8"),
        "{path}"
    );
    assert_eq!(
        header(headers, "cache-control"),
        Some("public, max-age=300"),
        "{path}"
    );
    assert_eq!(
        header(headers, "x-content-type-options"),
        Some("nosniff"),
        "{path}"
    );
    assert_eq!(
        header(headers, "referrer-policy"),
        Some("no-referrer"),
        "{path}"
    );
    let csp = header(headers, "content-security-policy").unwrap_or_default();
    assert!(csp.starts_with("default-src 'none'"), "{path}: {csp}");
    assert!(csp.contains("style-src 'unsafe-inline'"), "{path}: {csp}");
    assert!(!csp.contains("script-src"), "{path}: {csp}");
    assert!(csp.contains("frame-ancestors 'none'"), "{path}: {csp}");
    assert!(csp.contains("base-uri 'none'"), "{path}: {csp}");
}

#[tokio::test]
async fn legal_pages_serve_korean_html_with_public_security_headers() -> TestResult {
    let app = app(&config_for(OPERATOR, CONTACT, Some("Seoul Example-ro 1"))?)?;
    for (path, heading) in PAGES {
        let (status, headers, body) = fetch(&app, Method::GET, path).await?;
        assert_eq!(status, StatusCode::OK, "{path}");
        assert_public_headers(&headers, path);
        assert!(body.starts_with("<!doctype html>"), "{path}");
        assert!(body.contains("<html lang=\"ko\">"), "{path}");
        assert!(body.contains("name=\"viewport\""), "{path}");
        assert!(body.contains(&format!("<title>{heading}")), "{path}");
        assert!(body.contains(OPERATOR), "{path} omitted the operator name");
        assert!(body.contains(CONTACT), "{path} omitted the contact email");
        assert!(!body.contains("{{"), "{path} left a placeholder");
        assert!(!body.contains("<script"), "{path} must not carry scripts");
        assert!(
            !body.contains("src=\"http"),
            "{path} must not load resources"
        );
    }
    Ok(())
}

#[tokio::test]
async fn legal_pages_answer_head_and_reject_other_methods() -> TestResult {
    let app = app(&config_for(OPERATOR, CONTACT, None)?)?;
    for (path, _) in PAGES {
        let (status, headers, body) = fetch(&app, Method::HEAD, path).await?;
        assert_eq!(status, StatusCode::OK, "HEAD {path}");
        assert_public_headers(&headers, path);
        assert!(body.is_empty(), "HEAD {path} must not carry a body");

        let (status, _, _) = fetch(&app, Method::POST, path).await?;
        assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED, "POST {path}");
    }
    let (status, _, _) = fetch(&app, Method::GET, "/privacy/extra").await?;
    assert_eq!(status, StatusCode::NOT_FOUND);
    Ok(())
}

#[tokio::test]
async fn operator_values_are_html_escaped_and_the_address_block_is_optional() -> TestResult {
    let hostile = r#"Tom & Jerry <script>alert("x")</script> 'q'"#;
    let with_address = app(&config_for(
        hostile,
        "legal+ops@example.test",
        Some("Seoul <1> & Co"),
    )?)?;
    for (path, _) in PAGES {
        let (_, _, body) = fetch(&with_address, Method::GET, path).await?;
        assert!(!body.contains("<script"), "{path} reflected raw markup");
        assert!(
            body.contains(
                "Tom &amp; Jerry &lt;script&gt;alert(&quot;x&quot;)&lt;/script&gt; &#39;q&#39;"
            ),
            "{path} did not escape the operator name"
        );
        assert!(!body.contains("{{"), "{path}");
    }
    let (_, _, privacy) = fetch(&with_address, Method::GET, "/privacy").await?;
    assert!(privacy.contains("href=\"mailto:legal+ops@example.test\""));
    assert!(privacy.contains("<li>주소: Seoul &lt;1&gt; &amp; Co</li>"));
    assert!(!privacy.contains("address:start") && !privacy.contains("address:end"));

    let without_address = app(&config_for(OPERATOR, CONTACT, None)?)?;
    let (status, _, privacy) = fetch(&without_address, Method::GET, "/privacy").await?;
    assert_eq!(status, StatusCode::OK);
    assert!(
        !privacy.contains("<li>주소: "),
        "address line was not removed"
    );
    assert!(!privacy.contains("OPERATOR_ADDRESS"));
    assert!(!privacy.contains("address:"));
    assert!(!privacy.contains("{{"));
    assert!(privacy.contains(OPERATOR) && privacy.contains(CONTACT));
    Ok(())
}

#[test]
fn rendering_rejects_unresolved_placeholders_and_unbalanced_address_markers() -> TestResult {
    let config = config_for(OPERATOR, CONTACT, None)?;

    let unknown = render_body("privacy", "<p>{{UNKNOWN_VALUE}}</p>", &config)
        .err()
        .map(|error| error.to_string())
        .ok_or("an unknown placeholder rendered")?;
    assert!(unknown.contains("privacy") && unknown.contains("UNKNOWN_VALUE"));

    let address_outside_block = render_body("privacy", "<p>{{OPERATOR_ADDRESS}}</p>", &config)
        .err()
        .map(|error| error.to_string())
        .ok_or("an unset address placeholder rendered")?;
    assert!(address_outside_block.contains("OPERATOR_ADDRESS"));

    for malformed in [
        "<p>{{OPERATOR_NAME</p>",
        "<!-- address:start --><p>x</p>",
        "<p>x</p><!-- address:end -->",
        "<!-- address:start --><!-- address:start --><!-- address:end -->",
    ] {
        assert!(
            render_body("privacy", malformed, &config).is_err(),
            "{malformed} rendered"
        );
    }

    let rendered = render_body(
        "privacy",
        "<p>{{OPERATOR_NAME}}</p><!-- address:start --><p>{{OPERATOR_ADDRESS}}</p><!-- address:end -->",
        &config,
    )?;
    assert_eq!(rendered, "<p>Example Operator</p>");

    let with_address = config_for(OPERATOR, CONTACT, Some("Seoul Example-ro 1"))?;
    let rendered = render_body(
        "privacy",
        "<p>{{OPERATOR_NAME}}</p><!-- address:start --><p>{{OPERATOR_ADDRESS}}</p><!-- address:end -->",
        &with_address,
    )?;
    assert_eq!(rendered, "<p>Example Operator</p><p>Seoul Example-ro 1</p>");
    Ok(())
}

fn error_key(result: Result<Option<LegalConfig>, ConfigError>) -> Option<&'static str> {
    result.err().map(|error| error.key())
}

#[test]
fn legal_config_is_off_when_unset_and_key_named_errors_otherwise() -> TestResult {
    assert_eq!(LegalConfig::resolve(LegalConfigInput::default())?, None);
    assert_eq!(
        LegalConfig::resolve(input(Some(" "), Some(""), Some("\t")))?,
        None,
        "blank values count as unset"
    );

    let enabled = config_for("  Example Operator ", " support@example.test ", None)?;
    assert_eq!(enabled.operator_name(), OPERATOR);
    assert_eq!(enabled.contact_email(), CONTACT);
    assert_eq!(enabled.operator_address(), None);
    let with_address = config_for(OPERATOR, CONTACT, Some(" Seoul Example-ro 1 "))?;
    assert_eq!(with_address.operator_address(), Some("Seoul Example-ro 1"));

    assert_eq!(
        error_key(LegalConfig::resolve(input(Some(OPERATOR), None, None))),
        Some(EMAIL_KEY)
    );
    assert_eq!(
        error_key(LegalConfig::resolve(input(None, Some(CONTACT), None))),
        Some(NAME_KEY)
    );
    assert_eq!(
        error_key(LegalConfig::resolve(input(None, None, Some("Seoul")))),
        Some(NAME_KEY)
    );

    for malformed in [
        "no-at.example.test",
        "a@b",
        "a b@example.test",
        "a@@example.test",
        "a@exa mple.test",
        "<x>@example.test",
        "a@example..test",
        "a@-example.test",
        "a%b@example.test",
        "a@example.t",
        ".a@example.test",
        "{{x}}@example.test",
        "a@example.test\nBcc: x@example.test",
    ] {
        assert_eq!(
            error_key(LegalConfig::resolve(input(
                Some(OPERATOR),
                Some(malformed),
                None
            ))),
            Some(EMAIL_KEY),
            "{malformed:?} was accepted"
        );
    }

    let too_long = "x".repeat(101);
    for bad_name in ["{{OPERATOR_NAME}}", "line\nbreak", too_long.as_str()] {
        assert_eq!(
            error_key(LegalConfig::resolve(input(
                Some(bad_name),
                Some(CONTACT),
                None
            ))),
            Some(NAME_KEY),
            "{bad_name:?} was accepted"
        );
    }
    assert_eq!(
        error_key(LegalConfig::resolve(input(
            Some(OPERATOR),
            Some(CONTACT),
            Some("Seoul {{OPERATOR_ADDRESS}}")
        ))),
        Some(ADDRESS_KEY)
    );

    let error = LegalConfig::resolve(input(Some("{{secret-looking-value}}"), Some(CONTACT), None))
        .err()
        .ok_or("a placeholder-looking name was accepted")?;
    assert!(!error.to_string().contains("secret-looking-value"));
    Ok(())
}

fn app_config_input(legal: LegalConfigInput) -> ConfigInput {
    ConfigInput {
        environment: Some("test".to_owned()),
        database_url: Some("postgres://127.0.0.1/jamye_test".to_owned()),
        legal,
        ..ConfigInput::default()
    }
}

#[test]
fn app_config_carries_validated_legal_values_and_fails_startup_on_a_partial_set() -> TestResult {
    let dark = AppConfig::try_from(app_config_input(LegalConfigInput::default()))?;
    assert!(dark.legal().is_none());

    let lit = AppConfig::try_from(app_config_input(input(Some(OPERATOR), Some(CONTACT), None)))?;
    let legal = lit.legal().ok_or("legal values were not carried")?;
    assert_eq!(legal.operator_name(), OPERATOR);

    for (partial, key) in [
        (input(Some(OPERATOR), None, None), EMAIL_KEY),
        (input(None, Some(CONTACT), None), NAME_KEY),
        (input(Some(OPERATOR), Some("not-an-email"), None), EMAIL_KEY),
    ] {
        let error = AppConfig::try_from(app_config_input(partial))
            .err()
            .ok_or("a partial or malformed legal set passed startup")?;
        assert_eq!(error.key(), key);
    }
    Ok(())
}
