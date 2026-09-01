//! Explicit C2 selected-surface inventory.
//!
//! This is deliberately data, not a runtime registry or a source scan.  It is
//! the one reviewed input used to render the public contract snapshot and its
//! provenance mapping.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RestSurface {
    pub(crate) operation_id: &'static str,
    pub(crate) method: &'static str,
    pub(crate) path: &'static str,
    pub(crate) handler: &'static str,
    pub(crate) feature_behavior_test: &'static str,
    pub(crate) fixture: &'static str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RealtimeSurface {
    pub(crate) event_type: &'static str,
    pub(crate) version: u8,
    pub(crate) handler: &'static str,
    pub(crate) feature_behavior_test: &'static str,
    pub(crate) fixture: &'static str,
    pub(crate) schema: &'static str,
}

macro_rules! row {
    ($id:literal, $method:literal, $path:literal, $handler:literal, $test:literal, $fixture:literal) => {
        RestSurface {
            operation_id: $id,
            method: $method,
            path: $path,
            handler: $handler,
            feature_behavior_test: $test,
            fixture: $fixture,
        }
    };
}

pub(crate) const ROUTE_PROBE: &str = "tests/production_composition/composition.rs::api_root_matches_the_complete_frozen_selected_method_path_inventory";

pub(crate) const REST_SURFACES: &[RestSurface] = &[
    row!(
        "H1",
        "get",
        "/health/live",
        "src/transport/http/health/mod.rs::live",
        "tests/platform.rs::liveness_is_unconditional_and_has_server_request_id",
        "contracts/fixtures/c2-health-profile-account.json"
    ),
    row!(
        "H2",
        "get",
        "/health/ready",
        "src/transport/http/health/mod.rs::ready",
        "tests/platform.rs::readiness_requires_postgres_but_only_degrades_optional_services",
        "contracts/fixtures/c2-health-profile-account.json"
    ),
    row!(
        "A1",
        "post",
        "/api/v1/auth/oauth/{provider}/authorize",
        "src/transport/http/auth/api.rs::authorize",
        "tests/auth/oauth.rs::authorize_http_separates_shape_semantics_and_stable_rate_limit",
        "contracts/contributions/task-5/fixtures/mobile-auth-handoff.json"
    ),
    row!(
        "A2",
        "post",
        "/api/v1/auth/oauth/{provider}/exchange",
        "src/transport/http/auth/api.rs::exchange",
        "tests/auth/oauth.rs::auth_http_happy_path_returns_exact_mobile_token_pairs_and_stores_only_digest",
        "contracts/contributions/task-5/fixtures/mobile-auth-handoff.json"
    ),
    row!(
        "A3",
        "post",
        "/api/v1/auth/refresh",
        "src/transport/http/auth/api.rs::refresh",
        "tests/auth/session.rs::concurrent_refresh_allows_one_child_then_reuse_revokes_the_family",
        "contracts/contributions/task-5/fixtures/mobile-auth-handoff.json"
    ),
    row!(
        "A4",
        "post",
        "/api/v1/auth/logout",
        "src/transport/http/auth/api.rs::logout",
        "tests/auth/session.rs::logout_revokes_only_refresh_authority_and_access_remains_valid_to_expiry",
        "contracts/contributions/task-5/fixtures/mobile-auth-handoff.json"
    ),
    row!(
        "U1",
        "get",
        "/api/v1/me",
        "src/transport/http/users/mod.rs::get_me",
        "tests/profile.rs::get_and_patch_me_use_the_shared_production_bearer_extractor",
        "contracts/fixtures/c2-health-profile-account.json"
    ),
    row!(
        "U2",
        "patch",
        "/api/v1/me",
        "src/transport/http/users/mod.rs::patch_me",
        "tests/profile.rs::get_and_patch_me_use_the_shared_production_bearer_extractor",
        "contracts/fixtures/c2-health-profile-account.json"
    ),
    row!(
        "U3",
        "delete",
        "/api/v1/me",
        "src/transport/http/account_deletion/mod.rs::delete_account",
        "tests/account_deletion/http.rs::delete_me_commits_one_empty_204_then_revokes_account_access_and_anonymizes_retained_content",
        "contracts/fixtures/c2-health-profile-account.json"
    ),
    row!(
        "G1",
        "post",
        "/api/v1/groups",
        "src/transport/http/groups/mod.rs::create_group",
        "tests/groups/http.rs::group_http_crud_paginates_and_enforces_membership_without_disclosure",
        "contracts/contributions/task-6/fixtures/group-invite-flow.json"
    ),
    row!(
        "G2",
        "get",
        "/api/v1/groups",
        "src/transport/http/groups/mod.rs::list_groups",
        "tests/groups/http.rs::group_http_crud_paginates_and_enforces_membership_without_disclosure",
        "contracts/contributions/task-6/fixtures/group-invite-flow.json"
    ),
    row!(
        "G3",
        "get",
        "/api/v1/groups/{group_id}",
        "src/transport/http/groups/mod.rs::get_group",
        "tests/groups/http.rs::group_http_crud_paginates_and_enforces_membership_without_disclosure",
        "contracts/contributions/task-6/fixtures/group-invite-flow.json"
    ),
    row!(
        "G4",
        "get",
        "/api/v1/groups/{group_id}/members",
        "src/transport/http/groups/mod.rs::list_members",
        "tests/groups/http.rs::group_http_crud_paginates_and_enforces_membership_without_disclosure",
        "contracts/contributions/task-6/fixtures/group-invite-flow.json"
    ),
    row!(
        "G5",
        "patch",
        "/api/v1/groups/{group_id}",
        "src/transport/http/groups/mod.rs::rename_group",
        "tests/groups/http.rs::group_http_crud_paginates_and_enforces_membership_without_disclosure",
        "contracts/contributions/task-6/fixtures/group-invite-flow.json"
    ),
    row!(
        "G6",
        "delete",
        "/api/v1/groups/{group_id}",
        "src/transport/http/groups/mod.rs::delete_group",
        "tests/groups/http.rs::role_invite_join_removal_and_delete_routes_preserve_authority",
        "contracts/contributions/task-6/fixtures/group-invite-flow.json"
    ),
    row!(
        "G7",
        "delete",
        "/api/v1/groups/{group_id}/members/{user_id}",
        "src/transport/http/groups/mod.rs::remove_member",
        "tests/groups/http.rs::role_invite_join_removal_and_delete_routes_preserve_authority",
        "contracts/contributions/task-6/fixtures/group-invite-flow.json"
    ),
    row!(
        "G8",
        "patch",
        "/api/v1/groups/{group_id}/members/{user_id}",
        "src/transport/http/groups/mod.rs::set_member_role",
        "tests/groups/http.rs::role_invite_join_removal_and_delete_routes_preserve_authority",
        "contracts/contributions/task-6/fixtures/group-invite-flow.json"
    ),
    row!(
        "I1",
        "post",
        "/api/v1/groups/{group_id}/invites",
        "src/transport/http/groups/mod.rs::create_invite",
        "tests/groups/http.rs::role_invite_join_removal_and_delete_routes_preserve_authority",
        "contracts/contributions/task-6/fixtures/group-invite-flow.json"
    ),
    row!(
        "I2",
        "post",
        "/api/v1/invites/{code}/join",
        "src/transport/http/groups/mod.rs::redeem_invite",
        "tests/groups/http.rs::role_invite_join_removal_and_delete_routes_preserve_authority",
        "contracts/contributions/task-6/fixtures/group-invite-flow.json"
    ),
    row!(
        "T1",
        "post",
        "/api/v1/groups/{group_id}/topics",
        "src/transport/http/topics/mod.rs::create_topic",
        "tests/topics/http.rs::t1_through_t7_http_use_the_locked_authenticated_mobile_shapes",
        "contracts/contributions/task-7/fixtures/topic-flow.json"
    ),
    row!(
        "T2",
        "get",
        "/api/v1/groups/{group_id}/topics/dates",
        "src/transport/http/topics/mod.rs::list_topic_dates",
        "tests/topics/http.rs::t1_through_t7_http_use_the_locked_authenticated_mobile_shapes",
        "contracts/contributions/task-7/fixtures/topic-flow.json"
    ),
    row!(
        "T3",
        "get",
        "/api/v1/groups/{group_id}/topics",
        "src/transport/http/topics/mod.rs::list_topics",
        "tests/topics/http.rs::t1_through_t7_http_use_the_locked_authenticated_mobile_shapes",
        "contracts/contributions/task-7/fixtures/topic-flow.json"
    ),
    row!(
        "T4",
        "get",
        "/api/v1/groups/{group_id}/topics/{topic_id}",
        "src/transport/http/topics/mod.rs::get_topic",
        "tests/topics/http.rs::t1_through_t7_http_use_the_locked_authenticated_mobile_shapes",
        "contracts/contributions/task-7/fixtures/topic-flow.json"
    ),
    row!(
        "T5",
        "patch",
        "/api/v1/groups/{group_id}/topics/{topic_id}",
        "src/transport/http/topics/mod.rs::patch_topic",
        "tests/topics/http.rs::t1_through_t7_http_use_the_locked_authenticated_mobile_shapes",
        "contracts/contributions/task-7/fixtures/topic-flow.json"
    ),
    row!(
        "T6",
        "put",
        "/api/v1/groups/{group_id}/topics/{topic_id}/tags",
        "src/transport/http/topics/mod.rs::replace_tags",
        "tests/topics/http.rs::t1_through_t7_http_use_the_locked_authenticated_mobile_shapes",
        "contracts/contributions/task-7/fixtures/topic-flow.json"
    ),
    row!(
        "T7",
        "get",
        "/api/v1/groups/{group_id}/topics/{topic_id}/tags",
        "src/transport/http/topics/mod.rs::list_tags",
        "tests/topics/http.rs::t1_through_t7_http_use_the_locked_authenticated_mobile_shapes",
        "contracts/contributions/task-7/fixtures/topic-flow.json"
    ),
    row!(
        "MD1",
        "post",
        "/api/v1/media/uploads",
        "src/transport/http/media/mod.rs::create_upload_intent",
        "tests/media/upload_finalize_http.rs::md1_returns_a_server_minted_intent_and_constrained_put",
        "contracts/contributions/task-8/fixtures/media-flow.json"
    ),
    row!(
        "MD2",
        "post",
        "/api/v1/media/uploads/{upload_id}/finalize",
        "src/transport/http/media/mod.rs::finalize_upload",
        "tests/media/upload_finalize_http.rs::md2_chat_finalize_returns_the_confirmed_unbound_capability",
        "contracts/contributions/task-8/fixtures/media-flow.json"
    ),
    row!(
        "MD3",
        "get",
        "/api/v1/topics/{topic_id}/media",
        "src/transport/http/topics/mod.rs::list_media",
        "tests/media/md3_http.rs::md3_returns_stable_paginated_canonical_topic_media",
        "contracts/contributions/task-8/fixtures/media-flow.json"
    ),
    row!(
        "C1",
        "get",
        "/api/v1/groups/{group_id}/chatrooms",
        "src/transport/http/chatrooms/mod.rs::list_chatrooms",
        "tests/chatrooms/http.rs::c1_c2_and_c3_http_use_exact_authenticated_mobile_shapes",
        "contracts/contributions/task-6b/fixtures/chatroom-history-read.json"
    ),
    row!(
        "C2",
        "get",
        "/api/v1/chatrooms/{chatroom_id}/messages",
        "src/transport/http/chatrooms/mod.rs::message_history",
        "tests/chatrooms/http.rs::c1_c2_and_c3_http_use_exact_authenticated_mobile_shapes",
        "contracts/contributions/task-6b/fixtures/chatroom-history-read.json"
    ),
    row!(
        "C3",
        "post",
        "/api/v1/chatrooms/{chatroom_id}/read",
        "src/transport/http/chatrooms/mod.rs::mark_read",
        "tests/chatrooms/http.rs::c1_c2_and_c3_http_use_exact_authenticated_mobile_shapes",
        "contracts/contributions/task-6b/fixtures/chatroom-history-read.json"
    ),
    row!(
        "C4",
        "post",
        "/api/v1/chatrooms/{chatroom_id}/messages",
        "src/transport/http/messaging/mod.rs::create_message",
        "tests/messaging/http.rs::c4_preserves_content_idempotency_and_exact_text",
        "contracts/fixtures/c4-normal.json"
    ),
    row!(
        "MD4",
        "get",
        "/api/v1/media/{media_id}/url",
        "src/transport/http/media/mod.rs::view_url",
        "tests/media/access_http.rs::md4_returns_only_public_metadata_and_the_short_reissued_url",
        "contracts/contributions/task-8/fixtures/media-flow.json"
    ),
    row!(
        "MD5",
        "get",
        "/api/v1/media/{media_id}/download",
        "src/transport/http/media/mod.rs::download",
        "tests/media/access_http.rs::md5_returns_an_empty_307_redirect_to_the_authorized_download_url",
        "contracts/contributions/task-8/fixtures/media-flow.json"
    ),
    row!(
        "S1",
        "get",
        "/api/v1/conversations/{conversation_id}/events",
        "src/transport/http/messaging/mod.rs::events",
        "tests/messaging/delta.rs::s1_pages_strictly_forward_across_commits_and_an_unknown_marker",
        "contracts/fixtures/mobile-sync-handoff.json"
    ),
    row!(
        "R1",
        "post",
        "/api/v1/realtime/tickets",
        "src/transport/http/realtime/mod.rs::issue_ticket",
        "tests/realtime/ticket.rs::ticket_is_capped_by_access_expiry_and_consumed_exactly_once",
        "contracts/fixtures/realtime-lifecycle.json"
    ),
    row!(
        "P2",
        "post",
        "/api/v1/push/installations",
        "src/transport/http/push/mod.rs::upsert_installation",
        "tests/notifications/http.rs::p2_uses_create_then_canonical_upsert_status_and_never_returns_private_fields",
        "contracts/contributions/task-9/fixtures/notifications-push-flow.json"
    ),
    row!(
        "P3",
        "put",
        "/api/v1/push/installations/{installation_id}",
        "src/transport/http/push/mod.rs::update_installation",
        "tests/notifications/http.rs::p3_and_p4_are_current_owner_scoped_and_keep_the_public_response_shape",
        "contracts/contributions/task-9/fixtures/notifications-push-flow.json"
    ),
    row!(
        "P4",
        "delete",
        "/api/v1/push/installations/{installation_id}",
        "src/transport/http/push/mod.rs::delete_installation",
        "tests/notifications/http.rs::p3_and_p4_are_current_owner_scoped_and_keep_the_public_response_shape",
        "contracts/contributions/task-9/fixtures/notifications-push-flow.json"
    ),
    row!(
        "N1",
        "get",
        "/api/v1/notifications",
        "src/transport/http/notifications/mod.rs::list_notifications",
        "tests/notifications/http.rs::n1_returns_the_owner_scoped_structured_page_without_private_state",
        "contracts/contributions/task-9/fixtures/notifications-push-flow.json"
    ),
    row!(
        "N2",
        "post",
        "/api/v1/notifications/{notification_id}/read",
        "src/transport/http/notifications/mod.rs::mark_notification_read",
        "tests/notifications/http.rs::n2_is_idempotent_and_missing_or_foreign_ids_share_one_safe_not_found",
        "contracts/contributions/task-9/fixtures/notifications-push-flow.json"
    ),
];

pub(crate) const REALTIME_SURFACES: &[RealtimeSurface] = &[
    RealtimeSurface {
        event_type: "message.created",
        version: 1,
        handler: "src/adapters/postgres/messaging/send.rs::persist_event_and_outbox",
        feature_behavior_test: "tests/realtime/c1.rs::dev_c1_flows_from_seed_to_rest_outbox_redis_websocket_and_delta",
        fixture: "contracts/fixtures/message.created.json",
        schema: "contracts/realtime/message.created.schema.json",
    },
    RealtimeSurface {
        event_type: "topic.created",
        version: 1,
        handler: "src/adapters/postgres/topics/mutation.rs::create_topic",
        feature_behavior_test: "tests/topics/create.rs::t1_is_atomic_idempotent_and_emits_distinct_bootstrap_and_announcement_events",
        fixture: "contracts/contributions/task-7/fixtures/topic-flow.json",
        schema: "contracts/realtime/topic.created.schema.json",
    },
];
