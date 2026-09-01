use std::{
    collections::VecDeque,
    io,
    sync::{Arc, Mutex},
};

use axum::{
    Router,
    body::{Body, to_bytes},
    extract::{Request, State},
    http::{HeaderMap, HeaderName, Method, StatusCode, header},
    response::Response,
};
use jamye_server::{
    adapters::push::expo::{ExpoPushProvider, ExpoPushProviderConfigError},
    platform::logging::build_json_subscriber,
    ports::push::{
        ExpoPushDestination, NotificationType, PushEnvironment, PushProvider, PushProviderError,
        PushProviderOutcome, PushProviderRequest, PushTapPayload,
    },
};
use serde_json::{Value, json};
use tokio::{net::TcpListener, sync::oneshot, task::JoinHandle};
use uuid::Uuid;

use crate::{TestResult, logging_support::SharedWriter};

const SEND_PATH: &str = "/--/api/v2/push/send";
const ACCESS_TOKEN: &str = "task-9-expo-access-token-secret";
const EXPO_TOKEN: &str = "ExponentPushToken[task-9-expo-destination-secret]";
const PREVIEW: &str = "친구가 보낸 비공개 미리보기";
const NOTIFICATION_ID: &str = "11111111-1111-4111-8111-111111111111";
const CONVERSATION_ID: &str = "22222222-2222-4222-8222-222222222222";
const MESSAGE_ID: &str = "33333333-3333-4333-8333-333333333333";

#[test]
fn configuration_accepts_https_or_loopback_only_and_redacts_access_token() -> TestResult {
    let provider = ExpoPushProvider::new(
        "http://127.0.0.1:3100/--/api/v2/push/send",
        Some(ACCESS_TOKEN.to_owned()),
    )?;
    let debug = format!("{provider:?}");
    assert!(debug.contains("[REDACTED]"));
    assert!(!debug.contains(ACCESS_TOKEN));

    for endpoint in [
        "http://expo.example.test/--/api/v2/push/send",
        "https://user:password@expo.example.test/--/api/v2/push/send",
        "https://expo.example.test/not-the-send-path",
        "https://expo.example.test/--/api/v2/push/send?token=secret",
    ] {
        assert_eq!(
            ExpoPushProvider::new(endpoint, None).err(),
            Some(ExpoPushProviderConfigError::InvalidConfiguration)
        );
    }
    assert_eq!(
        ExpoPushProvider::new(
            "https://expo.example.test/--/api/v2/push/send",
            Some(String::new()),
        )
        .err(),
        Some(ExpoPushProviderConfigError::InvalidConfiguration)
    );
    Ok(())
}

#[tokio::test]
async fn request_maps_identifier_only_handoff_and_optional_preview_to_one_expo_message()
-> TestResult {
    let server = ScriptedExpo::start([
        ScriptedResponse::json(
            StatusCode::OK,
            json!({"data": {"status": "ok", "id": "ticket-without-preview"}}),
        ),
        ScriptedResponse::json(
            StatusCode::OK,
            json!({"data": {"status": "ok", "id": "ticket-with-preview"}}),
        ),
    ])
    .await?;
    let provider = provider(&server, Some(ACCESS_TOKEN))?;

    let without_preview = push_request(None)?;
    assert_eq!(
        provider.send(&without_preview).await,
        Ok(PushProviderOutcome::Accepted)
    );
    let with_preview = push_request(Some(PREVIEW))?;
    assert_eq!(
        provider.send(&with_preview).await,
        Ok(PushProviderOutcome::Accepted)
    );

    let requests = server.finish().await?;
    assert_eq!(requests.len(), 2);
    let expected_authorization = format!("Bearer {ACCESS_TOKEN}");
    for request in &requests {
        assert_eq!(request.method, Method::POST);
        assert_eq!(request.path, SEND_PATH);
        assert_eq!(request.accept.as_deref(), Some("application/json"));
        assert_eq!(request.content_type.as_deref(), Some("application/json"));
        assert_eq!(
            request.authorization.as_deref(),
            Some(expected_authorization.as_str())
        );
    }
    let route = json!({
        "type": "chat_unread",
        "notification_id": NOTIFICATION_ID,
        "conversation_id": CONVERSATION_ID,
        "message_id": MESSAGE_ID
    });
    assert_eq!(
        requests[0].body,
        json!({
            "to": EXPO_TOKEN,
            "data": route
        })
    );
    assert_eq!(
        requests[1].body,
        json!({
            "to": EXPO_TOKEN,
            "data": route,
            "body": PREVIEW
        })
    );
    Ok(())
}

#[tokio::test]
async fn ticket_and_http_failures_use_retry_safe_classification() -> TestResult {
    let server = ScriptedExpo::start([
        ScriptedResponse::json(
            StatusCode::OK,
            json!({
                "data": {
                    "status": "error",
                    "message": "not registered",
                    "details": {"error": "DeviceNotRegistered"}
                }
            }),
        ),
        ScriptedResponse::json(
            StatusCode::OK,
            json!({
                "data": {
                    "status": "error",
                    "message": "bad credentials",
                    "details": {"error": "InvalidCredentials"}
                }
            }),
        ),
        ScriptedResponse::json(
            StatusCode::BAD_REQUEST,
            json!({"errors": [{"code": "VALIDATION_ERROR"}]}),
        ),
        ScriptedResponse::json(
            StatusCode::TOO_MANY_REQUESTS,
            json!({"errors": [{"code": "TOO_MANY_REQUESTS"}]}),
        ),
        ScriptedResponse::json(
            StatusCode::SERVICE_UNAVAILABLE,
            json!({"errors": [{"code": "UPSTREAM_UNAVAILABLE"}]}),
        ),
        ScriptedResponse {
            status: StatusCode::OK,
            body: "not-json".to_owned(),
        },
    ])
    .await?;
    let provider = provider(&server, None)?;

    let mut actual = Vec::new();
    for _ in 0..6 {
        actual.push(provider.send(&push_request(None)?).await);
    }
    assert_eq!(
        actual,
        vec![
            Ok(PushProviderOutcome::DeviceNotRegistered),
            Err(PushProviderError::Rejected),
            Err(PushProviderError::Rejected),
            Err(PushProviderError::Unavailable),
            Err(PushProviderError::Unavailable),
            Err(PushProviderError::Unavailable),
        ]
    );
    let requests = server.finish().await?;
    assert_eq!(requests.len(), 6);
    assert!(
        requests
            .iter()
            .all(|request| request.authorization.is_none())
    );
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn structured_logs_exclude_destination_auth_and_preview_material() -> TestResult {
    let writer = SharedWriter::default();
    let output = writer.clone();
    let subscriber = build_json_subscriber(writer, "jamye_server=info")?;
    let _guard = tracing::subscriber::set_default(subscriber);
    let _interest_sentinel = tracing::Dispatch::new(tracing::subscriber::NoSubscriber::default());
    let server = ScriptedExpo::start([
        ScriptedResponse::json(
            StatusCode::OK,
            json!({"data": {"status": "ok", "id": "secret-safe-ticket"}}),
        ),
        ScriptedResponse::json(
            StatusCode::SERVICE_UNAVAILABLE,
            json!({"errors": [{"code": "UPSTREAM_UNAVAILABLE"}]}),
        ),
    ])
    .await?;
    let provider = provider(&server, Some(ACCESS_TOKEN))?;

    assert_eq!(
        provider.send(&push_request(Some(PREVIEW))?).await,
        Ok(PushProviderOutcome::Accepted)
    );
    assert_eq!(
        provider.send(&push_request(Some(PREVIEW))?).await,
        Err(PushProviderError::Unavailable)
    );
    assert_eq!(server.finish().await?.len(), 2);

    let logs = output.snapshot()?;
    let entries = output.parsed_lines()?;
    assert!(!entries.is_empty());
    assert!(entries.iter().all(Value::is_object));
    for expected in ["expo_push_accepted", "expo_push_unavailable"] {
        assert!(logs.contains(expected), "logs omitted {expected}");
    }
    let authorization = format!("Bearer {ACCESS_TOKEN}");
    for forbidden in [EXPO_TOKEN, ACCESS_TOKEN, PREVIEW, authorization.as_str()] {
        assert!(!logs.contains(forbidden), "logs leaked protected material");
    }
    Ok(())
}

fn provider(server: &ScriptedExpo, access_token: Option<&str>) -> TestResult<ExpoPushProvider> {
    Ok(ExpoPushProvider::new(
        server.endpoint(),
        access_token.map(str::to_owned),
    )?)
}

fn push_request(preview: Option<&str>) -> TestResult<PushProviderRequest> {
    Ok(PushProviderRequest {
        destination: ExpoPushDestination::new(PushEnvironment::Development, EXPO_TOKEN.to_owned()),
        route: PushTapPayload {
            notification_type: NotificationType::ChatUnread,
            notification_id: Uuid::parse_str(NOTIFICATION_ID)?,
            conversation_id: Uuid::parse_str(CONVERSATION_ID)?,
            message_id: Some(Uuid::parse_str(MESSAGE_ID)?),
        },
        preview: preview.map(str::to_owned),
    })
}

#[derive(Clone)]
struct ScriptState {
    responses: Arc<Mutex<VecDeque<ScriptedResponse>>>,
    requests: Arc<Mutex<Vec<ObservedRequest>>>,
}

struct ScriptedExpo {
    endpoint: String,
    state: ScriptState,
    shutdown: oneshot::Sender<()>,
    task: JoinHandle<Result<(), io::Error>>,
}

impl ScriptedExpo {
    async fn start(responses: impl IntoIterator<Item = ScriptedResponse>) -> TestResult<Self> {
        let state = ScriptState {
            responses: Arc::new(Mutex::new(responses.into_iter().collect())),
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
            endpoint: format!("http://{address}{SEND_PATH}"),
            state,
            shutdown,
            task,
        })
    }

    fn endpoint(&self) -> &str {
        &self.endpoint
    }

    async fn finish(self) -> TestResult<Vec<ObservedRequest>> {
        self.shutdown
            .send(())
            .map_err(|_| io::Error::other("scripted Expo server stopped before shutdown"))?;
        self.task.await??;

        let remaining = self
            .state
            .responses
            .lock()
            .map_err(|_| io::Error::other("scripted Expo response mutex is poisoned"))?;
        if !remaining.is_empty() {
            return Err(io::Error::other("scripted Expo did not receive every request").into());
        }
        drop(remaining);

        self.state
            .requests
            .lock()
            .map(|requests| requests.clone())
            .map_err(|_| io::Error::other("scripted Expo request mutex is poisoned").into())
    }
}

#[derive(Clone)]
struct ScriptedResponse {
    status: StatusCode,
    body: String,
}

impl ScriptedResponse {
    fn json(status: StatusCode, body: Value) -> Self {
        Self {
            status,
            body: body.to_string(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ObservedRequest {
    method: Method,
    path: String,
    accept: Option<String>,
    content_type: Option<String>,
    authorization: Option<String>,
    body: Value,
}

async fn handle_request(
    State(state): State<ScriptState>,
    request: Request<Body>,
) -> Response<Body> {
    let (parts, body) = request.into_parts();
    let body = to_bytes(body, 16 * 1024)
        .await
        .ok()
        .and_then(|body| serde_json::from_slice(&body).ok())
        .unwrap_or(Value::Null);
    let observed = ObservedRequest {
        method: parts.method,
        path: parts.uri.path().to_owned(),
        accept: header_value(&parts.headers, header::ACCEPT),
        content_type: header_value(&parts.headers, header::CONTENT_TYPE),
        authorization: header_value(&parts.headers, header::AUTHORIZATION),
        body,
    };
    let recorded = state
        .requests
        .lock()
        .map(|mut requests| requests.push(observed))
        .is_ok();
    let scripted = state
        .responses
        .lock()
        .ok()
        .and_then(|mut responses| responses.pop_front());
    let scripted = match (recorded, scripted) {
        (true, Some(scripted)) => scripted,
        _ => ScriptedResponse::json(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({"errors": [{"code": "SCRIPT_EXHAUSTED"}]}),
        ),
    };
    let mut response = Response::new(Body::from(scripted.body));
    *response.status_mut() = scripted.status;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("application/json"),
    );
    response
}

fn header_value(headers: &HeaderMap, name: HeaderName) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}
