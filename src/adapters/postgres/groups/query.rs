use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::ports::groups::{
    GetGroupQuery, GroupPage, GroupRecord, GroupRole, GroupsRepositoryError, ListGroupsQuery,
    ListMembersQuery, MemberPage, MemberRecord,
};

use super::database_error;

pub(super) type GroupRow = (Uuid, String, Uuid, i32, i64, OffsetDateTime, Uuid);
type GroupAccessRow = (Uuid, String, Uuid, i32, i64, OffsetDateTime, Uuid, bool);
type MemberAccessRow = (
    bool,
    bool,
    Option<Uuid>,
    Option<Uuid>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<OffsetDateTime>,
);

pub(super) async fn list_groups(
    pool: &PgPool,
    query: ListGroupsQuery,
) -> Result<GroupPage, GroupsRepositoryError> {
    let fetch_limit = i64::from(query.limit) + 1;
    let rows = sqlx::query_as::<_, GroupRow>(
        "SELECT g.id, g.name, g.owner_id, g.max_members, \
                (SELECT COUNT(*) FROM memberships all_members WHERE all_members.group_id = g.id), \
                g.created_at, main.id \
         FROM groups g \
         JOIN memberships actor_membership \
           ON actor_membership.group_id = g.id AND actor_membership.user_id = $1 \
         JOIN chatrooms main ON main.group_id = g.id AND main.type = 'main' \
         WHERE g.deleted_at IS NULL \
           AND ( \
             $2::uuid IS NULL \
             OR (g.created_at, g.id) > ( \
                 SELECT cursor_group.created_at, cursor_group.id \
                 FROM groups cursor_group \
                 JOIN memberships cursor_membership \
                   ON cursor_membership.group_id = cursor_group.id \
                  AND cursor_membership.user_id = $1 \
                 WHERE cursor_group.id = $2 AND cursor_group.deleted_at IS NULL \
             ) \
           ) \
         ORDER BY g.created_at, g.id \
         LIMIT $3",
    )
    .bind(query.user_id)
    .bind(query.after)
    .bind(fetch_limit)
    .fetch_all(pool)
    .await
    .map_err(|error| database_error("group_list", error))?;
    let mut items = rows.into_iter().map(group_from_row).collect::<Vec<_>>();
    let has_more = items.len() > query.limit as usize;
    if has_more {
        items.truncate(query.limit as usize);
    }
    let next_cursor = has_more
        .then(|| items.last().map(|group| group.id.to_string()))
        .flatten();
    Ok(GroupPage { items, next_cursor })
}

pub(super) async fn get_group(
    pool: &PgPool,
    query: GetGroupQuery,
) -> Result<GroupRecord, GroupsRepositoryError> {
    let row = sqlx::query_as::<_, GroupAccessRow>(
        "SELECT g.id, g.name, g.owner_id, g.max_members, \
                (SELECT COUNT(*) FROM memberships all_members WHERE all_members.group_id = g.id), \
                g.created_at, main.id, \
                EXISTS ( \
                    SELECT 1 FROM memberships actor_membership \
                    WHERE actor_membership.group_id = g.id \
                      AND actor_membership.user_id = $2 \
                ) \
         FROM groups g \
         JOIN chatrooms main ON main.group_id = g.id AND main.type = 'main' \
         WHERE g.id = $1 AND g.deleted_at IS NULL",
    )
    .bind(query.group_id)
    .bind(query.actor_id)
    .fetch_optional(pool)
    .await
    .map_err(|error| database_error("group_get", error))?
    .ok_or(GroupsRepositoryError::GroupNotFound)?;
    if !row.7 {
        return Err(GroupsRepositoryError::MembershipRequired);
    }
    Ok(group_from_row((
        row.0, row.1, row.2, row.3, row.4, row.5, row.6,
    )))
}

pub(super) async fn list_members(
    pool: &PgPool,
    query: ListMembersQuery,
) -> Result<MemberPage, GroupsRepositoryError> {
    let fetch_limit = i64::from(query.limit) + 1;
    let rows = sqlx::query_as::<_, MemberAccessRow>(
        "WITH access AS ( \
             SELECT \
                 EXISTS (SELECT 1 FROM groups WHERE id = $1 AND deleted_at IS NULL) AS live, \
                 EXISTS ( \
                     SELECT 1 FROM groups g \
                     JOIN memberships actor_membership ON actor_membership.group_id = g.id \
                     WHERE g.id = $1 AND g.deleted_at IS NULL \
                       AND actor_membership.user_id = $2 \
                 ) AS member \
         ), page AS ( \
             SELECT m.id AS membership_id, m.user_id, u.nickname, u.avatar_url, m.role, m.joined_at \
             FROM memberships m \
             JOIN users u ON u.id = m.user_id \
             CROSS JOIN access \
             WHERE m.group_id = $1 AND access.live AND access.member \
               AND ( \
                 $3::uuid IS NULL \
                 OR (CASE WHEN m.role = 'owner' THEN 0 ELSE 1 END, m.joined_at, m.id) > ( \
                     SELECT CASE WHEN cursor_membership.role = 'owner' THEN 0 ELSE 1 END, \
                            cursor_membership.joined_at, cursor_membership.id \
                     FROM memberships cursor_membership \
                     WHERE cursor_membership.id = $3 \
                       AND cursor_membership.group_id = $1 \
                 ) \
               ) \
             ORDER BY CASE WHEN m.role = 'owner' THEN 0 ELSE 1 END, m.joined_at, m.id \
             LIMIT $4 \
         ) \
         SELECT access.live, access.member, page.membership_id, page.user_id, \
                page.nickname, page.avatar_url, page.role, page.joined_at \
         FROM access LEFT JOIN page ON TRUE \
         ORDER BY CASE WHEN page.role = 'owner' THEN 0 ELSE 1 END, page.joined_at, page.membership_id",
    )
    .bind(query.group_id)
    .bind(query.actor_id)
    .bind(query.after)
    .bind(fetch_limit)
    .fetch_all(pool)
    .await
    .map_err(|error| database_error("member_list", error))?;
    let Some(first) = rows.first() else {
        return Err(GroupsRepositoryError::Unavailable);
    };
    if !first.0 {
        return Err(GroupsRepositoryError::GroupNotFound);
    }
    if !first.1 {
        return Err(GroupsRepositoryError::MembershipRequired);
    }
    let mut items = rows
        .into_iter()
        .filter_map(member_from_access_row)
        .collect::<Result<Vec<_>, _>>()?;
    let has_more = items.len() > query.limit as usize;
    if has_more {
        items.truncate(query.limit as usize);
    }
    let next_cursor = has_more
        .then(|| items.last().map(|member| member.membership_id.to_string()))
        .flatten();
    Ok(MemberPage { items, next_cursor })
}

pub(super) fn group_from_row(row: GroupRow) -> GroupRecord {
    GroupRecord {
        id: row.0,
        name: row.1,
        owner_id: row.2,
        max_members: row.3,
        member_count: row.4,
        created_at: row.5,
        main_chatroom_id: row.6,
    }
}

fn member_from_access_row(
    row: MemberAccessRow,
) -> Option<Result<MemberRecord, GroupsRepositoryError>> {
    let membership_id = row.2?;
    Some((|| {
        let role = row
            .6
            .as_deref()
            .and_then(GroupRole::parse)
            .ok_or(GroupsRepositoryError::InvalidData)?;
        Ok(MemberRecord {
            membership_id,
            user_id: row.3.ok_or(GroupsRepositoryError::InvalidData)?,
            nickname: row.4.ok_or(GroupsRepositoryError::InvalidData)?,
            avatar_url: row.5,
            role,
            joined_at: row.7.ok_or(GroupsRepositoryError::InvalidData)?,
        })
    })())
}
