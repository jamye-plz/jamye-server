use std::{
    any::Any,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use jamye_server::{
    application::{
        chatrooms::ChatroomsService,
        messaging::MessagingService,
        topics::{TopicsDependencies, TopicsService},
        transactions::{
            CreateTopicCompositionInput, MarkConversationReadCompositionInput,
            SendMessageCompositionInput, TransactionCompositionDependencies,
            TransactionCompositionError, TransactionCompositions,
        },
    },
    domain::messaging::{CanonicalMessage, EventPage, MessageKind, SendMessageCommand},
    ports::{
        chatrooms::{
            ChatroomPage, ChatroomsRepository, ChatroomsRepositoryError, ChatroomsRepositoryFuture,
            ListChatroomsQuery, MarkReadCommand, MessageHistoryPage, MessageHistoryQuery,
            ReadMarker, ReadMarkerQuery,
        },
        media::{
            AuthorizeMediaAccessQuery, BindMessageMediaCommand, CreateUploadIntentCommand,
            MediaAccessRecord, MediaRepository, MediaRepositoryError, MediaRepositoryFuture,
            PrepareUploadFinalizeQuery, UploadFinalizePreparation, UploadFinalizeRecord,
            UploadIntentRecord,
        },
        messaging::{
            DeltaQuery, MessagingFuture, MessagingRepository, MessagingRepositoryError,
            PersistMessageOutcome, PersistedMessage,
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
        transactions::{
            BoxTransactionHandle, TransactionFuture, TransactionHandle, TransactionManager,
        },
    },
};
use time::OffsetDateTime;
use uuid::Uuid;

#[path = "production_composition/composition.rs"]
mod composition;
#[path = "production_composition/contract_generation.rs"]
mod contract_generation;
#[path = "../src/contract_generation/mod.rs"]
mod contract_snapshot;
#[path = "production_composition/migration_chain.rs"]
mod migration_chain;
#[path = "production_composition/postgres.rs"]
mod postgres;
#[path = "support/postgres.rs"]
mod postgres_support;
#[path = "production_composition/uow.rs"]
mod uow;

include!("production_composition/recording_repositories.rs");
include!("production_composition/recording_fixture.rs");
include!("production_composition/composition_inputs.rs");
