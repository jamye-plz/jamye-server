//! Deterministic OpenAPI 3.1 document for the C0 slice.

use serde_json::Value;
use utoipa::{Modify, OpenApi};

use jamye_server::transport::http::health::{
    DependencyCheck, DependencyChecks, DependencyStatus, LivenessResponse, LivenessStatus,
    ReadinessResponse, ReadinessStatus,
};

use super::model::{
    CanonicalMessage, DeltaItem, ErrorBody, ErrorEnvelope, EventPage, MediaRef, MessageAttachment,
    MessageCreate, MessageCreatedEvent, MessageCreatedType, MessageKind, RealtimeTicket,
    ReconcileScope, UnsupportedEventMarker,
};
use super::{BoxError, invalid_data};

pub const OPERATION_IDS: &[&str] = &["H1", "H2", "C4", "S1", "R1"];

#[utoipa::path(
    get,
    operation_id = "H1",
    path = "/health/live",
    tag = "health",
    responses((status = 200, description = "Process is alive", body = LivenessResponse))
)]
#[allow(dead_code)]
fn health_live_contract() {}

#[utoipa::path(
    get,
    operation_id = "H2",
    path = "/health/ready",
    tag = "health",
    responses(
        (status = 200, description = "PostgreSQL is ready; optional dependencies are reported independently", body = ReadinessResponse),
        (status = 503, description = "Required PostgreSQL dependency is unavailable", body = ReadinessResponse)
    )
)]
#[allow(dead_code)]
fn health_ready_contract() {}

#[utoipa::path(
    post,
    operation_id = "C4",
    path = "/api/v1/chatrooms/{chatroom_id}/messages",
    tag = "messages",
    request_body(content = MessageCreate, description = "Stable idempotent message command", content_type = "application/json"),
    params(
        ("chatroom_id" = uuid::Uuid, Path, description = "Target chatroom"),
        ("Idempotency-Key" = Option<uuid::Uuid>, Header, description = "Optional; when present it must exactly equal body client_msg_id")
    ),
    responses(
        (status = 201, description = "New canonical message", body = CanonicalMessage),
        (status = 200, description = "D8=A same-payload retry; existing canonical message", body = CanonicalMessage),
        (status = 401, description = "Bearer authentication is required", body = ErrorEnvelope),
        (status = 403, description = "Membership is required without resource disclosure", body = ErrorEnvelope),
        (status = 409, description = "D8=A client_msg_id was reused with a different logical payload", body = ErrorEnvelope),
        (status = 422, description = "Content or Idempotency-Key validation failed", body = ErrorEnvelope),
        (status = 503, description = "Required PostgreSQL dependency is unavailable", body = ErrorEnvelope)
    ),
    security(("bearer_auth" = []))
)]
#[allow(dead_code)]
fn create_message_contract() {}

#[utoipa::path(
    get,
    operation_id = "S1",
    path = "/api/v1/conversations/{conversation_id}/events",
    tag = "sync",
    params(
        ("conversation_id" = uuid::Uuid, Path, description = "Conversation to recover"),
        ("after" = Option<String>, Query, description = "Opaque last-applied server cursor"),
        ("limit" = Option<u32>, Query, minimum = 1, description = "Bounded page size"),
        ("X-Jamye-Contract-Version" = String, Header, description = "Required current or previous contract version")
    ),
    responses(
        (status = 200, description = "Version-projected delta page", body = EventPage,
            headers(("X-Jamye-Contract-Version" = String, description = "Accepted contract version"))),
        (status = 401, description = "Bearer authentication is required", body = ErrorEnvelope),
        (status = 403, description = "Membership is required without resource disclosure", body = ErrorEnvelope),
        (status = 426, description = "Requested or persisted event version cannot converge safely", body = ErrorEnvelope),
        (status = 503, description = "Required PostgreSQL dependency is unavailable", body = ErrorEnvelope)
    ),
    security(("bearer_auth" = []))
)]
#[allow(dead_code)]
fn conversation_events_contract() {}

#[utoipa::path(
    post,
    operation_id = "R1",
    path = "/api/v1/realtime/tickets",
    tag = "realtime",
    params(
        ("X-Jamye-Contract-Version" = String, Header, description = "Required current or previous contract version")
    ),
    responses(
        (status = 201, description = "One-time version-bound realtime ticket", body = RealtimeTicket,
            headers(("X-Jamye-Contract-Version" = String, description = "Accepted contract version"))),
        (status = 401, description = "Bearer authentication is required", body = ErrorEnvelope),
        (status = 426, description = "Unsupported contract version", body = ErrorEnvelope),
        (status = 503, description = "Redis ticket service is unavailable", body = ErrorEnvelope)
    ),
    security(("bearer_auth" = []))
)]
#[allow(dead_code)]
fn realtime_ticket_contract() {}

#[derive(OpenApi)]
#[openapi(
    paths(
        health_live_contract,
        health_ready_contract,
        create_message_contract,
        conversation_events_contract,
        realtime_ticket_contract
    ),
    components(schemas(
        LivenessStatus,
        LivenessResponse,
        ReadinessStatus,
        DependencyStatus,
        DependencyCheck,
        DependencyChecks,
        ReadinessResponse,
        ErrorEnvelope,
        ErrorBody,
        MediaRef,
        MessageCreate,
        MessageKind,
        MessageAttachment,
        CanonicalMessage,
        ReconcileScope,
        UnsupportedEventMarker,
        MessageCreatedType,
        MessageCreatedEvent,
        DeltaItem,
        EventPage,
        RealtimeTicket
    )),
    modifiers(&SecurityAddon)
)]
struct C0OpenApi;

struct SecurityAddon;

impl Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};

        if let Some(components) = openapi.components.as_mut() {
            components.add_security_scheme(
                "bearer_auth",
                SecurityScheme::Http(
                    HttpBuilder::new()
                        .scheme(HttpAuthScheme::Bearer)
                        .bearer_format("JWT")
                        .description(Some("Short-lived signed access token"))
                        .build(),
                ),
            );
        }
    }
}

pub fn document() -> Result<Value, BoxError> {
    let mut openapi = C0OpenApi::openapi();
    openapi.info.title = "Jamye Server C0 API".to_owned();
    openapi.info.version = "1".to_owned();
    openapi.info.description = Some(
        "Deterministic C0 snapshot. Runtime feature owners extend this explicit compile-time surface."
            .to_owned(),
    );

    let mut value = serde_json::to_value(openapi)?;
    let root = value
        .as_object_mut()
        .ok_or_else(|| invalid_data("OpenAPI root must be an object"))?;
    root.insert(
        "jsonSchemaDialect".to_owned(),
        Value::String("https://json-schema.org/draft/2020-12/schema".to_owned()),
    );
    root.insert(
        "x-jamye-contract-stage".to_owned(),
        Value::String("C0".to_owned()),
    );

    enforce_exact_null_details(&mut value)?;
    enforce_message_content_rule(&mut value)?;
    enforce_contract_version_headers(&mut value)?;
    validate_operation_ids(&value)?;
    Ok(value)
}

fn enforce_exact_null_details(openapi: &mut Value) -> Result<(), BoxError> {
    let envelope = openapi
        .pointer_mut("/components/schemas/ErrorEnvelope")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| invalid_data("OpenAPI ErrorEnvelope component is missing"))?;
    envelope.insert("required".to_owned(), serde_json::json!(["error"]));
    envelope.insert("additionalProperties".to_owned(), Value::Bool(false));

    let error_body = openapi
        .pointer_mut("/components/schemas/ErrorBody")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| invalid_data("OpenAPI ErrorBody component is missing"))?;
    let properties = error_body
        .get_mut("properties")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| invalid_data("OpenAPI ErrorBody properties are missing"))?;
    properties.insert(
        "details".to_owned(),
        serde_json::json!({
            "description": "Reserved for a future additive contract; exactly null in v1",
            "type": "null"
        }),
    );
    error_body.insert(
        "required".to_owned(),
        serde_json::json!(["code", "message", "request_id", "details"]),
    );
    error_body.insert("additionalProperties".to_owned(), Value::Bool(false));
    Ok(())
}

fn enforce_message_content_rule(openapi: &mut Value) -> Result<(), BoxError> {
    let message_create = openapi
        .pointer_mut("/components/schemas/MessageCreate")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| invalid_data("OpenAPI MessageCreate component is missing"))?;
    message_create.insert(
        "anyOf".to_owned(),
        serde_json::json!([
            {
                "required": ["body"],
                "properties": {"body": {"type": "string", "minLength": 1}}
            },
            {
                "required": ["media"],
                "properties": {"media": {"type": "array", "minItems": 1, "maxItems": 4}}
            }
        ]),
    );
    message_create.insert("additionalProperties".to_owned(), Value::Bool(false));
    Ok(())
}

fn enforce_contract_version_headers(openapi: &mut Value) -> Result<(), BoxError> {
    let paths = openapi
        .get_mut("paths")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| invalid_data("OpenAPI paths are missing"))?;
    for path_item in paths.values_mut() {
        let Some(methods) = path_item.as_object_mut() else {
            continue;
        };
        for operation in methods.values_mut() {
            let Some(operation) = operation.as_object_mut() else {
                continue;
            };
            let Some(operation_id) = operation.get("operationId").and_then(Value::as_str) else {
                continue;
            };
            if operation_id != "S1" && operation_id != "R1" {
                continue;
            }
            let parameters = operation
                .get_mut("parameters")
                .and_then(Value::as_array_mut)
                .ok_or_else(|| invalid_data("versioned operation parameters are missing"))?;
            let version_header = parameters
                .iter_mut()
                .find(|parameter| {
                    parameter.get("name").and_then(Value::as_str)
                        == Some("X-Jamye-Contract-Version")
                })
                .and_then(Value::as_object_mut)
                .ok_or_else(|| invalid_data("X-Jamye-Contract-Version parameter is missing"))?;
            version_header.insert("required".to_owned(), Value::Bool(true));
            version_header.insert(
                "schema".to_owned(),
                serde_json::json!({"type": "string", "enum": ["1", "0"]}),
            );
        }
    }
    Ok(())
}

fn validate_operation_ids(openapi: &Value) -> Result<(), BoxError> {
    let paths = openapi
        .get("paths")
        .and_then(Value::as_object)
        .ok_or_else(|| invalid_data("OpenAPI paths are missing"))?;
    let mut actual = Vec::new();
    for path_item in paths.values() {
        let methods = path_item
            .as_object()
            .ok_or_else(|| invalid_data("OpenAPI path item must be an object"))?;
        for operation in methods.values() {
            if let Some(operation_id) = operation.get("operationId").and_then(Value::as_str) {
                actual.push(operation_id.to_owned());
            }
        }
    }
    actual.sort();
    let mut expected = OPERATION_IDS
        .iter()
        .map(|operation_id| (*operation_id).to_owned())
        .collect::<Vec<_>>();
    expected.sort();
    if actual != expected {
        return Err(invalid_data(format!(
            "C0 operation IDs differ: expected {expected:?}, got {actual:?}"
        ))
        .into());
    }
    if actual.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(invalid_data("duplicate C0 operation ID").into());
    }
    Ok(())
}
