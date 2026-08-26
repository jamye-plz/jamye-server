//! Group topology, membership, role, and bounded-invite persistence boundary.

use std::{fmt, future::Future, pin::Pin};

use time::OffsetDateTime;
use uuid::Uuid;

use crate::ports::transactions::TransactionHandle;

pub type GroupsRepositoryFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, GroupsRepositoryError>> + Send + 'a>>;

pub trait GroupsRepository: Send + Sync {
    fn create_group<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        command: &'a CreateGroupCommand,
    ) -> GroupsRepositoryFuture<'a, GroupRecord>;

    fn list_groups(&self, query: ListGroupsQuery) -> GroupsRepositoryFuture<'_, GroupPage>;

    fn get_group(&self, query: GetGroupQuery) -> GroupsRepositoryFuture<'_, GroupRecord>;

    fn list_members(&self, query: ListMembersQuery) -> GroupsRepositoryFuture<'_, MemberPage>;

    fn rename_group<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        command: &'a RenameGroupCommand,
    ) -> GroupsRepositoryFuture<'a, GroupRecord>;

    fn delete_group<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        command: &'a GroupActorCommand,
    ) -> GroupsRepositoryFuture<'a, ()>;

    fn remove_member<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        command: &'a RemoveMemberCommand,
    ) -> GroupsRepositoryFuture<'a, ()>;

    fn set_member_role<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        command: &'a SetMemberRoleCommand,
    ) -> GroupsRepositoryFuture<'a, ()>;

    fn create_invite<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        command: &'a CreateInviteCommand,
    ) -> GroupsRepositoryFuture<'a, InviteRecord>;

    fn redeem_invite<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        command: &'a RedeemInviteCommand,
    ) -> GroupsRepositoryFuture<'a, InviteJoinRecord>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreateGroupCommand {
    pub group_id: Uuid,
    pub membership_id: Uuid,
    pub main_chatroom_id: Uuid,
    pub owner_id: Uuid,
    pub name: String,
    pub max_members: i32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ListGroupsQuery {
    pub user_id: Uuid,
    pub after: Option<Uuid>,
    pub limit: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GetGroupQuery {
    pub group_id: Uuid,
    pub actor_id: Uuid,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ListMembersQuery {
    pub group_id: Uuid,
    pub actor_id: Uuid,
    pub after: Option<Uuid>,
    pub limit: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenameGroupCommand {
    pub group_id: Uuid,
    pub actor_id: Uuid,
    pub name: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GroupActorCommand {
    pub group_id: Uuid,
    pub actor_id: Uuid,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RemoveMemberCommand {
    pub group_id: Uuid,
    pub actor_id: Uuid,
    pub target_user_id: Uuid,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SetMemberRoleCommand {
    pub group_id: Uuid,
    pub actor_id: Uuid,
    pub target_user_id: Uuid,
    pub role: GroupRole,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreateInviteCommand {
    pub invite_id: Uuid,
    pub group_id: Uuid,
    pub actor_id: Uuid,
    pub code: String,
    pub expires_at: Option<OffsetDateTime>,
    pub max_uses: Option<i32>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RedeemInviteCommand {
    pub membership_id: Uuid,
    pub actor_id: Uuid,
    pub code: String,
    pub now: OffsetDateTime,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GroupRole {
    Owner,
    Member,
}

impl GroupRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Member => "member",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "owner" => Some(Self::Owner),
            "member" => Some(Self::Member),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GroupRecord {
    pub id: Uuid,
    pub name: String,
    pub owner_id: Uuid,
    pub max_members: i32,
    pub member_count: i64,
    pub created_at: OffsetDateTime,
    pub main_chatroom_id: Uuid,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GroupPage {
    pub items: Vec<GroupRecord>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemberRecord {
    pub membership_id: Uuid,
    pub user_id: Uuid,
    pub nickname: String,
    pub avatar_url: Option<String>,
    pub role: GroupRole,
    pub joined_at: OffsetDateTime,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemberPage {
    pub items: Vec<MemberRecord>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InviteRecord {
    pub id: Uuid,
    pub group_id: Uuid,
    pub code: String,
    pub created_by: Uuid,
    pub expires_at: Option<OffsetDateTime>,
    pub max_uses: Option<i32>,
    pub used_count: i32,
    pub created_at: OffsetDateTime,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InviteJoinRecord {
    pub group_id: Uuid,
    pub membership_id: Option<Uuid>,
    pub joined: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GroupsRepositoryError {
    GroupNotFound,
    MembershipRequired,
    OwnerRequired,
    MemberNotFound,
    OwnerConflict,
    GroupFull,
    InviteNotFound,
    InviteExpired,
    InviteExhausted,
    InviteCodeCollision,
    MembershipConflict,
    TopologyConflict,
    InvalidData,
    Unavailable,
}

impl fmt::Display for GroupsRepositoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("group persistence operation failed")
    }
}

impl std::error::Error for GroupsRepositoryError {}

pub trait GroupsClock: Send + Sync {
    fn now(&self) -> OffsetDateTime;
}
