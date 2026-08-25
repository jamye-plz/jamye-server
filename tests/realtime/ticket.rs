use std::{
    collections::HashMap,
    io,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use jamye_server::{
    application::{
        auth::{AccessIdentity, AccessTokenVerifier, AuthenticationError},
        realtime::{RealtimeTicketError, RealtimeTicketService},
    },
    platform::logging::build_json_subscriber,
    ports::realtime::{
        ConversationAuthorizer, RealtimeClock, RealtimeFuture, RealtimePortError,
        RealtimeTicketRecord, RealtimeTicketStore, TicketConsumeOutcome, TicketCredential,
        TicketCredentialSource, TicketDigest, TicketPutOutcome, TicketSecret,
    },
    transport::{
        http::{auth::AuthVerifierState, realtime::RealtimeHttpState},
        realtime::LocalRealtimeHub,
    },
};
use time::OffsetDateTime;
use tower::ServiceExt;
use tracing_subscriber::fmt::MakeWriter;
use uuid::Uuid;

use crate::TestResult;

#[tokio::test]
async fn unsupported_versions_create_no_ticket_material() -> TestResult {
    let harness = Harness::new(OffsetDateTime::UNIX_EPOCH);
    let result = harness
        .service
        .issue(
            &identity(OffsetDateTime::UNIX_EPOCH + time::Duration::minutes(1)),
            "2",
        )
        .await;
    assert_eq!(result, Err(RealtimeTicketError::ContractUpgradeRequired));
    assert_eq!(harness.credentials.generate_calls.load(Ordering::SeqCst), 0);
    assert_eq!(harness.store.len()?, 0);
    Ok(())
}

#[tokio::test]
async fn ticket_is_capped_by_access_expiry_and_consumed_exactly_once() -> TestResult {
    let now = OffsetDateTime::UNIX_EPOCH;
    let expires_at = now + time::Duration::seconds(1);
    let harness = Harness::new(now);
    let issued = harness.service.issue(&identity(expires_at), "1").await?;
    assert_eq!(issued.expires_at, expires_at);
    assert_eq!(harness.store.last_ttl()?, Some(Duration::from_secs(1)));

    let first = harness.service.consume(&issued.ticket);
    let second = harness.service.consume(&issued.ticket);
    let (first, second) = tokio::join!(first, second);
    let outcomes = [first.is_ok(), second.is_ok()];
    assert_eq!(outcomes.iter().filter(|value| **value).count(), 1);
    assert_eq!(outcomes.iter().filter(|value| !**value).count(), 1);
    assert_eq!(harness.store.len()?, 0);
    Ok(())
}

#[tokio::test]
async fn ticket_lifetime_is_capped_at_thirty_seconds_for_both_supported_versions() -> TestResult {
    let now = OffsetDateTime::UNIX_EPOCH;
    for contract_version in ["1", "0"] {
        let harness = Harness::new(now);
        let issued = harness
            .service
            .issue(
                &identity(now + time::Duration::minutes(5)),
                contract_version,
            )
            .await?;
        assert_eq!(issued.expires_at, now + time::Duration::seconds(30));
        assert_eq!(harness.store.last_ttl()?, Some(Duration::from_secs(30)));

        // Under selected D13=A, logout revokes refresh authority only. There is no immediate
        // revocation lookup here, so a ticket remains consumable until the bound access exp.
        assert!(harness.service.consume(&issued.ticket).await.is_ok());
    }
    Ok(())
}

#[tokio::test]
async fn non_positive_access_lifetime_creates_no_ticket_material() -> TestResult {
    let now = OffsetDateTime::UNIX_EPOCH;
    let harness = Harness::new(now);
    assert_eq!(
        harness.service.issue(&identity(now), "1").await,
        Err(RealtimeTicketError::AuthenticationRequired)
    );
    assert_eq!(harness.credentials.generate_calls.load(Ordering::SeqCst), 0);
    assert_eq!(harness.store.len()?, 0);
    Ok(())
}

#[tokio::test]
async fn consumed_after_the_bound_expiry_is_indistinguishable_from_missing() -> TestResult {
    let now = OffsetDateTime::UNIX_EPOCH;
    let expires_at = now + time::Duration::seconds(5);
    let harness = Harness::new(now);
    let issued = harness.service.issue(&identity(expires_at), "0").await?;
    harness.clock.set(expires_at);
    assert_eq!(
        harness.service.consume(&issued.ticket).await,
        Err(RealtimeTicketError::AuthenticationFailed)
    );
    assert_eq!(
        harness.service.consume(&issued.ticket).await,
        Err(RealtimeTicketError::AuthenticationFailed)
    );
    Ok(())
}

#[test]
fn ticket_secret_and_digest_debug_output_is_redacted() {
    assert_eq!(
        format!("{:?}", TicketSecret::new("raw".to_owned())),
        "[REDACTED]"
    );
    assert_eq!(
        format!("{:?}", TicketDigest::new("digest".to_owned())),
        "[REDACTED]"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn structured_ticket_logs_exclude_raw_and_digest_credentials() -> TestResult {
    let harness = Harness::new(OffsetDateTime::UNIX_EPOCH);
    let identity = identity(OffsetDateTime::UNIX_EPOCH + time::Duration::minutes(1));
    let raw_ticket = format!("{:064x}", 1);
    let digest = format!("digest:{raw_ticket}");
    let writer = SharedWriter::default();
    let output = writer.clone();
    let subscriber = build_json_subscriber(writer, "info")?;
    let _guard = tracing::subscriber::set_default(subscriber);
    let state = RealtimeHttpState::new(
        Arc::new(harness.service.clone()),
        LocalRealtimeHub::default(),
        Arc::new(AlwaysAuthorized),
        AuthVerifierState::new(Arc::new(StaticVerifier(identity))),
    );
    let response = jamye_server::transport::http::realtime::router(state)
        .oneshot(
            Request::post("/api/v1/realtime/tickets")
                .header("authorization", "Bearer opaque-test-token")
                .header("x-jamye-contract-version", "1")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::CREATED);

    let logs = output.snapshot()?;
    assert!(logs.contains("realtime ticket issued"));
    for forbidden in [raw_ticket.as_str(), digest.as_str(), "opaque-test-token"] {
        assert!(!logs.contains(forbidden), "logs leaked {forbidden}");
    }
    Ok(())
}

struct Harness {
    service: RealtimeTicketService,
    store: Arc<MemoryTicketStore>,
    credentials: Arc<DeterministicCredentials>,
    clock: Arc<TestClock>,
}

impl Harness {
    fn new(now: OffsetDateTime) -> Self {
        let store = Arc::new(MemoryTicketStore::default());
        let credentials = Arc::new(DeterministicCredentials::default());
        let clock = Arc::new(TestClock::new(now));
        let service = RealtimeTicketService::new(store.clone(), credentials.clone(), clock.clone());
        Self {
            service,
            store,
            credentials,
            clock,
        }
    }
}

fn identity(expires_at: OffsetDateTime) -> AccessIdentity {
    AccessIdentity::new(Uuid::new_v4(), Uuid::new_v4(), "task-4b-test")
        .with_access_token_expiry(expires_at)
}

#[derive(Default)]
struct MemoryTicketStore {
    records: Mutex<HashMap<String, RealtimeTicketRecord>>,
    last_ttl: Mutex<Option<Duration>>,
}

impl MemoryTicketStore {
    fn len(&self) -> Result<usize, RealtimePortError> {
        self.records
            .lock()
            .map(|records| records.len())
            .map_err(|_| RealtimePortError::Unavailable)
    }

    fn last_ttl(&self) -> Result<Option<Duration>, RealtimePortError> {
        self.last_ttl
            .lock()
            .map(|ttl| *ttl)
            .map_err(|_| RealtimePortError::Unavailable)
    }
}

impl RealtimeTicketStore for MemoryTicketStore {
    fn put<'a>(
        &'a self,
        digest: &'a TicketDigest,
        record: &'a RealtimeTicketRecord,
        ttl: Duration,
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
            *self
                .last_ttl
                .lock()
                .map_err(|_| RealtimePortError::Unavailable)? = Some(ttl);
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

#[derive(Default)]
struct DeterministicCredentials {
    generate_calls: AtomicUsize,
}

impl TicketCredentialSource for DeterministicCredentials {
    fn generate(&self) -> Result<TicketCredential, RealtimePortError> {
        let sequence = self.generate_calls.fetch_add(1, Ordering::SeqCst) + 1;
        let raw = format!("{sequence:064x}");
        Ok(TicketCredential {
            secret: TicketSecret::new(raw.clone()),
            digest: TicketDigest::new(format!("digest:{raw}")),
        })
    }

    fn digest(&self, raw_ticket: &str) -> Result<TicketDigest, RealtimePortError> {
        Ok(TicketDigest::new(format!("digest:{raw_ticket}")))
    }
}

struct TestClock(Mutex<OffsetDateTime>);

impl TestClock {
    fn new(now: OffsetDateTime) -> Self {
        Self(Mutex::new(now))
    }

    fn set(&self, now: OffsetDateTime) {
        match self.0.lock() {
            Ok(mut value) => *value = now,
            Err(poisoned) => *poisoned.into_inner() = now,
        }
    }
}

impl RealtimeClock for TestClock {
    fn now(&self) -> OffsetDateTime {
        match self.0.lock() {
            Ok(value) => *value,
            Err(poisoned) => *poisoned.into_inner(),
        }
    }
}

struct StaticVerifier(AccessIdentity);

impl AccessTokenVerifier for StaticVerifier {
    fn verify(&self, _token: &str) -> Result<AccessIdentity, AuthenticationError> {
        Ok(self.0.clone())
    }
}

struct AlwaysAuthorized;

impl ConversationAuthorizer for AlwaysAuthorized {
    fn is_authorized(&self, _user_id: Uuid, _conversation_id: Uuid) -> RealtimeFuture<'_, bool> {
        Box::pin(async { Ok(true) })
    }
}

#[derive(Clone, Default)]
struct SharedWriter(Arc<Mutex<Vec<u8>>>);

impl SharedWriter {
    fn snapshot(&self) -> io::Result<String> {
        let bytes = self
            .0
            .lock()
            .map_err(|_| io::Error::other("log writer lock poisoned"))?
            .clone();
        String::from_utf8(bytes).map_err(io::Error::other)
    }
}

impl<'a> MakeWriter<'a> for SharedWriter {
    type Writer = SharedWriterGuard;

    fn make_writer(&'a self) -> Self::Writer {
        SharedWriterGuard(self.0.clone())
    }
}

struct SharedWriterGuard(Arc<Mutex<Vec<u8>>>);

impl io::Write for SharedWriterGuard {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_| io::Error::other("log writer lock poisoned"))?
            .extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
