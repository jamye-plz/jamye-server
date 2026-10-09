//! Suspension at realtime ticket issue and at the WebSocket connection, over a real listener.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use jamye_server::{
    adapters::{
        postgres::{auth::PostgresAuthRepository, transactions::SqlxTransactionManager},
        redis::realtime::OsTicketCredentialSource,
    },
    application::{
        auth::{
            AccessIdentity, AccessTokenVerifier, AuthenticationError, GatedAccessTokenVerifier,
        },
        moderation::ModerationAccessGate,
        realtime::{RealtimeTicketService, SystemClock},
        users::UserService,
    },
    ports::realtime::{
        ConversationAuthorizer, RealtimeFuture, RealtimePortError, RealtimeTicketRecord,
        RealtimeTicketStore, TicketConsumeOutcome, TicketDigest, TicketPutOutcome,
    },
    transport::{
        http::{
            auth::AuthVerifierState,
            realtime::{RealtimeHttpState, router as realtime_router},
        },
        realtime::LocalRealtimeHub,
    },
};
use serde_json::Value;
use time::OffsetDateTime;
use tokio::net::TcpListener;
use tokio_tungstenite::{connect_async, tungstenite::Error as WebSocketError};
use uuid::Uuid;

use crate::{
    TestResult,
    support::{Harness, insert_user},
};

#[derive(Default)]
struct MemoryTicketStore {
    records: Mutex<HashMap<String, RealtimeTicketRecord>>,
}

impl RealtimeTicketStore for MemoryTicketStore {
    fn put<'a>(
        &'a self,
        digest: &'a TicketDigest,
        record: &'a RealtimeTicketRecord,
        _ttl: Duration,
    ) -> RealtimeFuture<'a, TicketPutOutcome> {
        Box::pin(async move {
            let key = digest.expose_for_storage().to_owned();
            let mut records = self
                .records
                .lock()
                .map_err(|_| RealtimePortError::Unavailable)?;
            if records.contains_key(&key) {
                return Ok(TicketPutOutcome::Collision);
            }
            records.insert(key, record.clone());
            Ok(TicketPutOutcome::Stored)
        })
    }

    fn consume<'a>(&'a self, digest: &'a TicketDigest) -> RealtimeFuture<'a, TicketConsumeOutcome> {
        Box::pin(async move {
            Ok(self
                .records
                .lock()
                .map_err(|_| RealtimePortError::Unavailable)?
                .remove(digest.expose_for_storage())
                .map(TicketConsumeOutcome::Found)
                .unwrap_or(TicketConsumeOutcome::Missing))
        })
    }
}

/// Verifies `mod-{uuid}` tokens and binds a ten-minute access expiry so tickets can be issued.
struct ExpiringVerifier;

impl AccessTokenVerifier for ExpiringVerifier {
    fn verify(&self, token: &str) -> Result<AccessIdentity, AuthenticationError> {
        let actor_id = token
            .strip_prefix("mod-")
            .and_then(|value| Uuid::try_parse(value).ok())
            .ok_or(AuthenticationError)?;
        Ok(
            AccessIdentity::new(actor_id, Uuid::new_v4(), "moderation-realtime-test")
                .with_access_token_expiry(OffsetDateTime::now_utc() + time::Duration::minutes(10)),
        )
    }
}

struct AlwaysAuthorized;

impl ConversationAuthorizer for AlwaysAuthorized {
    fn is_authorized(&self, _user_id: Uuid, _conversation_id: Uuid) -> RealtimeFuture<'_, bool> {
        Box::pin(async { Ok(true) })
    }
}

async fn issue_ticket(
    client: &reqwest::Client,
    base_url: &str,
    user: Uuid,
) -> TestResult<(u16, Value)> {
    let response = client
        .post(format!("{base_url}/api/v1/realtime/tickets"))
        .bearer_auth(format!("mod-{user}"))
        .header("x-jamye-contract-version", "2")
        .send()
        .await?;
    let status = response.status().as_u16();
    let body: Value = serde_json::from_slice(&response.bytes().await?)?;
    Ok((status, body))
}

#[tokio::test]
async fn a_suspended_account_gets_no_ticket_and_cannot_open_a_connection() -> TestResult {
    let harness = Harness::new().await?;
    let result: TestResult = async {
        let user = insert_user(&harness.pool, "realtime user").await?;
        let users = Arc::new(UserService::new(
            Arc::new(SqlxTransactionManager::new(harness.pool.clone())),
            Arc::new(PostgresAuthRepository::new(harness.pool.clone())),
        ));
        let tickets = Arc::new(RealtimeTicketService::new(
            Arc::new(MemoryTicketStore::default()),
            Arc::new(OsTicketCredentialSource),
            Arc::new(SystemClock),
        ));
        let verifier = Arc::new(GatedAccessTokenVerifier::new(
            Arc::new(ExpiringVerifier),
            Arc::new(ModerationAccessGate::new(harness.repository.clone())),
        ));
        let application = realtime_router(RealtimeHttpState::new(
            tickets,
            LocalRealtimeHub::default(),
            Arc::new(AlwaysAuthorized),
            AuthVerifierState::new(verifier),
            users,
        ));
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let address = listener.local_addr()?;
        let server = tokio::spawn(async move { axum::serve(listener, application).await });
        let base_url = format!("http://{address}");
        let client = reqwest::Client::new();

        // Active account: ticket issue and the WebSocket upgrade work.
        let (status, body) = issue_ticket(&client, &base_url, user).await?;
        assert_eq!(status, 201);
        let working_ticket = body["ticket"].as_str().ok_or("ticket missing")?.to_owned();
        let working_url = format!("ws://{address}/api/v1/realtime/ws?ticket={working_ticket}");
        let (_socket, _) = connect_async(working_url.as_str()).await?;

        // A ticket issued before the suspension must not open a connection afterwards.
        let (status, body) = issue_ticket(&client, &base_url, user).await?;
        assert_eq!(status, 201);
        let earlier_ticket = body["ticket"].as_str().ok_or("ticket missing")?.to_owned();
        harness.service.suspend_user(user, None).await?;

        // Ticket issue: the bearer extractor refuses the suspended account.
        let (status, body) = issue_ticket(&client, &base_url, user).await?;
        assert_eq!(status, 403);
        assert_eq!(body["error"]["code"], "account_suspended");

        // Connection: the upgrade is refused with 403 account_suspended before any frame.
        let earlier_url = format!("ws://{address}/api/v1/realtime/ws?ticket={earlier_ticket}");
        match connect_async(earlier_url.as_str()).await {
            Err(WebSocketError::Http(response)) => {
                assert_eq!(response.status().as_u16(), 403);
                let body = response.body().clone().unwrap_or_default();
                assert!(
                    String::from_utf8_lossy(&body).contains("account_suspended"),
                    "the refusal did not carry account_suspended"
                );
            }
            other => {
                return Err(format!(
                    "a suspended account's ticket opened a connection: {:?}",
                    other.map(|_| ())
                )
                .into());
            }
        }

        // Unsuspend restores both.
        harness.service.unsuspend_user(user).await?;
        let (status, _) = issue_ticket(&client, &base_url, user).await?;
        assert_eq!(status, 201);

        server.abort();
        let _ = server.await;
        Ok(())
    }
    .await;
    harness.finish(result).await
}
