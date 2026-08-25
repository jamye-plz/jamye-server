use std::collections::HashSet;

use jamye_server::domain::messaging::{
    CanonicalMessage, DeltaItem, EventPage, MessageCreatedEvent, MessageCreatedType, MessageKind,
    ReconcileScope, UnsupportedEventMarker,
};
use time::OffsetDateTime;
use uuid::Uuid;

#[test]
fn two_phase_delta_drain_closes_the_join_gap_without_ws_cursor_skips() {
    let conversation_id = Uuid::new_v4();
    let phase_one_known = [known(conversation_id, 1), known(conversation_id, 2)];
    let phase_one_more = [known(conversation_id, 3)];
    let persisted_unknown = unsupported(4, ReconcileScope::ChatHistory);
    let persisted_after_unknown = known(conversation_id, 5);
    let event_a = known(conversation_id, 6);
    let event_b = known(conversation_id, 7);
    let phase_two_unknown = unsupported(8, ReconcileScope::ChatHistory);
    let phase_two_known = known(conversation_id, 9);
    let unknown_ws_id = Uuid::new_v4();
    let mut observer = RecoveryObserver::default();

    assert_eq!(
        observer.apply_page(&page(
            vec![phase_one_known[0].clone(), phase_one_known[1].clone()],
            Some("2"),
        )),
        PageProgress::Advanced
    );
    assert_eq!(
        observer.apply_page(&page(
            vec![phase_one_more[0].clone(), persisted_unknown.clone(),],
            Some("4"),
        )),
        PageProgress::Advanced
    );
    assert_eq!(
        observer.apply_page(&page(vec![persisted_after_unknown.clone()], None)),
        PageProgress::Exhausted
    );
    assert_eq!(
        observer.apply_page(&page(Vec::new(), None)),
        PageProgress::Exhausted
    );
    assert_eq!(observer.last_cursor, Some(5));

    // A commits after phase #1 but before the new subscription and is therefore delta-only.
    // B arrives after the subscribe ack. Applying it may update the view, but must not move the
    // durable delta cursor past the unseen A join-gap event.
    assert_eq!(observer.observe_ws(&event_b), WsProgress::Applied);
    assert_eq!(observer.observe_ws(&event_b), WsProgress::Duplicate);
    assert_eq!(
        observer.observe_ws(&phase_one_more[0]),
        WsProgress::Duplicate
    );
    assert_eq!(
        observer.observe_unknown_ws(unknown_ws_id),
        WsProgress::NeedDelta
    );
    assert_eq!(observer.last_cursor, Some(5));

    assert_eq!(
        observer.apply_page(&page(vec![event_a.clone(), event_b.clone()], Some("7"))),
        PageProgress::Advanced
    );
    assert_eq!(observer.last_cursor, Some(7));
    assert_eq!(
        observer.apply_page(&page(
            vec![phase_two_unknown.clone(), phase_two_known.clone()],
            None,
        )),
        PageProgress::Exhausted
    );
    assert_eq!(
        observer.apply_page(&page(Vec::new(), None)),
        PageProgress::Exhausted
    );
    assert_eq!(observer.last_cursor, Some(9));
    assert!(observer.dirty_scopes.contains(&ReconcileScope::ChatHistory));

    let before_non_progress = observer.snapshot();
    assert_eq!(
        observer.apply_page(&page(vec![phase_two_known.clone()], Some("9"))),
        PageProgress::NonProgress
    );
    assert_eq!(
        observer.apply_page(&page(vec![phase_one_known[0].clone()], Some("1"))),
        PageProgress::NonProgress
    );
    assert_eq!(observer.snapshot(), before_non_progress);

    let expected = [
        phase_one_known[0].event_id(),
        phase_one_known[1].event_id(),
        phase_one_more[0].event_id(),
        persisted_unknown.event_id(),
        persisted_after_unknown.event_id(),
        event_a.event_id(),
        event_b.event_id(),
        phase_two_unknown.event_id(),
        phase_two_known.event_id(),
    ];
    for event_id in expected {
        assert!(observer.seen_event_ids.contains(&event_id));
    }
    assert_eq!(observer.seen_event_ids.len(), 9);
    assert!(!observer.seen_event_ids.contains(&unknown_ws_id));
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PageProgress {
    Advanced,
    Exhausted,
    NonProgress,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WsProgress {
    Applied,
    Duplicate,
    NeedDelta,
}

#[derive(Default)]
struct RecoveryObserver {
    seen_event_ids: HashSet<Uuid>,
    last_cursor: Option<i64>,
    dirty_scopes: Vec<ReconcileScope>,
}

impl RecoveryObserver {
    fn apply_page(&mut self, page: &EventPage) -> PageProgress {
        if page.items.is_empty() {
            return PageProgress::Exhausted;
        }
        let parsed = page
            .items
            .iter()
            .map(|item| item.cursor().parse::<i64>().map(|cursor| (item, cursor)))
            .collect::<Result<Vec<_>, _>>();
        let Ok(parsed) = parsed else {
            return PageProgress::NonProgress;
        };
        let mut previous = self.last_cursor.unwrap_or(0);
        if parsed.iter().any(|(_, cursor)| {
            let progresses = *cursor > previous;
            previous = *cursor;
            !progresses
        }) {
            return PageProgress::NonProgress;
        }

        // The page is validated before mutation. Each item is applied/idempotently reconciled
        // before its cursor becomes the new durable recovery position.
        for (item, cursor) in parsed {
            self.seen_event_ids.insert(item.event_id());
            if let DeltaItem::Unsupported(marker) = item
                && !self.dirty_scopes.contains(&marker.reconcile_scope)
            {
                self.dirty_scopes.push(marker.reconcile_scope);
            }
            self.last_cursor = Some(cursor);
        }
        if page.next_cursor.is_some() {
            PageProgress::Advanced
        } else {
            PageProgress::Exhausted
        }
    }

    fn observe_ws(&mut self, item: &DeltaItem) -> WsProgress {
        if self.seen_event_ids.insert(item.event_id()) {
            WsProgress::Applied
        } else {
            WsProgress::Duplicate
        }
    }

    fn observe_unknown_ws(&self, _event_id: Uuid) -> WsProgress {
        WsProgress::NeedDelta
    }

    fn snapshot(&self) -> (HashSet<Uuid>, Option<i64>, Vec<ReconcileScope>) {
        (
            self.seen_event_ids.clone(),
            self.last_cursor,
            self.dirty_scopes.clone(),
        )
    }
}

fn page(items: Vec<DeltaItem>, next_cursor: Option<&str>) -> EventPage {
    EventPage {
        items,
        next_cursor: next_cursor.map(str::to_owned),
    }
}

fn known(conversation_id: Uuid, cursor: i64) -> DeltaItem {
    DeltaItem::Known(MessageCreatedEvent {
        version: 1,
        event_type: MessageCreatedType::MessageCreated,
        event_id: Uuid::new_v4(),
        conversation_id,
        cursor: cursor.to_string(),
        occurred_at: OffsetDateTime::UNIX_EPOCH,
        data: CanonicalMessage {
            id: Uuid::new_v4(),
            chatroom_id: conversation_id,
            sender_id: Some(Uuid::new_v4()),
            client_msg_id: Some(Uuid::new_v4()),
            body: Some(format!("event-{cursor}")),
            message_type: MessageKind::User,
            created_at: OffsetDateTime::UNIX_EPOCH,
            media: Vec::new(),
        },
    })
}

fn unsupported(cursor: i64, reconcile_scope: ReconcileScope) -> DeltaItem {
    DeltaItem::Unsupported(UnsupportedEventMarker {
        event_id: Uuid::new_v4(),
        cursor: cursor.to_string(),
        reconcile_scope,
    })
}
