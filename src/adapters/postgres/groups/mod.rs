//! PostgreSQL group topology, membership, role, and bounded-invite adapter.

mod mutation;
mod query;

use sqlx::PgPool;

use crate::{
    adapters::postgres::transactions::connection,
    ports::{
        groups::{
            CreateGroupCommand, CreateInviteCommand, GetGroupQuery, GroupActorCommand, GroupPage,
            GroupRecord, GroupsRepository, GroupsRepositoryError, GroupsRepositoryFuture,
            InviteJoinRecord, InviteRecord, ListGroupsQuery, ListMembersQuery, MemberPage,
            RedeemInviteCommand, RemoveMemberCommand, RenameGroupCommand, SetMemberRoleCommand,
        },
        transactions::TransactionHandle,
    },
};

#[derive(Clone)]
pub struct PostgresGroupsRepository {
    pool: PgPool,
}

impl PostgresGroupsRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl GroupsRepository for PostgresGroupsRepository {
    fn create_group<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        command: &'a CreateGroupCommand,
    ) -> GroupsRepositoryFuture<'a, GroupRecord> {
        Box::pin(async move {
            let connection =
                connection(transaction).map_err(|_| GroupsRepositoryError::InvalidData)?;
            mutation::create_group(connection, command).await
        })
    }

    fn list_groups(&self, query: ListGroupsQuery) -> GroupsRepositoryFuture<'_, GroupPage> {
        Box::pin(query::list_groups(&self.pool, query))
    }

    fn get_group(&self, query: GetGroupQuery) -> GroupsRepositoryFuture<'_, GroupRecord> {
        Box::pin(query::get_group(&self.pool, query))
    }

    fn list_members(&self, query: ListMembersQuery) -> GroupsRepositoryFuture<'_, MemberPage> {
        Box::pin(query::list_members(&self.pool, query))
    }

    fn rename_group<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        command: &'a RenameGroupCommand,
    ) -> GroupsRepositoryFuture<'a, GroupRecord> {
        Box::pin(async move {
            let connection =
                connection(transaction).map_err(|_| GroupsRepositoryError::InvalidData)?;
            mutation::rename_group(connection, command).await
        })
    }

    fn delete_group<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        command: &'a GroupActorCommand,
    ) -> GroupsRepositoryFuture<'a, ()> {
        Box::pin(async move {
            let connection =
                connection(transaction).map_err(|_| GroupsRepositoryError::InvalidData)?;
            mutation::delete_group(connection, command).await
        })
    }

    fn remove_member<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        command: &'a RemoveMemberCommand,
    ) -> GroupsRepositoryFuture<'a, ()> {
        Box::pin(async move {
            let connection =
                connection(transaction).map_err(|_| GroupsRepositoryError::InvalidData)?;
            mutation::remove_member(connection, command).await
        })
    }

    fn set_member_role<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        command: &'a SetMemberRoleCommand,
    ) -> GroupsRepositoryFuture<'a, ()> {
        Box::pin(async move {
            let connection =
                connection(transaction).map_err(|_| GroupsRepositoryError::InvalidData)?;
            mutation::set_member_role(connection, command).await
        })
    }

    fn create_invite<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        command: &'a CreateInviteCommand,
    ) -> GroupsRepositoryFuture<'a, InviteRecord> {
        Box::pin(async move {
            let connection =
                connection(transaction).map_err(|_| GroupsRepositoryError::InvalidData)?;
            mutation::create_invite(connection, command).await
        })
    }

    fn redeem_invite<'a>(
        &'a self,
        transaction: &'a mut dyn TransactionHandle,
        command: &'a RedeemInviteCommand,
    ) -> GroupsRepositoryFuture<'a, InviteJoinRecord> {
        Box::pin(async move {
            let connection =
                connection(transaction).map_err(|_| GroupsRepositoryError::InvalidData)?;
            mutation::redeem_invite(connection, command).await
        })
    }
}

pub(super) fn database_error(operation: &'static str, error: sqlx::Error) -> GroupsRepositoryError {
    if let sqlx::Error::Database(database) = &error {
        match database.constraint() {
            Some("uq_invites_code") => return GroupsRepositoryError::InviteCodeCollision,
            Some("uq_memberships_group_user") => {
                return GroupsRepositoryError::MembershipConflict;
            }
            Some("ux_chatrooms_one_main_per_group") => {
                return GroupsRepositoryError::TopologyConflict;
            }
            Some(
                "invites_code_length_check"
                | "invites_max_uses_check"
                | "invites_used_count_check"
                | "invites_usage_bound_check"
                | "groups_max_members_check"
                | "memberships_role_check"
                | "chatrooms_type_topic_check",
            ) => return GroupsRepositoryError::InvalidData,
            _ => {}
        }
    }
    tracing::warn!(
        dependency = "postgres",
        failure_kind = "groups",
        operation,
        "PostgreSQL group operation failed"
    );
    GroupsRepositoryError::Unavailable
}
