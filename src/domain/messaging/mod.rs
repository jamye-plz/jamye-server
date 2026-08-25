//! Reliable messaging domain values shared by commands and recovery.

mod model;

pub use model::{
    CanonicalMessage, ConversationEvent, DeltaItem, EventPage, MessageAttachment,
    MessageCreatedEvent, MessageCreatedType, MessageKind, ReconcileScope, SendMessageCommand,
    UnsupportedEventMarker,
};
