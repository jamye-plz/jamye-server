//! Redis ticket storage, Pub/Sub publishing, and event subscription.

use std::time::Duration;

use futures_util::StreamExt;
use redis::{Client, aio::PubSub};
use sha2::{Digest, Sha256};

use crate::{
    domain::messaging::MessageCreatedEvent,
    ports::realtime::{
        ClaimedOutboxEvent, RealtimeEventPublisher, RealtimeFuture, RealtimePortError,
        RealtimeTicketRecord, RealtimeTicketStore, TicketConsumeOutcome, TicketCredential,
        TicketCredentialSource, TicketDigest, TicketPutOutcome, TicketSecret,
    },
};

const TICKET_KEY_PREFIX: &str = "jamye:realtime:ticket:";
const CONVERSATION_CHANNEL_PREFIX: &str = "jamye:conversation:";
const CONVERSATION_PATTERN: &str = "jamye:conversation:*";
const TICKET_BYTES: usize = 32;

#[derive(Clone)]
pub struct RedisRealtimeAdapter {
    client: Client,
}

impl RedisRealtimeAdapter {
    pub fn new(redis_url: &str) -> Result<Self, RealtimePortError> {
        Client::open(redis_url)
            .map(|client| Self { client })
            .map_err(|_| redis_error("configure"))
    }

    pub async fn event_subscriber(&self) -> Result<RedisEventSubscriber, RealtimePortError> {
        let mut pubsub = self
            .client
            .get_async_pubsub()
            .await
            .map_err(|_| redis_error("subscriber_connect"))?;
        pubsub
            .psubscribe(CONVERSATION_PATTERN)
            .await
            .map_err(|_| redis_error("subscriber_subscribe"))?;
        Ok(RedisEventSubscriber { pubsub })
    }

    async fn store_ticket(
        &self,
        digest: &TicketDigest,
        record: &RealtimeTicketRecord,
        ttl: Duration,
    ) -> Result<TicketPutOutcome, RealtimePortError> {
        let ttl_milliseconds = u64::try_from(ttl.as_millis())
            .ok()
            .filter(|ttl| *ttl > 0)
            .ok_or(RealtimePortError::InvalidData)?;
        let payload = serde_json::to_string(record).map_err(|_| RealtimePortError::InvalidData)?;
        let mut connection = self
            .client
            .get_multiplexed_async_connection()
            .await
            .map_err(|_| redis_error("ticket_connect"))?;
        let stored = redis::cmd("SET")
            .arg(ticket_key(digest))
            .arg(payload)
            .arg("NX")
            .arg("PX")
            .arg(ttl_milliseconds)
            .query_async::<Option<String>>(&mut connection)
            .await
            .map_err(|_| redis_error("ticket_store"))?;
        Ok(if stored.is_some() {
            TicketPutOutcome::Stored
        } else {
            TicketPutOutcome::Collision
        })
    }

    async fn consume_ticket(
        &self,
        digest: &TicketDigest,
    ) -> Result<TicketConsumeOutcome, RealtimePortError> {
        let mut connection = self
            .client
            .get_multiplexed_async_connection()
            .await
            .map_err(|_| redis_error("ticket_connect"))?;
        let payload = redis::cmd("GETDEL")
            .arg(ticket_key(digest))
            .query_async::<Option<String>>(&mut connection)
            .await
            .map_err(|_| redis_error("ticket_consume"))?;
        payload.map_or(Ok(TicketConsumeOutcome::Missing), |payload| {
            serde_json::from_str(&payload)
                .map(TicketConsumeOutcome::Found)
                .map_err(|_| RealtimePortError::InvalidData)
        })
    }

    async fn publish_event(&self, event: &ClaimedOutboxEvent) -> Result<(), RealtimePortError> {
        let channel = format!("{CONVERSATION_CHANNEL_PREFIX}{}", event.conversation_id);
        let payload =
            serde_json::to_string(&event.payload).map_err(|_| RealtimePortError::InvalidData)?;
        let mut connection = self
            .client
            .get_multiplexed_async_connection()
            .await
            .map_err(|_| redis_error("publisher_connect"))?;
        redis::cmd("PUBLISH")
            .arg(channel)
            .arg(payload)
            .query_async::<i64>(&mut connection)
            .await
            .map(|_| ())
            .map_err(|_| redis_error("publish"))
    }
}

impl RealtimeTicketStore for RedisRealtimeAdapter {
    fn put<'a>(
        &'a self,
        digest: &'a TicketDigest,
        record: &'a RealtimeTicketRecord,
        ttl: Duration,
    ) -> RealtimeFuture<'a, TicketPutOutcome> {
        Box::pin(self.store_ticket(digest, record, ttl))
    }

    fn consume<'a>(&'a self, digest: &'a TicketDigest) -> RealtimeFuture<'a, TicketConsumeOutcome> {
        Box::pin(self.consume_ticket(digest))
    }
}

impl RealtimeEventPublisher for RedisRealtimeAdapter {
    fn publish<'a>(&'a self, event: &'a ClaimedOutboxEvent) -> RealtimeFuture<'a, ()> {
        Box::pin(self.publish_event(event))
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct OsTicketCredentialSource;

impl TicketCredentialSource for OsTicketCredentialSource {
    fn generate(&self) -> Result<TicketCredential, RealtimePortError> {
        let mut bytes = [0_u8; TICKET_BYTES];
        getrandom::fill(&mut bytes).map_err(|_| RealtimePortError::Unavailable)?;
        let raw = encode_hex(&bytes);
        let digest = ticket_digest(&raw);
        Ok(TicketCredential {
            secret: TicketSecret::new(raw),
            digest,
        })
    }

    fn digest(&self, raw_ticket: &str) -> Result<TicketDigest, RealtimePortError> {
        if raw_ticket.len() != TICKET_BYTES * 2
            || !raw_ticket.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(RealtimePortError::InvalidData);
        }
        Ok(ticket_digest(raw_ticket))
    }
}

pub struct RedisEventSubscriber {
    pubsub: PubSub,
}

impl RedisEventSubscriber {
    pub async fn next_event(&mut self) -> Result<Option<MessageCreatedEvent>, RealtimePortError> {
        let mut stream = self.pubsub.on_message();
        let Some(message) = stream.next().await else {
            return Ok(None);
        };
        let payload = message
            .get_payload::<String>()
            .map_err(|_| RealtimePortError::InvalidData)?;
        serde_json::from_str(&payload)
            .map(Some)
            .map_err(|_| RealtimePortError::InvalidData)
    }
}

fn ticket_key(digest: &TicketDigest) -> String {
    format!("{TICKET_KEY_PREFIX}{}", digest.expose_for_storage())
}

fn ticket_digest(raw_ticket: &str) -> TicketDigest {
    let digest = Sha256::digest(raw_ticket.as_bytes());
    TicketDigest::new(encode_hex(&digest))
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

fn redis_error(operation: &'static str) -> RealtimePortError {
    tracing::warn!(
        dependency = "redis",
        failure_kind = "realtime",
        operation,
        "Redis realtime operation failed"
    );
    RealtimePortError::Unavailable
}
