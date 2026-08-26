use sqlx::PgConnection;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::ports::groups::{
    CreateGroupCommand, CreateInviteCommand, GroupActorCommand, GroupRecord, GroupRole,
    GroupsRepositoryError, InviteJoinRecord, InviteRecord, RedeemInviteCommand,
    RemoveMemberCommand, RenameGroupCommand, SetMemberRoleCommand,
};

use super::{database_error, query::group_from_row};

type GroupRow = (Uuid, String, Uuid, i32, i64, OffsetDateTime, Uuid);
type MembershipRow = (Uuid, GroupRole);
type InviteRow = (
    Uuid,
    Uuid,
    String,
    Uuid,
    Option<OffsetDateTime>,
    Option<i32>,
    i32,
    OffsetDateTime,
);

pub(super) async fn create_group(
    connection: &mut PgConnection,
    command: &CreateGroupCommand,
) -> Result<GroupRecord, GroupsRepositoryError> {
    let created_at = sqlx::query_scalar::<_, OffsetDateTime>(
        "INSERT INTO groups (id, name, owner_id, max_members) \
         VALUES ($1, $2, $3, $4) RETURNING created_at",
    )
    .bind(command.group_id)
    .bind(&command.name)
    .bind(command.owner_id)
    .bind(command.max_members)
    .fetch_one(&mut *connection)
    .await
    .map_err(|error| database_error("group_insert", error))?;
    sqlx::query(
        "INSERT INTO memberships (id, group_id, user_id, role) \
         VALUES ($1, $2, $3, 'owner')",
    )
    .bind(command.membership_id)
    .bind(command.group_id)
    .bind(command.owner_id)
    .execute(&mut *connection)
    .await
    .map_err(|error| database_error("owner_membership_insert", error))?;
    sqlx::query(
        "INSERT INTO chatrooms (id, group_id, type, topic_id) \
         VALUES ($1, $2, 'main', NULL)",
    )
    .bind(command.main_chatroom_id)
    .bind(command.group_id)
    .execute(&mut *connection)
    .await
    .map_err(|error| database_error("main_chatroom_insert", error))?;
    Ok(GroupRecord {
        id: command.group_id,
        name: command.name.clone(),
        owner_id: command.owner_id,
        max_members: command.max_members,
        member_count: 1,
        created_at,
        main_chatroom_id: command.main_chatroom_id,
    })
}

pub(super) async fn rename_group(
    connection: &mut PgConnection,
    command: &RenameGroupCommand,
) -> Result<GroupRecord, GroupsRepositoryError> {
    let mut group = lock_live_group(connection, command.group_id).await?;
    require_owner(connection, &group, command.actor_id).await?;
    sqlx::query("UPDATE groups SET name = $2 WHERE id = $1")
        .bind(command.group_id)
        .bind(&command.name)
        .execute(&mut *connection)
        .await
        .map_err(|error| database_error("group_rename", error))?;
    group.name = command.name.clone();
    Ok(group)
}

pub(super) async fn delete_group(
    connection: &mut PgConnection,
    command: &GroupActorCommand,
) -> Result<(), GroupsRepositoryError> {
    let group = lock_live_group(connection, command.group_id).await?;
    require_owner(connection, &group, command.actor_id).await?;
    sqlx::query("UPDATE groups SET deleted_at = clock_timestamp() WHERE id = $1")
        .bind(command.group_id)
        .execute(connection)
        .await
        .map(|_| ())
        .map_err(|error| database_error("group_delete", error))
}

pub(super) async fn remove_member(
    connection: &mut PgConnection,
    command: &RemoveMemberCommand,
) -> Result<(), GroupsRepositoryError> {
    let group = lock_live_group(connection, command.group_id).await?;
    let actor = membership(connection, command.group_id, command.actor_id)
        .await?
        .ok_or(GroupsRepositoryError::MembershipRequired)?;
    if command.actor_id == command.target_user_id {
        if actor.1 == GroupRole::Owner || group.owner_id == command.actor_id {
            return Err(GroupsRepositoryError::OwnerConflict);
        }
        return delete_membership(connection, actor.0).await;
    }
    if actor.1 != GroupRole::Owner || group.owner_id != command.actor_id {
        return Err(GroupsRepositoryError::OwnerRequired);
    }
    let target = membership(connection, command.group_id, command.target_user_id)
        .await?
        .ok_or(GroupsRepositoryError::MemberNotFound)?;
    if target.1 == GroupRole::Owner || group.owner_id == command.target_user_id {
        return Err(GroupsRepositoryError::OwnerConflict);
    }
    delete_membership(connection, target.0).await
}

pub(super) async fn set_member_role(
    connection: &mut PgConnection,
    command: &SetMemberRoleCommand,
) -> Result<(), GroupsRepositoryError> {
    let group = lock_live_group(connection, command.group_id).await?;
    let actor = require_owner(connection, &group, command.actor_id).await?;
    let target = membership(connection, command.group_id, command.target_user_id)
        .await?
        .ok_or(GroupsRepositoryError::MemberNotFound)?;
    match command.role {
        GroupRole::Owner => {
            if target.1 == GroupRole::Owner || group.owner_id == command.target_user_id {
                return Err(GroupsRepositoryError::OwnerConflict);
            }
            sqlx::query("UPDATE groups SET owner_id = $2 WHERE id = $1")
                .bind(command.group_id)
                .bind(command.target_user_id)
                .execute(&mut *connection)
                .await
                .map_err(|error| database_error("group_owner_update", error))?;
            sqlx::query("UPDATE memberships SET role = 'member' WHERE id = $1")
                .bind(actor.0)
                .execute(&mut *connection)
                .await
                .map_err(|error| database_error("owner_demote", error))?;
            sqlx::query("UPDATE memberships SET role = 'owner' WHERE id = $1")
                .bind(target.0)
                .execute(connection)
                .await
                .map(|_| ())
                .map_err(|error| database_error("owner_promote", error))
        }
        GroupRole::Member => {
            if target.1 == GroupRole::Owner || group.owner_id == command.target_user_id {
                return Err(GroupsRepositoryError::OwnerConflict);
            }
            Ok(())
        }
    }
}

pub(super) async fn create_invite(
    connection: &mut PgConnection,
    command: &CreateInviteCommand,
) -> Result<InviteRecord, GroupsRepositoryError> {
    let group = lock_live_group(connection, command.group_id).await?;
    require_owner(connection, &group, command.actor_id).await?;
    let row = sqlx::query_as::<_, InviteRow>(
        "INSERT INTO invites \
         (id, group_id, code, created_by, expires_at, max_uses) \
         VALUES ($1, $2, $3, $4, $5, $6) \
         RETURNING id, group_id, code, created_by, expires_at, max_uses, used_count, created_at",
    )
    .bind(command.invite_id)
    .bind(command.group_id)
    .bind(&command.code)
    .bind(command.actor_id)
    .bind(command.expires_at)
    .bind(command.max_uses)
    .fetch_one(connection)
    .await
    .map_err(|error| database_error("invite_insert", error))?;
    Ok(invite_from_row(row))
}

pub(super) async fn redeem_invite(
    connection: &mut PgConnection,
    command: &RedeemInviteCommand,
) -> Result<InviteJoinRecord, GroupsRepositoryError> {
    let group_id = sqlx::query_scalar::<_, Uuid>("SELECT group_id FROM invites WHERE code = $1")
        .bind(&command.code)
        .fetch_optional(&mut *connection)
        .await
        .map_err(|error| database_error("invite_lookup", error))?
        .ok_or(GroupsRepositoryError::InviteNotFound)?;

    let group = lock_live_group(connection, group_id).await?;
    if membership(connection, group_id, command.actor_id)
        .await?
        .is_some()
    {
        return Ok(InviteJoinRecord {
            group_id,
            membership_id: None,
            joined: false,
        });
    }

    let invite = sqlx::query_as::<_, InviteRow>(
        "SELECT id, group_id, code, created_by, expires_at, max_uses, used_count, created_at \
         FROM invites WHERE code = $1 FOR UPDATE",
    )
    .bind(&command.code)
    .fetch_optional(&mut *connection)
    .await
    .map_err(|error| database_error("invite_lock", error))?
    .ok_or(GroupsRepositoryError::InviteNotFound)?;
    if invite.1 != group_id {
        return Err(GroupsRepositoryError::InvalidData);
    }

    if membership(connection, group_id, command.actor_id)
        .await?
        .is_some()
    {
        return Ok(InviteJoinRecord {
            group_id,
            membership_id: None,
            joined: false,
        });
    }
    if invite.4.is_some_and(|expires_at| expires_at <= command.now) {
        return Err(GroupsRepositoryError::InviteExpired);
    }
    if invite.5.is_some_and(|max_uses| invite.6 >= max_uses) {
        return Err(GroupsRepositoryError::InviteExhausted);
    }

    let member_count =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM memberships WHERE group_id = $1")
            .bind(group_id)
            .fetch_one(&mut *connection)
            .await
            .map_err(|error| database_error("membership_count", error))?;
    if member_count >= i64::from(group.max_members) {
        return Err(GroupsRepositoryError::GroupFull);
    }

    sqlx::query(
        "INSERT INTO memberships (id, group_id, user_id, role) \
         VALUES ($1, $2, $3, 'member')",
    )
    .bind(command.membership_id)
    .bind(group_id)
    .bind(command.actor_id)
    .execute(&mut *connection)
    .await
    .map_err(|error| database_error("invite_membership_insert", error))?;
    let consumed = sqlx::query_scalar::<_, i32>(
        "UPDATE invites SET used_count = used_count + 1 \
         WHERE id = $1 AND (max_uses IS NULL OR used_count < max_uses) \
         RETURNING used_count",
    )
    .bind(invite.0)
    .fetch_optional(connection)
    .await
    .map_err(|error| database_error("invite_consume", error))?;
    if consumed.is_none() {
        return Err(GroupsRepositoryError::InviteExhausted);
    }
    Ok(InviteJoinRecord {
        group_id,
        membership_id: Some(command.membership_id),
        joined: true,
    })
}

async fn lock_live_group(
    connection: &mut PgConnection,
    group_id: Uuid,
) -> Result<GroupRecord, GroupsRepositoryError> {
    sqlx::query_as::<_, GroupRow>(
        "SELECT g.id, g.name, g.owner_id, g.max_members, \
                (SELECT COUNT(*) FROM memberships all_members WHERE all_members.group_id = g.id), \
                g.created_at, main.id \
         FROM groups g \
         JOIN chatrooms main ON main.group_id = g.id AND main.type = 'main' \
         WHERE g.id = $1 AND g.deleted_at IS NULL \
         FOR UPDATE OF g",
    )
    .bind(group_id)
    .fetch_optional(connection)
    .await
    .map_err(|error| database_error("group_lock", error))?
    .map(group_from_row)
    .ok_or(GroupsRepositoryError::GroupNotFound)
}

async fn membership(
    connection: &mut PgConnection,
    group_id: Uuid,
    user_id: Uuid,
) -> Result<Option<MembershipRow>, GroupsRepositoryError> {
    let row = sqlx::query_as::<_, (Uuid, String)>(
        "SELECT id, role FROM memberships WHERE group_id = $1 AND user_id = $2",
    )
    .bind(group_id)
    .bind(user_id)
    .fetch_optional(connection)
    .await
    .map_err(|error| database_error("membership_lookup", error))?;
    row.map(|(id, role)| {
        GroupRole::parse(&role)
            .map(|role| (id, role))
            .ok_or(GroupsRepositoryError::InvalidData)
    })
    .transpose()
}

async fn require_owner(
    connection: &mut PgConnection,
    group: &GroupRecord,
    actor_id: Uuid,
) -> Result<MembershipRow, GroupsRepositoryError> {
    let actor = membership(connection, group.id, actor_id)
        .await?
        .ok_or(GroupsRepositoryError::MembershipRequired)?;
    if actor.1 != GroupRole::Owner || group.owner_id != actor_id {
        return Err(GroupsRepositoryError::OwnerRequired);
    }
    Ok(actor)
}

async fn delete_membership(
    connection: &mut PgConnection,
    membership_id: Uuid,
) -> Result<(), GroupsRepositoryError> {
    sqlx::query("DELETE FROM memberships WHERE id = $1")
        .bind(membership_id)
        .execute(connection)
        .await
        .map(|_| ())
        .map_err(|error| database_error("membership_delete", error))
}

fn invite_from_row(row: InviteRow) -> InviteRecord {
    InviteRecord {
        id: row.0,
        group_id: row.1,
        code: row.2,
        created_by: row.3,
        expires_at: row.4,
        max_uses: row.5,
        used_count: row.6,
        created_at: row.7,
    }
}
