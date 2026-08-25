use std::{env, io, sync::Arc, time::Duration};

use jamye_server::{
    adapters::redis::realtime::{OsTicketCredentialSource, RedisRealtimeAdapter},
    application::{
        auth::AccessIdentity,
        realtime::{RealtimeTicketError, RealtimeTicketService, SystemClock},
    },
    domain::messaging::{CanonicalMessage, MessageCreatedEvent, MessageCreatedType, MessageKind},
    ports::realtime::{ClaimedOutboxEvent, RealtimeEventPublisher, RealtimePortError},
};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use url::Url;
use uuid::Uuid;

use crate::TestResult;

#[tokio::test]
async fn redis_stores_only_the_digest_and_getdel_allows_one_consumer() -> TestResult {
    let redis_url = guarded_redis_url()?;
    let adapter = Arc::new(RedisRealtimeAdapter::new(&redis_url)?);
    let tickets = RealtimeTicketService::new(
        adapter,
        Arc::new(OsTicketCredentialSource),
        Arc::new(SystemClock),
    );
    let identity = AccessIdentity::new(Uuid::new_v4(), Uuid::new_v4(), "task-4b-redis")
        .with_access_token_expiry(OffsetDateTime::now_utc() + time::Duration::seconds(20));
    let issued = tickets.issue(&identity, "1").await?;
    assert_eq!(issued.ticket.len(), 64);
    assert!(issued.ticket.bytes().all(|byte| byte.is_ascii_hexdigit()));

    let digest = encode_hex(&Sha256::digest(issued.ticket.as_bytes()));
    let mut connection = redis::Client::open(redis_url.as_str())?
        .get_multiplexed_async_connection()
        .await?;
    let raw_key_exists = redis::cmd("EXISTS")
        .arg(format!("jamye:realtime:ticket:{}", issued.ticket))
        .query_async::<i64>(&mut connection)
        .await?;
    let digest_key_exists = redis::cmd("EXISTS")
        .arg(format!("jamye:realtime:ticket:{digest}"))
        .query_async::<i64>(&mut connection)
        .await?;
    assert_eq!(raw_key_exists, 0);
    assert_eq!(digest_key_exists, 1);

    let first = tickets.consume(&issued.ticket);
    let second = tickets.consume(&issued.ticket);
    let (first, second) = tokio::join!(first, second);
    assert_eq!(
        [first.is_ok(), second.is_ok()]
            .into_iter()
            .filter(|outcome| *outcome)
            .count(),
        1
    );
    assert_eq!(
        [first, second]
            .into_iter()
            .filter(|outcome| { matches!(outcome, Err(RealtimeTicketError::AuthenticationFailed)) })
            .count(),
        1
    );
    Ok(())
}

#[tokio::test]
async fn redis_pubsub_preserves_the_stable_event_id_and_canonical_payload() -> TestResult {
    let redis_url = guarded_redis_url()?;
    let adapter = RedisRealtimeAdapter::new(&redis_url)?;
    let mut subscriber = adapter.event_subscriber().await?;
    let event = message_event();
    let claim = ClaimedOutboxEvent {
        id: Uuid::new_v4(),
        conversation_id: event.conversation_id,
        event_id: event.event_id,
        payload: serde_json::to_value(&event)?,
        claim_owner: "task-4b-redis".to_owned(),
        claim_generation: 1,
        claim_expires_at: OffsetDateTime::now_utc() + time::Duration::seconds(5),
        attempt_count: 0,
    };

    adapter.publish(&claim).await?;
    let expected_event_id = event.event_id;
    let received = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let received = subscriber
                .next_event()
                .await?
                .ok_or(RealtimePortError::Unavailable)?;
            if received.event_id == expected_event_id {
                return Ok::<_, RealtimePortError>(received);
            }
        }
    })
    .await??;
    assert_eq!(received, event);
    Ok(())
}

fn guarded_redis_url() -> TestResult<String> {
    if env::var("JAMYE_ENVIRONMENT").as_deref() != Ok("test") {
        return Err(
            io::Error::other("Redis integration tests require JAMYE_ENVIRONMENT=test").into(),
        );
    }
    let redis_url = env::var("REDIS_URL")
        .map_err(|_| io::Error::other("REDIS_URL is required for Redis integration tests"))?;
    let parsed = Url::parse(&redis_url)?;
    if parsed.scheme() != "redis"
        || !matches!(parsed.host_str(), Some("127.0.0.1" | "localhost" | "::1"))
    {
        return Err(io::Error::other(
            "Redis integration tests accept only a loopback redis:// URL",
        )
        .into());
    }
    Ok(redis_url)
}

fn message_event() -> MessageCreatedEvent {
    let conversation_id = Uuid::new_v4();
    MessageCreatedEvent {
        version: 1,
        event_type: MessageCreatedType::MessageCreated,
        event_id: Uuid::new_v4(),
        conversation_id,
        cursor: "42".to_owned(),
        occurred_at: OffsetDateTime::UNIX_EPOCH,
        data: CanonicalMessage {
            id: Uuid::new_v4(),
            chatroom_id: conversation_id,
            sender_id: Some(Uuid::new_v4()),
            client_msg_id: Some(Uuid::new_v4()),
            body: Some("task-4b redis".to_owned()),
            message_type: MessageKind::User,
            created_at: OffsetDateTime::UNIX_EPOCH,
            media: Vec::new(),
        },
    }
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}
