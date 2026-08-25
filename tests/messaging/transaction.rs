use std::{
    any::Any,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use jamye_server::{
    application::{
        auth::AccessIdentity,
        messaging::{MessagingError, MessagingService, SendMessageInput},
    },
    domain::messaging::{CanonicalMessage, EventPage, MessageKind, SendMessageCommand},
    ports::{
        messaging::{
            DeltaQuery, MessagingFuture, MessagingRepository, MessagingRepositoryError,
            PersistMessageOutcome,
        },
        transactions::{
            BoxTransactionHandle, TransactionFuture, TransactionHandle, TransactionManager,
        },
    },
};
use time::OffsetDateTime;
use uuid::Uuid;

#[tokio::test]
async fn application_owns_one_commit_and_rolls_back_repository_failure() {
    let committed = Harness::new(Ok(PersistMessageOutcome::Created(message())));
    let result = committed.service.send_message(&identity(), input()).await;
    assert!(result.is_ok());
    assert_eq!(committed.transactions.counts(), (1, 1, 0));
    assert_eq!(committed.repository.handles.load(Ordering::SeqCst), 1);

    let rolled_back = Harness::new(Err(MessagingRepositoryError::DatabaseUnavailable));
    let result = rolled_back.service.send_message(&identity(), input()).await;
    assert_eq!(result, Err(MessagingError::DatabaseUnavailable));
    assert_eq!(rolled_back.transactions.counts(), (1, 0, 1));
    assert_eq!(rolled_back.repository.handles.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn validation_failure_never_opens_a_transaction() {
    let harness = Harness::new(Ok(PersistMessageOutcome::Created(message())));
    let mut invalid = input();
    invalid.idempotency_key = Some(Uuid::new_v4());

    let result = harness.service.send_message(&identity(), invalid).await;
    assert_eq!(result, Err(MessagingError::IdempotencyKeyMismatch));
    assert_eq!(harness.transactions.counts(), (0, 0, 0));
    assert_eq!(harness.repository.handles.load(Ordering::SeqCst), 0);
}

struct Harness {
    service: MessagingService,
    transactions: Arc<RecordingTransactions>,
    repository: Arc<RecordingRepository>,
}

impl Harness {
    fn new(result: Result<PersistMessageOutcome, MessagingRepositoryError>) -> Self {
        let transactions = Arc::new(RecordingTransactions::default());
        let repository = Arc::new(RecordingRepository {
            result,
            handles: AtomicUsize::new(0),
        });
        let service = MessagingService::new(transactions.clone(), repository.clone());
        Self {
            service,
            transactions,
            repository,
        }
    }
}

#[derive(Default)]
struct RecordingTransactions {
    begin: AtomicUsize,
    commit: AtomicUsize,
    rollback: AtomicUsize,
}

impl RecordingTransactions {
    fn counts(&self) -> (usize, usize, usize) {
        (
            self.begin.load(Ordering::SeqCst),
            self.commit.load(Ordering::SeqCst),
            self.rollback.load(Ordering::SeqCst),
        )
    }
}

impl TransactionManager for RecordingTransactions {
    fn begin(&self) -> TransactionFuture<'_, BoxTransactionHandle> {
        self.begin.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(Box::new(RecordingHandle) as BoxTransactionHandle) })
    }

    fn commit<'a>(&'a self, handle: BoxTransactionHandle) -> TransactionFuture<'a, ()> {
        assert!(handle.into_any().downcast::<RecordingHandle>().is_ok());
        self.commit.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(()) })
    }

    fn rollback<'a>(&'a self, handle: BoxTransactionHandle) -> TransactionFuture<'a, ()> {
        assert!(handle.into_any().downcast::<RecordingHandle>().is_ok());
        self.rollback.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(()) })
    }
}

struct RecordingHandle;

impl TransactionHandle for RecordingHandle {
    fn as_any_mut(&mut self) -> &mut (dyn Any + Send) {
        self
    }

    fn into_any(self: Box<Self>) -> Box<dyn Any + Send> {
        self
    }
}

struct RecordingRepository {
    result: Result<PersistMessageOutcome, MessagingRepositoryError>,
    handles: AtomicUsize,
}

impl MessagingRepository for RecordingRepository {
    fn send<'a>(
        &'a self,
        handle: &'a mut dyn TransactionHandle,
        _command: &'a SendMessageCommand,
    ) -> MessagingFuture<'a, PersistMessageOutcome> {
        assert!(
            handle
                .as_any_mut()
                .downcast_mut::<RecordingHandle>()
                .is_some()
        );
        self.handles.fetch_add(1, Ordering::SeqCst);
        let result = self.result.clone();
        Box::pin(async move { result })
    }

    fn events(&self, _query: DeltaQuery) -> MessagingFuture<'_, EventPage> {
        Box::pin(async {
            Ok(EventPage {
                items: Vec::new(),
                next_cursor: None,
            })
        })
    }
}

fn identity() -> AccessIdentity {
    AccessIdentity::new(Uuid::new_v4(), Uuid::new_v4(), "task-4a-test")
}

fn input() -> SendMessageInput {
    SendMessageInput {
        chatroom_id: Uuid::new_v4(),
        client_msg_id: Uuid::new_v4(),
        body: Some("hello".to_owned()),
        media_upload_ids: Vec::new(),
        idempotency_key: None,
    }
}

fn message() -> CanonicalMessage {
    CanonicalMessage {
        id: Uuid::new_v4(),
        chatroom_id: Uuid::new_v4(),
        sender_id: Some(Uuid::new_v4()),
        client_msg_id: Some(Uuid::new_v4()),
        body: Some("hello".to_owned()),
        message_type: MessageKind::User,
        created_at: OffsetDateTime::UNIX_EPOCH,
        media: Vec::new(),
    }
}
