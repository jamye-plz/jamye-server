//! Local terminal control delivered only after registry state has been removed.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RealtimeEvictionReason {
    MembershipRevoked,
    GroupDeleted,
}

impl RealtimeEvictionReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MembershipRevoked => "membership_revoked",
            Self::GroupDeleted => "group_deleted",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RealtimeEviction {
    pub reason: RealtimeEvictionReason,
}

impl RealtimeEviction {
    pub(super) fn new(reason: RealtimeEvictionReason) -> Self {
        Self { reason }
    }
}
