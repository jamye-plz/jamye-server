use std::{error::Error, fmt, sync::Arc, time::Duration};

use time::{OffsetDateTime, SignedDuration};

use crate::{
    application::{
        auth::AccessIdentity,
        messaging::{CURRENT_CONTRACT_VERSION, PREVIOUS_CONTRACT_VERSION},
    },
    ports::realtime::{
        RealtimeClock, RealtimePortError, RealtimeTicketRecord, RealtimeTicketStore,
        TicketConsumeOutcome, TicketCredentialSource, TicketPutOutcome,
    },
};

const MAX_TICKET_LIFETIME: SignedDuration = SignedDuration::seconds(30);
const MAX_CREDENTIAL_ATTEMPTS: usize = 3;

#[derive(Clone)]
pub struct RealtimeTicketService {
    store: Arc<dyn RealtimeTicketStore>,
    credentials: Arc<dyn TicketCredentialSource>,
    clock: Arc<dyn RealtimeClock>,
}

impl RealtimeTicketService {
    pub fn new(
        store: Arc<dyn RealtimeTicketStore>,
        credentials: Arc<dyn TicketCredentialSource>,
        clock: Arc<dyn RealtimeClock>,
    ) -> Self {
        Self {
            store,
            credentials,
            clock,
        }
    }

    pub async fn issue(
        &self,
        identity: &AccessIdentity,
        contract_version: &str,
    ) -> Result<IssuedRealtimeTicket, RealtimeTicketError> {
        validate_contract_version(contract_version)?;
        let now = self.clock.now();
        let access_token_expires_at = identity
            .access_token_expires_at
            .ok_or(RealtimeTicketError::AuthenticationRequired)?;
        let expires_at = access_token_expires_at.min(
            now.checked_add(MAX_TICKET_LIFETIME)
                .ok_or(RealtimeTicketError::Unavailable)?,
        );
        let ttl = positive_ttl(now, expires_at)?;
        let record = RealtimeTicketRecord {
            user_id: identity.user_id,
            session_id: identity.session_id,
            contract_version: contract_version.to_owned(),
            access_token_expires_at,
        };

        for _ in 0..MAX_CREDENTIAL_ATTEMPTS {
            let credential = self.credentials.generate().map_err(map_port_error)?;
            match self
                .store
                .put(&credential.digest, &record, ttl)
                .await
                .map_err(map_port_error)?
            {
                TicketPutOutcome::Stored => {
                    return Ok(IssuedRealtimeTicket {
                        ticket: credential.secret.expose_secret().to_owned(),
                        expires_at,
                        contract_version: contract_version.to_owned(),
                    });
                }
                TicketPutOutcome::Collision => {}
            }
        }
        Err(RealtimeTicketError::Unavailable)
    }

    pub async fn consume(&self, raw_ticket: &str) -> Result<RealtimeSession, RealtimeTicketError> {
        if raw_ticket.is_empty() || raw_ticket.chars().any(char::is_whitespace) {
            return Err(RealtimeTicketError::AuthenticationFailed);
        }
        let digest = self
            .credentials
            .digest(raw_ticket)
            .map_err(|_| RealtimeTicketError::AuthenticationFailed)?;
        let record = match self.store.consume(&digest).await.map_err(map_port_error)? {
            TicketConsumeOutcome::Found(record) => record,
            TicketConsumeOutcome::Missing => {
                return Err(RealtimeTicketError::AuthenticationFailed);
            }
        };
        if record.access_token_expires_at <= self.clock.now()
            || validate_contract_version(&record.contract_version).is_err()
        {
            return Err(RealtimeTicketError::AuthenticationFailed);
        }
        Ok(RealtimeSession {
            user_id: record.user_id,
            session_id: record.session_id,
            contract_version: record.contract_version,
            access_token_expires_at: record.access_token_expires_at,
        })
    }
}

fn validate_contract_version(contract_version: &str) -> Result<(), RealtimeTicketError> {
    if matches!(
        contract_version,
        CURRENT_CONTRACT_VERSION | PREVIOUS_CONTRACT_VERSION
    ) {
        Ok(())
    } else {
        Err(RealtimeTicketError::ContractUpgradeRequired)
    }
}

fn positive_ttl(
    now: OffsetDateTime,
    expires_at: OffsetDateTime,
) -> Result<Duration, RealtimeTicketError> {
    let milliseconds = (expires_at - now).whole_milliseconds();
    let milliseconds = u64::try_from(milliseconds)
        .ok()
        .filter(|milliseconds| *milliseconds > 0)
        .ok_or(RealtimeTicketError::AuthenticationRequired)?;
    Ok(Duration::from_millis(milliseconds))
}

fn map_port_error(error: RealtimePortError) -> RealtimeTicketError {
    match error {
        RealtimePortError::Unavailable | RealtimePortError::InvalidData => {
            RealtimeTicketError::Unavailable
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IssuedRealtimeTicket {
    pub ticket: String,
    pub expires_at: OffsetDateTime,
    pub contract_version: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RealtimeSession {
    pub user_id: uuid::Uuid,
    pub session_id: uuid::Uuid,
    pub contract_version: String,
    pub access_token_expires_at: OffsetDateTime,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RealtimeTicketError {
    AuthenticationRequired,
    AuthenticationFailed,
    ContractUpgradeRequired,
    Unavailable,
}

impl fmt::Display for RealtimeTicketError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("realtime ticket operation failed")
    }
}

impl Error for RealtimeTicketError {}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl RealtimeClock for SystemClock {
    fn now(&self) -> OffsetDateTime {
        OffsetDateTime::now_utc()
    }
}
