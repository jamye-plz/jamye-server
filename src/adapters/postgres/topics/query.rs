use std::collections::HashMap;

use sqlx::PgPool;
use time::{Date, OffsetDateTime};
use uuid::Uuid;

use crate::ports::topics::{
    GetTopicQuery, ListTopicDatesQuery, ListTopicMediaQuery, ListTopicTagsQuery, ListTopicsQuery,
    TopicDatePage, TopicMediaPage, TopicMediaRecord, TopicPage, TopicRecord, TopicStatus,
    TopicTagPage, TopicTagRecord, TopicTagSource, TopicsRepositoryError,
};

use super::database_error;

pub(super) type TopicBaseRow = (
    Uuid,
    Uuid,
    Uuid,
    String,
    Option<String>,
    String,
    OffsetDateTime,
    OffsetDateTime,
    Uuid,
    String,
    Option<String>,
    bool,
);

pub(super) type TopicTagRow = (Uuid, Uuid, String, String, Option<f64>);
pub(super) type TopicMediaRow = (
    Uuid,
    Uuid,
    Uuid,
    String,
    String,
    Option<i32>,
    Option<i32>,
    Option<i64>,
    OffsetDateTime,
);
type TopicMediaAccessRow = (
    bool,
    bool,
    Option<Uuid>,
    Option<Uuid>,
    Option<Uuid>,
    Option<String>,
    Option<String>,
    Option<i32>,
    Option<i32>,
    Option<i64>,
    Option<OffsetDateTime>,
);

pub(super) async fn list_topics(
    pool: &PgPool,
    query: ListTopicsQuery,
) -> Result<TopicPage, TopicsRepositoryError> {
    require_group_access(pool, query.group_id, query.actor_id).await?;
    let fetch_limit = i64::from(query.limit) + 1;
    let rows = sqlx::query_as::<_, TopicBaseRow>(
        "SELECT t.id, t.group_id, t.author_id, t.title, t.body, t.status, \
                t.created_at, t.updated_at, topic_chat.id, author.nickname, \
                author.avatar_url, \
                EXISTS ( \
                    SELECT 1 FROM conversation_events event \
                    WHERE event.conversation_id = topic_chat.id \
                      AND event.cursor > COALESCE(( \
                          SELECT marker.last_read_cursor FROM chatroom_reads marker \
                          WHERE marker.user_id = $2 AND marker.chatroom_id = topic_chat.id \
                      ), 0) \
                ) AS unread \
         FROM topics t \
         JOIN chatrooms topic_chat \
           ON topic_chat.topic_id = t.id AND topic_chat.type = 'topic' \
         JOIN users author ON author.id = t.author_id \
         WHERE t.group_id = $1 \
           AND ( \
             $3::uuid IS NULL \
             OR (t.created_at, t.id) < ( \
                 SELECT cursor_topic.created_at, cursor_topic.id \
                 FROM topics cursor_topic \
                 WHERE cursor_topic.id = $3 AND cursor_topic.group_id = $1 \
             ) \
           ) \
           AND ( \
             $4::date IS NULL \
             OR timezone('Asia/Seoul', t.created_at)::date = $4 \
           ) \
         ORDER BY t.created_at DESC, t.id DESC \
         LIMIT $5",
    )
    .bind(query.group_id)
    .bind(query.actor_id)
    .bind(query.after)
    .bind(query.date)
    .bind(fetch_limit)
    .fetch_all(pool)
    .await
    .map_err(|error| database_error("topic_list", error))?;

    let mut topics = hydrate(pool, rows).await?;
    let has_more = topics.len() > query.limit as usize;
    if has_more {
        topics.truncate(query.limit as usize);
    }
    let next_cursor = has_more
        .then(|| topics.last().map(|topic| topic.id.to_string()))
        .flatten();
    Ok(TopicPage {
        items: topics,
        next_cursor,
    })
}

pub(super) async fn list_topic_dates(
    pool: &PgPool,
    query: ListTopicDatesQuery,
) -> Result<TopicDatePage, TopicsRepositoryError> {
    require_group_access(pool, query.group_id, query.actor_id).await?;
    let today =
        sqlx::query_scalar::<_, Date>("SELECT timezone('Asia/Seoul', clock_timestamp())::date")
            .fetch_one(pool)
            .await
            .map_err(|error| database_error("topic_today", error))?;
    let fetch_limit = i64::from(query.limit) + 1;
    let mut dates = sqlx::query_scalar::<_, Date>(
        "WITH days AS ( \
             SELECT DISTINCT timezone('Asia/Seoul', created_at)::date AS day \
             FROM topics WHERE group_id = $1 \
             UNION SELECT timezone('Asia/Seoul', clock_timestamp())::date \
         ) \
         SELECT day FROM days \
         WHERE $2::date IS NULL OR day < $2 \
         ORDER BY day DESC \
         LIMIT $3",
    )
    .bind(query.group_id)
    .bind(query.after)
    .bind(fetch_limit)
    .fetch_all(pool)
    .await
    .map_err(|error| database_error("topic_date_list", error))?;
    let has_more = dates.len() > query.limit as usize;
    if has_more {
        dates.truncate(query.limit as usize);
    }
    let next_cursor = has_more
        .then(|| dates.last().map(|date| date.to_string()))
        .flatten();
    Ok(TopicDatePage {
        dates: dates.into_iter().map(|date| date.to_string()).collect(),
        today: today.to_string(),
        next_cursor,
    })
}

pub(super) async fn get_topic(
    pool: &PgPool,
    query: GetTopicQuery,
) -> Result<TopicRecord, TopicsRepositoryError> {
    require_group_access(pool, query.group_id, query.actor_id).await?;
    let row = sqlx::query_as::<_, TopicBaseRow>(
        "SELECT t.id, t.group_id, t.author_id, t.title, t.body, t.status, \
                t.created_at, t.updated_at, topic_chat.id, author.nickname, \
                author.avatar_url, \
                EXISTS ( \
                    SELECT 1 FROM conversation_events event \
                    WHERE event.conversation_id = topic_chat.id \
                      AND event.cursor > COALESCE(( \
                          SELECT marker.last_read_cursor FROM chatroom_reads marker \
                          WHERE marker.user_id = $3 AND marker.chatroom_id = topic_chat.id \
                      ), 0) \
                ) AS unread \
         FROM topics t \
         JOIN chatrooms topic_chat \
           ON topic_chat.topic_id = t.id AND topic_chat.type = 'topic' \
         JOIN users author ON author.id = t.author_id \
         WHERE t.id = $1 AND t.group_id = $2",
    )
    .bind(query.topic_id)
    .bind(query.group_id)
    .bind(query.actor_id)
    .fetch_optional(pool)
    .await
    .map_err(|error| database_error("topic_get", error))?
    .ok_or(TopicsRepositoryError::TopicNotFound)?;
    hydrate(pool, vec![row])
        .await?
        .into_iter()
        .next()
        .ok_or(TopicsRepositoryError::InvalidData)
}

pub(super) async fn list_tags(
    pool: &PgPool,
    query: ListTopicTagsQuery,
) -> Result<TopicTagPage, TopicsRepositoryError> {
    require_group_access(pool, query.group_id, query.actor_id).await?;
    let exists = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM topics WHERE id = $1 AND group_id = $2)",
    )
    .bind(query.topic_id)
    .bind(query.group_id)
    .fetch_one(pool)
    .await
    .map_err(|error| database_error("topic_tag_parent", error))?;
    if !exists {
        return Err(TopicsRepositoryError::TopicNotFound);
    }
    let fetch_limit = i64::from(query.limit) + 1;
    let rows = sqlx::query_as::<_, TopicTagRow>(
        "SELECT id, topic_id, tag, source, confidence \
         FROM topic_tags \
         WHERE topic_id = $1 \
           AND ( \
             $2::uuid IS NULL \
             OR (tag, id) > ( \
                 SELECT cursor_tag.tag, cursor_tag.id \
                 FROM topic_tags cursor_tag \
                 WHERE cursor_tag.id = $2 AND cursor_tag.topic_id = $1 \
             ) \
           ) \
         ORDER BY tag, id \
         LIMIT $3",
    )
    .bind(query.topic_id)
    .bind(query.after)
    .bind(fetch_limit)
    .fetch_all(pool)
    .await
    .map_err(|error| database_error("topic_tag_list", error))?;
    let mut items = rows
        .into_iter()
        .map(tag_from_row)
        .collect::<Result<Vec<_>, _>>()?;
    let has_more = items.len() > query.limit as usize;
    if has_more {
        items.truncate(query.limit as usize);
    }
    let next_cursor = has_more
        .then(|| items.last().map(|tag| tag.id.to_string()))
        .flatten();
    Ok(TopicTagPage { items, next_cursor })
}

pub(super) async fn list_media(
    pool: &PgPool,
    query: ListTopicMediaQuery,
) -> Result<TopicMediaPage, TopicsRepositoryError> {
    let fetch_limit = i64::from(query.limit) + 1;
    let rows = sqlx::query_as::<_, TopicMediaAccessRow>(
        "WITH actor_access AS ( \
             SELECT \
                 EXISTS ( \
                     SELECT 1 FROM topics topic \
                     JOIN groups live_group \
                       ON live_group.id = topic.group_id \
                      AND live_group.deleted_at IS NULL \
                     JOIN memberships actor_membership \
                       ON actor_membership.group_id = live_group.id \
                      AND actor_membership.user_id = $2 \
                     WHERE topic.id = $1 \
                 ) AS member, \
                 ( \
                     $3::uuid IS NULL \
                     OR EXISTS ( \
                         SELECT 1 FROM topic_media cursor_media \
                         WHERE cursor_media.id = $3 \
                           AND cursor_media.topic_id = $1 \
                     ) \
                 ) AS cursor_valid \
         ), page AS ( \
             SELECT media.id, media.topic_id, media.media_upload_id, media.type, \
                    media.object_key, media.width, media.height, media.byte_size, \
                    media.created_at \
             FROM topic_media media \
             CROSS JOIN actor_access \
             WHERE media.topic_id = $1 \
               AND actor_access.member \
               AND actor_access.cursor_valid \
               AND ( \
                   $3::uuid IS NULL \
                   OR (media.created_at, media.id) > ( \
                       SELECT cursor_media.created_at, cursor_media.id \
                       FROM topic_media cursor_media \
                       WHERE cursor_media.id = $3 \
                         AND cursor_media.topic_id = $1 \
                   ) \
               ) \
             ORDER BY media.created_at, media.id \
             LIMIT $4 \
         ) \
         SELECT actor_access.member, actor_access.cursor_valid, page.id, page.topic_id, \
                page.media_upload_id, page.type, page.object_key, page.width, page.height, \
                page.byte_size, page.created_at \
         FROM actor_access LEFT JOIN page ON TRUE \
         ORDER BY page.created_at, page.id",
    )
    .bind(query.topic_id)
    .bind(query.actor_id)
    .bind(query.after)
    .bind(fetch_limit)
    .fetch_all(pool)
    .await
    .map_err(|error| database_error("topic_media_list", error))?;

    let first = rows.first().ok_or(TopicsRepositoryError::Unavailable)?;
    require_media_access(first.0, first.1)?;
    let mut items = rows
        .into_iter()
        .filter_map(media_from_access_row)
        .collect::<Result<Vec<_>, _>>()?;
    let has_more = items.len() > query.limit as usize;
    if has_more {
        items.truncate(query.limit as usize);
    }
    let next_cursor = has_more
        .then(|| items.last().map(|media| media.id.to_string()))
        .flatten();
    Ok(TopicMediaPage { items, next_cursor })
}

fn require_media_access(member: bool, cursor_valid: bool) -> Result<(), TopicsRepositoryError> {
    if !member || !cursor_valid {
        return Err(TopicsRepositoryError::MembershipRequired);
    }
    Ok(())
}

fn media_from_access_row(
    row: TopicMediaAccessRow,
) -> Option<Result<TopicMediaRecord, TopicsRepositoryError>> {
    let (
        _,
        _,
        id,
        topic_id,
        media_upload_id,
        content_type,
        object_key,
        width,
        height,
        byte_size,
        created_at,
    ) = row;
    let Some(id) = id else {
        return if topic_id.is_none()
            && media_upload_id.is_none()
            && content_type.is_none()
            && object_key.is_none()
            && width.is_none()
            && height.is_none()
            && byte_size.is_none()
            && created_at.is_none()
        {
            None
        } else {
            Some(Err(TopicsRepositoryError::InvalidData))
        };
    };
    Some(
        match (
            topic_id,
            media_upload_id,
            content_type,
            object_key,
            created_at,
        ) {
            (
                Some(topic_id),
                Some(media_upload_id),
                Some(content_type),
                Some(object_key),
                Some(created_at),
            ) => Ok(TopicMediaRecord {
                id,
                topic_id,
                media_upload_id,
                content_type,
                object_key,
                width,
                height,
                byte_size,
                created_at,
            }),
            _ => Err(TopicsRepositoryError::InvalidData),
        },
    )
}

async fn require_group_access(
    pool: &PgPool,
    group_id: Uuid,
    actor_id: Uuid,
) -> Result<(), TopicsRepositoryError> {
    let (live, member) = sqlx::query_as::<_, (bool, bool)>(
        "SELECT \
             EXISTS (SELECT 1 FROM groups WHERE id = $1 AND deleted_at IS NULL), \
             EXISTS ( \
                 SELECT 1 FROM groups g \
                 JOIN memberships membership ON membership.group_id = g.id \
                 WHERE g.id = $1 AND g.deleted_at IS NULL \
                   AND membership.user_id = $2 \
             )",
    )
    .bind(group_id)
    .bind(actor_id)
    .fetch_one(pool)
    .await
    .map_err(|error| database_error("topic_access", error))?;
    if !live {
        return Err(TopicsRepositoryError::GroupNotFound);
    }
    if !member {
        return Err(TopicsRepositoryError::MembershipRequired);
    }
    Ok(())
}

async fn hydrate(
    pool: &PgPool,
    rows: Vec<TopicBaseRow>,
) -> Result<Vec<TopicRecord>, TopicsRepositoryError> {
    let mut topics = rows
        .into_iter()
        .map(topic_from_row)
        .collect::<Result<Vec<_>, _>>()?;
    if topics.is_empty() {
        return Ok(topics);
    }
    let topic_ids = topics.iter().map(|topic| topic.id).collect::<Vec<_>>();
    let positions = topics
        .iter()
        .enumerate()
        .map(|(position, topic)| (topic.id, position))
        .collect::<HashMap<_, _>>();
    let tags = sqlx::query_as::<_, TopicTagRow>(
        "SELECT id, topic_id, tag, source, confidence \
         FROM topic_tags WHERE topic_id = ANY($1) ORDER BY tag, id",
    )
    .bind(&topic_ids)
    .fetch_all(pool)
    .await
    .map_err(|error| database_error("topic_tags_hydrate", error))?;
    for row in tags {
        let topic_id = row.1;
        let position = positions
            .get(&topic_id)
            .copied()
            .ok_or(TopicsRepositoryError::InvalidData)?;
        topics[position].tags.push(tag_from_row(row)?);
    }
    let media = sqlx::query_as::<_, TopicMediaRow>(
        "SELECT id, topic_id, media_upload_id, type, object_key, width, height, byte_size, created_at \
         FROM topic_media WHERE topic_id = ANY($1) ORDER BY created_at, id",
    )
    .bind(&topic_ids)
    .fetch_all(pool)
    .await
    .map_err(|error| database_error("topic_media_hydrate", error))?;
    for row in media {
        let topic_id = row.1;
        let position = positions
            .get(&topic_id)
            .copied()
            .ok_or(TopicsRepositoryError::InvalidData)?;
        topics[position].media.push(media_from_row(row));
    }
    Ok(topics)
}

pub(super) fn topic_from_row(row: TopicBaseRow) -> Result<TopicRecord, TopicsRepositoryError> {
    let status = TopicStatus::parse(&row.5).ok_or(TopicsRepositoryError::InvalidData)?;
    Ok(TopicRecord {
        id: row.0,
        group_id: row.1,
        author_id: row.2,
        title: row.3,
        body: row.4,
        status,
        created_at: row.6,
        updated_at: row.7,
        chatroom_id: row.8,
        author_nickname: row.9,
        author_avatar_url: row.10,
        unread: row.11,
        tags: Vec::new(),
        media: Vec::new(),
    })
}

pub(super) fn tag_from_row(row: TopicTagRow) -> Result<TopicTagRecord, TopicsRepositoryError> {
    let source = TopicTagSource::parse(&row.3).ok_or(TopicsRepositoryError::InvalidData)?;
    Ok(TopicTagRecord {
        id: row.0,
        topic_id: row.1,
        tag: row.2,
        source,
        confidence: row.4,
    })
}

pub(super) fn media_from_row(row: TopicMediaRow) -> TopicMediaRecord {
    TopicMediaRecord {
        id: row.0,
        topic_id: row.1,
        media_upload_id: row.2,
        content_type: row.3,
        object_key: row.4,
        width: row.5,
        height: row.6,
        byte_size: row.7,
        created_at: row.8,
    }
}
