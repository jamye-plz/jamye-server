use std::{
    any::Any,
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use jamye_server::{
    application::notifications::{
        NotificationPageInput, NotificationsDependencies, NotificationsError, NotificationsService,
    },
    ports::{
        push::{
            ListNotificationsQuery, MarkNotificationReadCommand, NotificationPage,
            NotificationReadRecord, NotificationRecord, NotificationType, NotificationsRepository,
            NotificationsRepositoryError, NotificationsRepositoryFuture,
        },
        transactions::{
            BoxTransactionHandle, TransactionFuture, TransactionHandle, TransactionManager,
        },
    },
};
use serde_json::json;
use time::OffsetDateTime;
use uuid::Uuid;

#[tokio::test]
async fn n1_rejects_malformed_cursor_and_out_of_range_limit_before_repository_access() {
    for input in [
        NotificationPageInput {
            after: Some("not-an-opaque-cursor".to_owned()),
            limit: Some(10),
        },
        NotificationPageInput {
            after: None,
            limit: Some(0),
        },
        NotificationPageInput {
            after: None,
            limit: Some(101),
        },
    ] {
        let harness = Harness::success();
        assert_eq!(
            harness.service.list_notifications(user_id(), input).await,
            Err(NotificationsError::RequestValidation)
        );
        assert_eq!(harness.calls(), Vec::<Call>::new());
    }
}

#[tokio::test]
async fn n1_scopes_the_opaque_page_query_and_returns_the_global_unread_count() {
    let expected = notification_page();
    let harness = Harness::new(Ok(expected.clone()), Ok(read_record()));

    assert_eq!(
        harness
            .service
            .list_notifications(
                user_id(),
                NotificationPageInput {
                    after: Some(after_id().to_string()),
                    limit: Some(2),
                },
            )
            .await,
        Ok(expected)
    );
    assert_eq!(harness.calls(), vec![Call::List]);
    assert_eq!(
        harness.repository.list_queries(),
        vec![ListNotificationsQuery {
            user_id: user_id(),
            after: Some(after_id()),
            limit: 2,
        }]
    );
}

#[tokio::test]
async fn n2_first_and_repeated_reads_each_commit_one_owner_scoped_command() {
    let harness = Harness::success();

    assert_eq!(
        harness
            .service
            .mark_read(user_id(), notification_id())
            .await,
        Ok(())
    );
    assert_eq!(
        harness
            .service
            .mark_read(user_id(), notification_id())
            .await,
        Ok(())
    );
    assert_eq!(
        harness.calls(),
        vec![
            Call::Begin,
            Call::MarkRead,
            Call::Commit,
            Call::Begin,
            Call::MarkRead,
            Call::Commit,
        ]
    );
    assert_eq!(
        harness.repository.mark_commands(),
        vec![
            MarkNotificationReadCommand {
                user_id: user_id(),
                notification_id: notification_id(),
            },
            MarkNotificationReadCommand {
                user_id: user_id(),
                notification_id: notification_id(),
            },
        ]
    );
}

#[tokio::test]
async fn n2_missing_and_foreign_ids_share_one_not_found_result_and_rollback() {
    for notification_id in [missing_id(), foreign_id()] {
        let harness = Harness::new(
            Ok(notification_page()),
            Err(NotificationsRepositoryError::NotificationNotFound),
        );
        assert_eq!(
            harness.service.mark_read(user_id(), notification_id).await,
            Err(NotificationsError::NotificationNotFound)
        );
        assert_eq!(
            harness.calls(),
            vec![Call::Begin, Call::MarkRead, Call::Rollback]
        );
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Call {
    List,
    Begin,
    MarkRead,
    Commit,
    Rollback,
}

struct Harness {
    service: NotificationsService,
    calls: Arc<Mutex<Vec<Call>>>,
    repository: Arc<RecordingRepository>,
}

impl Harness {
    fn success() -> Self {
        Self::new(Ok(notification_page()), Ok(read_record()))
    }

    fn new(
        list_result: Result<NotificationPage, NotificationsRepositoryError>,
        mark_result: Result<NotificationReadRecord, NotificationsRepositoryError>,
    ) -> Self {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let transactions = Arc::new(RecordingTransactions {
            calls: calls.clone(),
        });
        let repository = Arc::new(RecordingRepository {
            calls: calls.clone(),
            list_queries: Mutex::new(Vec::new()),
            mark_commands: Mutex::new(Vec::new()),
            list_result,
            mark_result,
        });
        let service = NotificationsService::new(NotificationsDependencies {
            transactions,
            repository: repository.clone(),
        });
        Self {
            service,
            calls,
            repository,
        }
    }

    fn calls(&self) -> Vec<Call> {
        crate::lock_test_mutex(&self.calls, "call").clone()
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

struct RecordingTransactions {
    calls: Arc<Mutex<Vec<Call>>>,
}

impl TransactionManager for RecordingTransactions {
    fn begin(&self) -> TransactionFuture<'_, BoxTransactionHandle> {
        record(&self.calls, Call::Begin);
        Box::pin(async { Ok(Box::new(RecordingHandle) as BoxTransactionHandle) })
    }

    fn commit<'a>(&'a self, handle: BoxTransactionHandle) -> TransactionFuture<'a, ()> {
        assert!(handle.into_any().downcast::<RecordingHandle>().is_ok());
        record(&self.calls, Call::Commit);
        Box::pin(async { Ok(()) })
    }

    fn rollback<'a>(&'a self, handle: BoxTransactionHandle) -> TransactionFuture<'a, ()> {
        assert!(handle.into_any().downcast::<RecordingHandle>().is_ok());
        record(&self.calls, Call::Rollback);
        Box::pin(async { Ok(()) })
    }
}

struct RecordingRepository {
    calls: Arc<Mutex<Vec<Call>>>,
    list_queries: Mutex<Vec<ListNotificationsQuery>>,
    mark_commands: Mutex<Vec<MarkNotificationReadCommand>>,
    list_result: Result<NotificationPage, NotificationsRepositoryError>,
    mark_result: Result<NotificationReadRecord, NotificationsRepositoryError>,
}

impl RecordingRepository {
    fn list_queries(&self) -> Vec<ListNotificationsQuery> {
        crate::lock_test_mutex(&self.list_queries, "list query").clone()
    }

    fn mark_commands(&self) -> Vec<MarkNotificationReadCommand> {
        crate::lock_test_mutex(&self.mark_commands, "mark command").clone()
    }
}

impl NotificationsRepository for RecordingRepository {
    fn list_notifications(
        &self,
        query: ListNotificationsQuery,
    ) -> NotificationsRepositoryFuture<'_, NotificationPage> {
        record(&self.calls, Call::List);
        crate::lock_test_mutex(&self.list_queries, "list query").push(query);
        let result = self.list_result.clone();
        Box::pin(async move { result })
    }

    fn mark_notification_read<'a>(
        &'a self,
        _transaction: &'a mut dyn TransactionHandle,
        command: &'a MarkNotificationReadCommand,
    ) -> NotificationsRepositoryFuture<'a, NotificationReadRecord> {
        record(&self.calls, Call::MarkRead);
        crate::lock_test_mutex(&self.mark_commands, "mark command").push(*command);
        let result = self.mark_result;
        Box::pin(async move { result })
    }
}

fn record(calls: &Mutex<Vec<Call>>, call: Call) {
    crate::lock_test_mutex(calls, "call").push(call);
}

fn notification_page() -> NotificationPage {
    NotificationPage {
        items: vec![NotificationRecord {
            id: notification_id(),
            notification_type: NotificationType::ChatUnread,
            args: BTreeMap::from([("sender_name".to_owned(), json!("친구"))]),
            topic_id: Some(topic_id()),
            conversation_id: Some(conversation_id()),
            source_cursor: Some(41),
            read_at: None,
            created_at: OffsetDateTime::UNIX_EPOCH,
        }],
        next_cursor: Some(notification_id().to_string()),
        unread_count: 17,
    }
}

fn read_record() -> NotificationReadRecord {
    NotificationReadRecord {
        notification_id: notification_id(),
        read_at: OffsetDateTime::UNIX_EPOCH,
    }
}

fn user_id() -> Uuid {
    Uuid::from_u128(0xaaaaaaaa_aaaa_4aaa_8aaa_aaaaaaaaaaaa)
}

fn notification_id() -> Uuid {
    Uuid::from_u128(0x11111111_1111_4111_8111_111111111111)
}

fn after_id() -> Uuid {
    Uuid::from_u128(0x22222222_2222_4222_8222_222222222222)
}

fn missing_id() -> Uuid {
    Uuid::from_u128(0x33333333_3333_4333_8333_333333333333)
}

fn foreign_id() -> Uuid {
    Uuid::from_u128(0x44444444_4444_4444_8444_444444444444)
}

fn topic_id() -> Uuid {
    Uuid::from_u128(0x55555555_5555_4555_8555_555555555555)
}

fn conversation_id() -> Uuid {
    Uuid::from_u128(0x66666666_6666_4666_8666_666666666666)
}
