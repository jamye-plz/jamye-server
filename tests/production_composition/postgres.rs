use std::{
    io,
    sync::{Arc, Mutex},
};

use jamye_server::{
    adapters::postgres::{
        chatrooms::PostgresChatroomsRepository, media::PostgresMediaRepository,
        messaging::PostgresMessagingRepository, notifications::PostgresNotificationsRepository,
        topics::PostgresTopicsRepository, transactions::SqlxTransactionManager,
    },
    application::{
        chatrooms::ChatroomsService,
        messaging::MessagingService,
        topics::{TopicsDependencies, TopicsService},
        transactions::{
            CreateTopicCompositionInput, MarkConversationReadCompositionInput,
            SendMessageCompositionInput, TransactionCompositionDependencies,
            TransactionCompositions,
        },
    },
    domain::{
        media::{FinalizedObject, MediaKind},
        messaging::SendMessageCommand,
    },
    ports::{
        chatrooms::{
            ChatroomPage, ChatroomsRepository, ChatroomsRepositoryError, ChatroomsRepositoryFuture,
            ListChatroomsQuery, MarkReadCommand, MessageHistoryPage, MessageHistoryQuery,
            ReadMarker, ReadMarkerQuery,
        },
        media::{
            AuthorizeMediaAccessQuery, BindMessageMediaCommand, BindMessageMediaItem,
            CreateUploadIntentCommand, FinalizeUploadCommand, MediaAccessRecord, MediaRepository,
            MediaRepositoryError, MediaRepositoryFuture, PrepareUploadFinalizeQuery,
            UploadFinalizePreparation, UploadFinalizeRecord, UploadIntentRecord,
        },
        messaging::{
            DeltaQuery, MessagingFuture, MessagingRepository, MessagingRepositoryError,
            PersistedMessage,
        },
        push::{
            ClearTopicNotificationsCommand, NotificationClearReport, NotificationEventsRepository,
            NotificationEventsRepositoryFuture, NotificationFanoutReport,
            NotificationsRepositoryError, RecordMessageNotificationCommand,
            RecordTopicNotificationCommand,
        },
        topics::{
            CreateTopicCommand, CreateTopicOutcome, GetTopicQuery, ListTopicDatesQuery,
            ListTopicMediaQuery, ListTopicTagsQuery, ListTopicsQuery, PatchTopicCommand,
            ReplaceTopicTagsCommand, TopicDatePage, TopicMediaPage, TopicPage, TopicRecord,
            TopicStatus, TopicTagPage, TopicsRepository, TopicsRepositoryError,
            TopicsRepositoryFuture,
        },
        transactions::TransactionManager,
    },
};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use crate::{TestResult, postgres_support::TestDatabase};

include!("postgres/postgres_cases.rs");
include!("postgres/postgres_fixture.rs");
include!("postgres/snapshot_support.rs");
