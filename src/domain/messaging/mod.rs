//! Reliable messaging domain values shared by commands and recovery.

mod model;

pub use model::{
    CanonicalMessage, ConversationEvent, DeltaItem, EventPage, MessageAttachment,
    MessageCreatedEvent, MessageCreatedType, MessageDeletedData, MessageDeletedEvent,
    MessageDeletedType, MessageKind, RealtimeServerEvent, ReconcileScope, SendMessageCommand,
    TopicCreatedData, TopicCreatedEvent, TopicCreatedType, TopicDeletedData, TopicDeletedEvent,
    TopicDeletedType, UnsupportedEventMarker,
};
