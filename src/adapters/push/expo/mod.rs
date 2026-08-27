//! Expo Push Service HTTP adapter.

use std::{fmt, net::IpAddr};

use reqwest::{Client, redirect::Policy};
use url::Url;

use crate::ports::push::{
    PushProvider, PushProviderError, PushProviderFuture, PushProviderOutcome, PushProviderRequest,
};

pub const EXPO_PUSH_SEND_URL: &str = "https://exp.host/--/api/v2/push/send";

const EXPO_PUSH_SEND_PATH: &str = "/--/api/v2/push/send";
const MAX_ACCESS_TOKEN_BYTES: usize = 4096;

#[derive(Clone)]
pub struct ExpoPushProvider {
    endpoint: Url,
    access_token: Option<SensitiveValue>,
    client: Client,
}

impl ExpoPushProvider {
    pub fn new(
        endpoint: impl AsRef<str>,
        access_token: Option<String>,
    ) -> Result<Self, ExpoPushProviderConfigError> {
        let endpoint_value = endpoint.as_ref();
        let endpoint = Url::parse(endpoint_value)
            .map_err(|_| ExpoPushProviderConfigError::InvalidConfiguration)?;
        if !valid_endpoint(&endpoint) {
            return Err(ExpoPushProviderConfigError::InvalidConfiguration);
        }
        let access_token = access_token
            .map(|access_token| {
                valid_access_token_configuration(&access_token)
                    .then_some(access_token)
                    .ok_or(ExpoPushProviderConfigError::InvalidConfiguration)
            })
            .transpose()?
            .map(SensitiveValue);
        let client = Client::builder()
            .redirect(Policy::none())
            .build()
            .map_err(|_| ExpoPushProviderConfigError::InvalidConfiguration)?;
        Ok(Self {
            endpoint,
            access_token,
            client,
        })
    }

    async fn send_request(
        &self,
        request: &PushProviderRequest,
    ) -> Result<PushProviderOutcome, PushProviderError> {
        let message = ExpoMessage {
            to: request.destination.token(),
            data: ExpoRoute {
                notification_type: request.route.notification_type.as_str(),
                notification_id: request.route.notification_id,
                conversation_id: request.route.conversation_id,
                message_id: request.route.message_id,
            },
            body: request.preview.as_deref(),
        };
        let mut builder = self
            .client
            .post(self.endpoint.clone())
            .header(reqwest::header::ACCEPT, "application/json")
            .json(&message);
        if let Some(access_token) = &self.access_token {
            builder = builder.bearer_auth(access_token.expose());
        }
        let response = builder
            .send()
            .await
            .map_err(|_| provider_unavailable("network"))?;
        let status = response.status();
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS
            || status == reqwest::StatusCode::REQUEST_TIMEOUT
            || status.is_server_error()
        {
            return Err(provider_unavailable("http_retryable"));
        }
        if !status.is_success() {
            return Err(provider_rejected("http_terminal"));
        }
        let response = response
            .json::<ExpoSendResponse>()
            .await
            .map_err(|_| provider_unavailable("response_decode"))?;
        classify_ticket(response.data)
    }
}

impl fmt::Debug for ExpoPushProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ExpoPushProvider")
            .field("endpoint", &self.endpoint)
            .field("access_token", &self.access_token)
            .finish()
    }
}

impl PushProvider for ExpoPushProvider {
    fn send<'a>(&'a self, request: &'a PushProviderRequest) -> PushProviderFuture<'a> {
        Box::pin(self.send_request(request))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExpoPushProviderConfigError {
    InvalidConfiguration,
}

impl fmt::Display for ExpoPushProviderConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("invalid Expo push provider configuration")
    }
}

impl std::error::Error for ExpoPushProviderConfigError {}

#[derive(Clone)]
struct SensitiveValue(String);

impl SensitiveValue {
    fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SensitiveValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[REDACTED]")
    }
}

fn valid_endpoint(endpoint: &Url) -> bool {
    let no_private_url_parts = endpoint.username().is_empty()
        && endpoint.password().is_none()
        && endpoint.query().is_none()
        && endpoint.fragment().is_none();
    let secure_or_loopback = endpoint.scheme() == "https"
        || (endpoint.scheme() == "http"
            && endpoint.host_str().is_some_and(|host| {
                host.eq_ignore_ascii_case("localhost")
                    || host
                        .parse::<IpAddr>()
                        .is_ok_and(|address| address.is_loopback())
            }));
    no_private_url_parts
        && secure_or_loopback
        && endpoint.host().is_some()
        && endpoint.path() == EXPO_PUSH_SEND_PATH
}

pub(crate) fn valid_endpoint_configuration(endpoint: &str) -> bool {
    Url::parse(endpoint).as_ref().is_ok_and(valid_endpoint)
}

pub(crate) fn valid_access_token_configuration(access_token: &str) -> bool {
    !access_token.is_empty()
        && access_token.len() <= MAX_ACCESS_TOKEN_BYTES
        && access_token.bytes().all(|byte| byte.is_ascii_graphic())
}

#[derive(serde::Serialize)]
struct ExpoMessage<'a> {
    to: &'a str,
    data: ExpoRoute,
    #[serde(skip_serializing_if = "Option::is_none")]
    body: Option<&'a str>,
}

#[derive(serde::Serialize)]
struct ExpoRoute {
    #[serde(rename = "type")]
    notification_type: &'static str,
    notification_id: uuid::Uuid,
    conversation_id: uuid::Uuid,
    message_id: Option<uuid::Uuid>,
}

#[derive(serde::Deserialize)]
struct ExpoSendResponse {
    data: ExpoTicket,
}

#[derive(serde::Deserialize)]
struct ExpoTicket {
    status: ExpoTicketStatus,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    details: Option<ExpoTicketDetails>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "lowercase")]
enum ExpoTicketStatus {
    Ok,
    Error,
}

#[derive(serde::Deserialize)]
struct ExpoTicketDetails {
    #[serde(default)]
    error: Option<String>,
}

fn classify_ticket(ticket: ExpoTicket) -> Result<PushProviderOutcome, PushProviderError> {
    match ticket.status {
        ExpoTicketStatus::Ok
            if ticket
                .id
                .as_deref()
                .is_some_and(|ticket_id| !ticket_id.is_empty()) =>
        {
            tracing::info!(
                target: "jamye_server",
                provider = "expo",
                outcome = "accepted",
                "expo_push_accepted"
            );
            Ok(PushProviderOutcome::Accepted)
        }
        ExpoTicketStatus::Ok => Err(provider_unavailable("invalid_ok_ticket")),
        ExpoTicketStatus::Error
            if ticket
                .details
                .as_ref()
                .and_then(|details| details.error.as_deref())
                == Some("DeviceNotRegistered") =>
        {
            tracing::warn!(
                target: "jamye_server",
                provider = "expo",
                outcome = "device_not_registered",
                "expo_push_device_not_registered"
            );
            Ok(PushProviderOutcome::DeviceNotRegistered)
        }
        ExpoTicketStatus::Error => Err(provider_rejected("ticket_error")),
    }
}

fn provider_unavailable(failure_kind: &'static str) -> PushProviderError {
    tracing::warn!(
        target: "jamye_server",
        dependency = "expo_push",
        failure_kind,
        "expo_push_unavailable"
    );
    PushProviderError::Unavailable
}

fn provider_rejected(failure_kind: &'static str) -> PushProviderError {
    tracing::warn!(
        target: "jamye_server",
        dependency = "expo_push",
        failure_kind,
        "expo_push_rejected"
    );
    PushProviderError::Rejected
}
