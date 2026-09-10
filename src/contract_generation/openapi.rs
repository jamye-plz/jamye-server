//! Deterministic OpenAPI 3.1 document for the C0 slice.

use serde_json::{Value, json};
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
use super::{BoxError, invalid_data, selected};

pub const C0_OPERATION_IDS: &[&str] = &["H1", "H2", "C4", "S1", "R1"];
pub const OPERATION_IDS: &[&str] = &[
    "H1", "H2", "A1", "A2", "A3", "A4", "A5", "U1", "U2", "U3", "G1", "G2", "G3", "G4", "G5", "G6",
    "G7", "G8", "I1", "I2", "T1", "T2", "T3", "T4", "T5", "T6", "T7", "MD1", "MD2", "MD3", "C1",
    "C2", "C3", "C4", "MD4", "MD5", "S1", "R1", "P2", "P3", "P4", "N1", "N2",
];

struct OwnerOperationContribution {
    path: &'static str,
    document: &'static str,
}

struct OwnerSchemaContribution {
    path: &'static str,
    document: &'static str,
}

const OWNER_OPERATION_CONTRIBUTIONS: &[OwnerOperationContribution] = &[
    OwnerOperationContribution {
        path: "contracts/contributions/task-5/dto/operations.json",
        document: include_str!("../../contracts/contributions/task-5/dto/operations.json"),
    },
    OwnerOperationContribution {
        path: "contracts/contributions/task-6/dto/operations.json",
        document: include_str!("../../contracts/contributions/task-6/dto/operations.json"),
    },
    OwnerOperationContribution {
        path: "contracts/contributions/task-6b/dto/operations.json",
        document: include_str!("../../contracts/contributions/task-6b/dto/operations.json"),
    },
    OwnerOperationContribution {
        path: "contracts/contributions/task-7/dto/operations.json",
        document: include_str!("../../contracts/contributions/task-7/dto/operations.json"),
    },
    OwnerOperationContribution {
        path: "contracts/contributions/task-8/dto/operations.json",
        document: include_str!("../../contracts/contributions/task-8/dto/operations.json"),
    },
    OwnerOperationContribution {
        path: "contracts/contributions/task-9/dto/operations.json",
        document: include_str!("../../contracts/contributions/task-9/dto/operations.json"),
    },
];

const OWNER_SCHEMA_CONTRIBUTIONS: &[OwnerSchemaContribution] = &[
    OwnerSchemaContribution {
        path: "contracts/contributions/task-5/schemas/auth-wire.schema.json",
        document: include_str!(
            "../../contracts/contributions/task-5/schemas/auth-wire.schema.json"
        ),
    },
    OwnerSchemaContribution {
        path: "contracts/contributions/task-6/schemas/groups-wire.schema.json",
        document: include_str!(
            "../../contracts/contributions/task-6/schemas/groups-wire.schema.json"
        ),
    },
    OwnerSchemaContribution {
        path: "contracts/contributions/task-6b/schemas/chatrooms-wire.schema.json",
        document: include_str!(
            "../../contracts/contributions/task-6b/schemas/chatrooms-wire.schema.json"
        ),
    },
    OwnerSchemaContribution {
        path: "contracts/contributions/task-7/schemas/topics-wire.schema.json",
        document: include_str!(
            "../../contracts/contributions/task-7/schemas/topics-wire.schema.json"
        ),
    },
    OwnerSchemaContribution {
        path: "contracts/contributions/task-8/schemas/media-wire.schema.json",
        document: include_str!(
            "../../contracts/contributions/task-8/schemas/media-wire.schema.json"
        ),
    },
    OwnerSchemaContribution {
        path: "contracts/contributions/task-9/schemas/notifications-push-wire.schema.json",
        document: include_str!(
            "../../contracts/contributions/task-9/schemas/notifications-push-wire.schema.json"
        ),
    },
];

const PRODUCTION_SERVER_URL: &str = "https://jamye-api.ridewithmin.com";
const PUBLIC_OPERATION_IDS: &[&str] = &["H1", "H2", "A1", "A2", "A3", "A5"];

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
    validate_operation_ids(&value, C0_OPERATION_IDS, "C0")?;
    Ok(value)
}

pub fn document_release_candidate() -> Result<Value, BoxError> {
    let mut value = document()?;
    {
        let root = value
            .as_object_mut()
            .ok_or_else(|| invalid_data("OpenAPI root must be an object"))?;
        root.insert(
            "x-jamye-contract-stage".to_owned(),
            Value::String("C2".to_owned()),
        );
        let info = root
            .get_mut("info")
            .and_then(Value::as_object_mut)
            .ok_or_else(|| invalid_data("OpenAPI info is missing"))?;
        info.insert(
            "title".to_owned(),
            Value::String("Jamye Server API".to_owned()),
        );
        info.insert("version".to_owned(), Value::String("1".to_owned()));
        info.insert(
            "description".to_owned(),
            Value::String(
                "Production HTTP API for Jamye mobile clients. All error responses use the shared ErrorEnvelope schema."
                    .to_owned(),
            ),
        );
    }

    merge_owner_schemas(&mut value)?;
    add_release_metadata(&mut value)?;

    {
        let paths = value
            .get_mut("paths")
            .and_then(Value::as_object_mut)
            .ok_or_else(|| invalid_data("OpenAPI paths are missing"))?;
        for surface in selected::REST_SURFACES {
            let path = paths
                .entry(surface.path.to_owned())
                .or_insert_with(|| json!({}));
            let methods = path
                .as_object_mut()
                .ok_or_else(|| invalid_data("OpenAPI path item must be an object"))?;
            if let Some(existing) = methods.get_mut(surface.method) {
                if existing.get("operationId").and_then(Value::as_str) != Some(surface.operation_id)
                {
                    return Err(invalid_data(
                        "C2 static surface conflicts with a typed OpenAPI operation",
                    )
                    .into());
                }
                decorate_operation(surface, existing)?;
                continue;
            }
            methods.insert(
                surface.method.to_owned(),
                release_candidate_operation(surface)?,
            );
        }
    }
    validate_operation_ids(&value, OPERATION_IDS, "C2")?;
    Ok(value)
}

fn release_candidate_operation(surface: &selected::RestSurface) -> Result<Value, BoxError> {
    let owner = owner_operation(surface.operation_id)?;
    if owner.is_none() && surface.operation_id != "U3" {
        return Err(invalid_data(format!(
            "C2 operation {} has neither a typed C0 operation nor an owner contribution",
            surface.operation_id
        ))
        .into());
    }

    let mut operation = serde_json::Map::new();
    operation.insert(
        "operationId".to_owned(),
        Value::String(surface.operation_id.to_owned()),
    );
    operation.insert(
        "summary".to_owned(),
        Value::String(operation_summary(surface.operation_id)?.to_owned()),
    );
    operation.insert(
        "tags".to_owned(),
        json!([operation_tag(surface.operation_id)?]),
    );

    let parameters = operation_parameters(surface);
    if !parameters.is_empty() {
        operation.insert("parameters".to_owned(), Value::Array(parameters));
    }
    if let Some(component) = request_component(surface.operation_id) {
        operation.insert(
            "requestBody".to_owned(),
            json!({
                "required": true,
                "content": {
                    "application/json": {
                        "schema": component_ref(component)
                    }
                }
            }),
        );
    }
    operation.insert(
        "responses".to_owned(),
        success_and_error_responses(surface.operation_id)?,
    );
    if !PUBLIC_OPERATION_IDS.contains(&surface.operation_id) {
        operation.insert("security".to_owned(), json!([{"bearer_auth": []}]));
    }
    operation.insert(
        "x-jamye-behavior-test".to_owned(),
        Value::String(surface.feature_behavior_test.to_owned()),
    );
    operation.insert(
        "x-jamye-fixture".to_owned(),
        Value::String(surface.fixture.to_owned()),
    );
    if let Some((contribution_path, owner_operation)) = owner {
        operation.insert(
            "x-jamye-owner-contribution".to_owned(),
            Value::String(contribution_path.to_owned()),
        );
        if let Some(authorization) = owner_operation.get("auth") {
            operation.insert("x-jamye-authorization".to_owned(), authorization.clone());
        }
        if let Some(errors) = owner_operation.get("errors") {
            operation.insert("x-jamye-errors".to_owned(), errors.clone());
        }
    }
    Ok(Value::Object(operation))
}

fn decorate_operation(
    surface: &selected::RestSurface,
    operation: &mut Value,
) -> Result<(), BoxError> {
    let operation = operation
        .as_object_mut()
        .ok_or_else(|| invalid_data("OpenAPI operation must be an object"))?;
    operation.insert(
        "summary".to_owned(),
        Value::String(operation_summary(surface.operation_id)?.to_owned()),
    );
    operation.insert(
        "x-jamye-behavior-test".to_owned(),
        Value::String(surface.feature_behavior_test.to_owned()),
    );
    operation.insert(
        "x-jamye-fixture".to_owned(),
        Value::String(surface.fixture.to_owned()),
    );
    Ok(())
}

fn owner_operation(operation_id: &str) -> Result<Option<(&'static str, Value)>, BoxError> {
    for contribution in OWNER_OPERATION_CONTRIBUTIONS {
        let document: Value = serde_json::from_str(contribution.document)?;
        let Some(operations) = document.get("operations").and_then(Value::as_array) else {
            return Err(
                invalid_data("owner operation contribution has no operations array").into(),
            );
        };
        if let Some(operation) = operations
            .iter()
            .find(|operation| operation.get("id").and_then(Value::as_str) == Some(operation_id))
        {
            return Ok(Some((contribution.path, operation.clone())));
        }
    }
    Ok(None)
}

fn merge_owner_schemas(openapi: &mut Value) -> Result<(), BoxError> {
    let schemas = openapi
        .pointer_mut("/components/schemas")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| invalid_data("OpenAPI component schemas are missing"))?;
    for contribution in OWNER_SCHEMA_CONTRIBUTIONS {
        let document: Value = serde_json::from_str(contribution.document)?;
        let definitions = document
            .get("$defs")
            .and_then(Value::as_object)
            .ok_or_else(|| {
                invalid_data(format!(
                    "owner schema contribution has no $defs object: {}",
                    contribution.path
                ))
            })?;
        for (name, definition) in definitions {
            let mut definition = definition.clone();
            normalize_owner_schema_refs(&mut definition);
            if schemas.contains_key(name) && !owner_schema_can_replace(name) {
                return Err(invalid_data(format!(
                    "duplicate OpenAPI component schema from {}: {name}",
                    contribution.path
                ))
                .into());
            }
            schemas.insert(name.clone(), definition);
        }
    }
    Ok(())
}

fn owner_schema_can_replace(name: &str) -> bool {
    matches!(name, "MediaRef" | "MessageAttachment" | "TopicMedia")
}

fn normalize_owner_schema_refs(value: &mut Value) {
    match value {
        Value::String(reference) => {
            if let Some((_, name)) = reference.rsplit_once("#/$defs/") {
                *reference = format!("#/components/schemas/{name}");
            }
        }
        Value::Array(items) => {
            for item in items {
                normalize_owner_schema_refs(item);
            }
        }
        Value::Object(object) => {
            let normalized = object
                .get("$ref")
                .and_then(Value::as_str)
                .and_then(|reference| reference.rsplit_once("#/$defs/"))
                .map(|(_, name)| format!("#/components/schemas/{name}"));
            if let Some(normalized) = normalized {
                object.insert("$ref".to_owned(), Value::String(normalized));
            }
            for child in object.values_mut() {
                normalize_owner_schema_refs(child);
            }
        }
        _ => {}
    }
}

fn add_release_metadata(openapi: &mut Value) -> Result<(), BoxError> {
    let root = openapi
        .as_object_mut()
        .ok_or_else(|| invalid_data("OpenAPI root must be an object"))?;
    root.insert(
        "servers".to_owned(),
        json!([{
            "url": PRODUCTION_SERVER_URL,
            "description": "Production"
        }]),
    );
    root.insert(
        "tags".to_owned(),
        json!([
            {"name": "health", "description": "Process and dependency health"},
            {"name": "auth", "description": "OAuth and token lifecycle"},
            {"name": "users", "description": "Current user profile and account lifecycle"},
            {"name": "groups", "description": "Groups and memberships"},
            {"name": "invites", "description": "Group invitations"},
            {"name": "chatrooms", "description": "Chatroom discovery, history, and read markers"},
            {"name": "messages", "description": "Idempotent chat message commands"},
            {"name": "topics", "description": "Topics, dates, tags, and topic media"},
            {"name": "media", "description": "Private media upload, finalize, and access"},
            {"name": "sync", "description": "Cursor-based event recovery"},
            {"name": "realtime", "description": "One-time tickets for WebSocket sessions"},
            {"name": "push", "description": "Expo push installation lifecycle"},
            {"name": "notifications", "description": "Notification history and read state"}
        ]),
    );
    root.insert(
        "x-jamye-realtime".to_owned(),
        json!({
            "url": "wss://jamye-api.ridewithmin.com/api/v1/realtime/ws?ticket={ticket}",
            "ticket_operation_id": "R1",
            "ticket_query_parameter": "ticket",
            "client_frame_schema": "./realtime/client-frame.schema.json",
            "server_frame_schema": "./realtime/server-frame.schema.json",
            "protocol": "./realtime/protocol.json",
            "description": "Obtain a one-time ticket with R1, then use it for the WebSocket HTTP upgrade. Realtime frames are defined by the linked JSON Schemas."
        }),
    );
    Ok(())
}

fn component_ref(name: &str) -> Value {
    json!({"$ref": format!("#/components/schemas/{name}")})
}

fn request_component(operation_id: &str) -> Option<&'static str> {
    match operation_id {
        "A1" => Some("OAuthAuthorizeIn"),
        "A2" => Some("OAuthExchangeIn"),
        "A3" => Some("RefreshIn"),
        "U2" => Some("UserPatch"),
        "G1" => Some("GroupCreate"),
        "G5" => Some("GroupPatch"),
        "G8" => Some("MemberRolePatch"),
        "I1" => Some("InviteCreate"),
        "T1" => Some("TopicCreate"),
        "T5" => Some("TopicPatch"),
        "T6" => Some("TagReplace"),
        "MD1" => Some("UploadIntentCreate"),
        "MD2" => Some("UploadFinalize"),
        "C3" => Some("ReadAnchorIn"),
        "P2" => Some("ExpoInstallationCreate"),
        "P3" => Some("ExpoInstallationPut"),
        _ => None,
    }
}

fn response_component(operation_id: &str) -> Option<&'static str> {
    match operation_id {
        "A1" => Some("OAuthAuthorizeOut"),
        "A2" | "A3" => Some("TokenPair"),
        "U1" | "U2" => Some("User"),
        "G1" | "G3" | "G5" => Some("Group"),
        "G2" => Some("GroupPage"),
        "G4" => Some("MemberPage"),
        "I1" => Some("Invite"),
        "I2" => Some("InviteJoinResult"),
        "T1" | "T4" | "T5" => Some("CanonicalTopic"),
        "T2" => Some("TopicDatePage"),
        "T3" => Some("TopicPage"),
        "T6" | "T7" => Some("TagPage"),
        "MD1" => Some("UploadIntentWithPresignedPut"),
        "MD2" => Some("UploadFinalizeResult"),
        "MD3" => Some("TopicMediaPage"),
        "C1" => Some("ChatroomPage"),
        "C2" => Some("DenormalizedMessagePage"),
        "C3" => Some("ReadMarker"),
        "MD4" => Some("MediaAccessUrl"),
        "P2" | "P3" => Some("PushInstallation"),
        "N1" => Some("NotificationPage"),
        _ => None,
    }
}

fn operation_parameters(surface: &selected::RestSurface) -> Vec<Value> {
    let mut parameters = surface
        .path
        .split('/')
        .filter_map(|segment| {
            segment
                .strip_prefix('{')
                .and_then(|name| name.strip_suffix('}'))
        })
        .map(path_parameter)
        .collect::<Vec<_>>();

    let cursor = match surface.operation_id {
        "G2" | "G4" | "C1" | "T2" | "T3" | "T7" | "MD3" | "N1" => Some("after"),
        "C2" => Some("before"),
        _ => None,
    };
    if let Some(cursor) = cursor {
        parameters.push(json!({
            "name": cursor,
            "in": "query",
            "required": false,
            "description": "Opaque pagination cursor returned by the previous page",
            "schema": {"type": "string"}
        }));
        let (default, maximum) = page_limit(surface.operation_id);
        parameters.push(json!({
            "name": "limit",
            "in": "query",
            "required": false,
            "description": "Bounded page size",
            "schema": {
                "type": "integer",
                "format": "int32",
                "minimum": 1,
                "maximum": maximum,
                "default": default
            }
        }));
    }
    if surface.operation_id == "T3" {
        parameters.push(json!({
            "name": "date",
            "in": "query",
            "required": false,
            "description": "Seoul calendar date filter",
            "schema": {"type": "string", "format": "date"}
        }));
    }
    if surface.operation_id == "T1" {
        parameters.push(json!({
            "name": "Idempotency-Key",
            "in": "header",
            "required": true,
            "description": "Stable UUID reused for an exact retry",
            "schema": {"type": "string", "format": "uuid"}
        }));
    }
    if surface.operation_id == "A5" {
        parameters.extend([
            json!({
                "name": "state",
                "in": "query",
                "required": true,
                "description": "The exact 43-character state returned by authorization",
                "schema": {"type": "string", "pattern": "^[A-Za-z0-9_-]{43}$"}
            }),
            json!({
                "name": "code",
                "in": "query",
                "required": false,
                "description": "Provider authorization code; mutually exclusive with error",
                "schema": {"type": "string", "minLength": 1, "maxLength": 4096}
            }),
            json!({
                "name": "error",
                "in": "query",
                "required": false,
                "description": "Provider error; mutually exclusive with code and normalized before the app redirect",
                "schema": {"type": "string", "minLength": 1, "maxLength": 1024}
            }),
        ]);
    }
    parameters
}

fn path_parameter(name: &str) -> Value {
    let schema = if name == "provider" {
        json!({"type": "string", "enum": ["kakao", "google"]})
    } else if name.ends_with("_id") {
        json!({"type": "string", "format": "uuid"})
    } else {
        json!({"type": "string", "minLength": 1})
    };
    json!({
        "name": name,
        "in": "path",
        "required": true,
        "schema": schema
    })
}

fn page_limit(operation_id: &str) -> (u32, u32) {
    match operation_id {
        "T2" => (31, 366),
        "T3" | "MD3" => (20, 100),
        _ => (50, 100),
    }
}

fn success_and_error_responses(operation_id: &str) -> Result<Value, BoxError> {
    let mut responses = serde_json::Map::new();
    for status in success_statuses(operation_id)? {
        let mut response = serde_json::Map::new();
        response.insert(
            "description".to_owned(),
            Value::String(success_description(status).to_owned()),
        );
        if let Some(component) = response_component(operation_id) {
            response.insert(
                "content".to_owned(),
                json!({
                    "application/json": {
                        "schema": component_ref(component)
                    }
                }),
            );
        }
        if operation_id == "MD5" {
            response.insert(
                "headers".to_owned(),
                json!({
                    "Location": {
                        "description": "Short-lived authorized download URL",
                        "schema": {"type": "string", "format": "uri"}
                    }
                }),
            );
        }
        if operation_id == "A5" {
            response.insert(
                "headers".to_owned(),
                json!({
                    "Location": {
                        "description": "Fixed provider-specific Jamye app return URI with safely encoded code/state or normalized error/state",
                        "schema": {"type": "string", "format": "uri"}
                    },
                    "Cache-Control": {"schema": {"const": "no-store", "type": "string"}},
                    "Referrer-Policy": {"schema": {"const": "no-referrer", "type": "string"}},
                    "X-Content-Type-Options": {"schema": {"const": "nosniff", "type": "string"}}
                }),
            );
        }
        responses.insert((*status).to_owned(), Value::Object(response));
    }
    let mut default = serde_json::Map::new();
    default.insert(
        "description".to_owned(),
        Value::String(
            "Error response; inspect error.code for the stable machine-readable reason".to_owned(),
        ),
    );
    default.insert(
        "content".to_owned(),
        json!({"application/json": {"schema": component_ref("ErrorEnvelope")}}),
    );
    if operation_id == "A5" {
        default.insert("headers".to_owned(), callback_safety_headers());
    }
    responses.insert("default".to_owned(), Value::Object(default));
    Ok(Value::Object(responses))
}

fn callback_safety_headers() -> Value {
    json!({
        "Cache-Control": {"schema": {"const": "no-store", "type": "string"}},
        "Referrer-Policy": {"schema": {"const": "no-referrer", "type": "string"}},
        "X-Content-Type-Options": {"schema": {"const": "nosniff", "type": "string"}}
    })
}

fn success_statuses(operation_id: &str) -> Result<&'static [&'static str], BoxError> {
    match operation_id {
        "A1" | "A2" | "A3" | "U1" | "U2" | "G2" | "G3" | "G4" | "G5" | "I2" | "T2" | "T3"
        | "T4" | "T5" | "T6" | "T7" | "MD2" | "MD3" | "C1" | "C2" | "C3" | "MD4" | "P3" | "N1" => {
            Ok(&["200"])
        }
        "G1" | "I1" | "MD1" => Ok(&["201"]),
        "T1" | "P2" => Ok(&["200", "201"]),
        "A4" | "U3" | "G6" | "G7" | "G8" | "P4" | "N2" => Ok(&["204"]),
        "A5" => Ok(&["302"]),
        "MD5" => Ok(&["307"]),
        _ => Err(invalid_data(format!(
            "release-candidate success status is missing for {operation_id}"
        ))
        .into()),
    }
}

fn success_description(status: &str) -> &'static str {
    match status {
        "200" => "Successful response or canonical idempotent retry",
        "201" => "Resource created",
        "204" => "Successful response with no body",
        "302" => "Temporary redirect to the fixed Jamye app callback URI",
        "307" => "Temporary redirect to a short-lived signed URL",
        _ => "Successful response",
    }
}

fn operation_tag(operation_id: &str) -> Result<&'static str, BoxError> {
    match operation_id {
        "H1" | "H2" => Ok("health"),
        "A1" | "A2" | "A3" | "A4" | "A5" => Ok("auth"),
        "U1" | "U2" | "U3" => Ok("users"),
        "G1" | "G2" | "G3" | "G4" | "G5" | "G6" | "G7" | "G8" => Ok("groups"),
        "I1" | "I2" => Ok("invites"),
        "C1" | "C2" | "C3" => Ok("chatrooms"),
        "C4" => Ok("messages"),
        "T1" | "T2" | "T3" | "T4" | "T5" | "T6" | "T7" => Ok("topics"),
        "MD1" | "MD2" | "MD3" | "MD4" | "MD5" => Ok("media"),
        "S1" => Ok("sync"),
        "R1" => Ok("realtime"),
        "P2" | "P3" | "P4" => Ok("push"),
        "N1" | "N2" => Ok("notifications"),
        _ => Err(invalid_data(format!(
            "release-candidate tag is missing for {operation_id}"
        ))
        .into()),
    }
}

fn operation_summary(operation_id: &str) -> Result<&'static str, BoxError> {
    match operation_id {
        "H1" => Ok("Check process liveness"),
        "H2" => Ok("Check dependency readiness"),
        "A1" => Ok("Start OAuth authorization"),
        "A2" => Ok("Exchange an OAuth authorization code"),
        "A3" => Ok("Rotate a refresh token"),
        "A4" => Ok("Log out the current session"),
        "A5" => Ok("Bridge an OAuth provider callback to the fixed Jamye app URI"),
        "U1" => Ok("Get the current user profile"),
        "U2" => Ok("Update the current user profile"),
        "U3" => Ok("Delete the current account"),
        "G1" => Ok("Create a group"),
        "G2" => Ok("List the current user's groups"),
        "G3" => Ok("Get a group"),
        "G4" => Ok("List group members"),
        "G5" => Ok("Rename a group"),
        "G6" => Ok("Delete a group"),
        "G7" => Ok("Remove or leave a group"),
        "G8" => Ok("Change a group member role"),
        "I1" => Ok("Create a group invitation"),
        "I2" => Ok("Join a group with an invitation"),
        "T1" => Ok("Create a topic"),
        "T2" => Ok("List dates containing topics"),
        "T3" => Ok("List group topics"),
        "T4" => Ok("Get a topic"),
        "T5" => Ok("Update a topic"),
        "T6" => Ok("Replace topic tags"),
        "T7" => Ok("List topic tags"),
        "MD1" => Ok("Create a media upload intent"),
        "MD2" => Ok("Finalize a media upload"),
        "MD3" => Ok("List topic media"),
        "C1" => Ok("List group chatrooms"),
        "C2" => Ok("List chatroom message history"),
        "C3" => Ok("Advance a chatroom read marker"),
        "C4" => Ok("Send an idempotent chat message"),
        "MD4" => Ok("Issue a short-lived media view URL"),
        "MD5" => Ok("Redirect to a media download URL"),
        "S1" => Ok("Recover conversation events after a cursor"),
        "R1" => Ok("Issue a one-time realtime ticket"),
        "P2" => Ok("Create or upsert a push installation"),
        "P3" => Ok("Update a push installation"),
        "P4" => Ok("Delete a push installation"),
        "N1" => Ok("List notifications"),
        "N2" => Ok("Mark a notification as read"),
        _ => Err(invalid_data(format!(
            "release-candidate summary is missing for {operation_id}"
        ))
        .into()),
    }
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

fn validate_operation_ids(
    openapi: &Value,
    expected_ids: &[&str],
    stage: &str,
) -> Result<(), BoxError> {
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
    let mut expected = expected_ids
        .iter()
        .map(|operation_id| (*operation_id).to_owned())
        .collect::<Vec<_>>();
    expected.sort();
    if actual != expected {
        return Err(invalid_data(format!(
            "{stage} operation IDs differ: expected {expected:?}, got {actual:?}"
        ))
        .into());
    }
    if actual.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(invalid_data(format!("duplicate {stage} operation ID")).into());
    }
    Ok(())
}
