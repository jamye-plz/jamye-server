# ADR 0011: Soft Delete, Contract v2, Account Deletion Grace

## Status

Accepted for task-14 server phase 1. Not yet deployed to production.

## Context

Task-14 introduces user-visible deletion and recovery semantics after the C2
server contract. Immediate account purge made retained message/topic authorship
safe for privacy, but it prevented the product requirement that a user can
restore an account during a 30-day grace period and recover profile and
membership state.

The server also needs to add delete events and recovery signals without
silently changing the existing mobile token body or breaking previous clients
during rollout.

## Decision

Use forward-only soft-delete columns and live-row predicates as the phase-1
server boundary.

- Account deletion starts a grace period by setting `users.deleted_at`, revoking
  and soft-deleting refresh sessions, soft-deleting auth identities, disabling
  push installations, and soft-deleting memberships with
  `memberships.account_deleted_at`.
- Retained authored content keeps `sender_id` and `author_id` during grace.
  Read paths anonymize deleted users as `탈퇴한 사용자` with a null avatar.
- D10 hard purge moves to a leased worker that uses DB clock, batch claim, and
  the existing account deletion finalization transition after the grace period.
- A2 OAuth exchange restores a grace-deleted account only through an explicit
  restore path for a soft-deleted identity. The response body remains
  `TokenPair`; the one-time recovery signal is the
  `X-Jamye-Account-Restored: true` response header.
- Contract v2 is the current negotiation target for delete/recovery behavior;
  previous contract projection remains available where the base phase exposes
  it.
- Migration 0016 rewrites only the phase-1 uniqueness constraints:
  `uq_memberships_group_user` and
  `uq_auth_identities_provider_principal` become partial live-row unique
  indexes.

## Alternatives Considered

Immediate D10 purge on U3 was rejected because it cannot satisfy account
restoration within the grace period.

Adding a `restored` field to `TokenPair` was rejected because it would change
the stable auth body and force unrelated client parsing changes.

Restoring every historical membership was rejected because group deletion,
ordinary leave/kick, and intervening rejoin are different product events. Only
memberships marked by account deletion are eligible.

## Consequences

Read paths must consistently mask deleted users rather than relying on stored
payload mutation. Purge remains responsible for the irreversible D10 transition
and object cleanup intents. Operational rollout needs pre/post read-only count
SQL across migrations 0014 through 0016 before considering production deploy.
