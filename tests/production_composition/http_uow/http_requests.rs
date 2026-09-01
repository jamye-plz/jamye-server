async fn post_json_without_auth(
    router: Router,
    path: &str,
    idempotency_key: Option<Uuid>,
    payload: Value,
) -> Result<HttpResponse, tower::BoxError> {
    let mut builder = Request::builder()
        .method(Method::POST)
        .uri(path)
        .header(CONTENT_TYPE, "application/json");
    if let Some(idempotency_key) = idempotency_key {
        builder = builder.header("idempotency-key", idempotency_key.to_string());
    }
    let response = router
        .oneshot(builder.body(Body::from(serde_json::to_vec(&payload)?))?)
        .await?;
    let status = response.status();
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let body = to_bytes(response.into_body(), 64 * 1024).await?;
    Ok(HttpResponse {
        status,
        content_type,
        body: String::from_utf8_lossy(&body).into_owned(),
    })
}

fn assert_golden_error(
    response: &HttpResponse,
    name: &str,
    expected_error: GoldenError,
) -> TestResult {
    let expected = format!(
        r#"{{"error":{{"code":"{}","message":"{}","request_id":"<request_id>","details":null}}}}"#,
        expected_error.code, expected_error.message,
    );
    assert_golden_body(response, name, expected_error.status, &expected)
}

fn assert_golden_body(
    response: &HttpResponse,
    name: &str,
    expected_status: StatusCode,
    expected_body: &str,
) -> TestResult {
    let body = normalize_dynamic_body_bytes(&response.body)?;
    require(
        response.status == expected_status
            && response.content_type.as_deref() == Some("application/json")
            && body == expected_body,
        &format!(
            "{name} golden baseline changed: status={}, content_type={:?}, body={body}",
            response.status, response.content_type
        ),
    )
}

fn normalize_dynamic_body_bytes(body: &str) -> TestResult<String> {
    let value: Value = serde_json::from_str(body).map_err(|error| {
        io::Error::other(format!(
            "golden response was not JSON before dynamic normalization: {error}; body={body}"
        ))
    })?;
    let mut replacements = Vec::new();
    collect_dynamic_replacements(&value, &mut replacements);
    replacements.sort_by_key(|replacement| std::cmp::Reverse(replacement.0.len()));
    let mut normalized = body.to_owned();
    for (from, to) in replacements {
        normalized =
            normalized.replace(&serde_json::to_string(&from)?, &serde_json::to_string(to)?);
    }
    Ok(normalized)
}

fn collect_dynamic_replacements(value: &Value, replacements: &mut Vec<(String, &'static str)>) {
    let Value::Object(fields) = value else {
        return;
    };
    for (name, field) in fields {
        match field {
            Value::String(text) if name == "request_id" && Uuid::try_parse(text).is_ok() => {
                replacements.push((text.clone(), "<request_id>"));
            }
            Value::String(text)
                if matches!(
                    name.as_str(),
                    "id" | "chatroom_id"
                        | "group_id"
                        | "topic_id"
                        | "author_id"
                        | "sender_id"
                        | "user_id"
                        | "client_msg_id"
                        | "media_upload_id"
                        | "source_event_id"
                ) && Uuid::try_parse(text).is_ok() =>
            {
                replacements.push((text.clone(), "<resource_uuid>"));
            }
            Value::String(text)
                if matches!(
                    name.as_str(),
                    "created_at" | "updated_at" | "confirmed_at" | "consumed_at"
                ) =>
            {
                replacements.push((text.clone(), "<timestamp>"));
            }
            Value::Array(values) => {
                for value in values {
                    collect_dynamic_replacements(value, replacements);
                }
            }
            Value::Object(_) => collect_dynamic_replacements(field, replacements),
            _ => {}
        }
    }
}

async fn insert_upload(
    pool: &PgPool,
    user_id: Uuid,
    chatroom_id: Uuid,
    confirmed: bool,
) -> TestResult<Uuid> {
    let upload_id = Uuid::new_v4();
    sqlx::query(
        "WITH stamped AS (SELECT clock_timestamp() AS now) \
         INSERT INTO media_uploads \
             (id, user_id, object_key, scope, target_id, content_type, byte_size, duration, \
              filename, status, confirmed_at, expires_at, created_at) \
         SELECT $1, $2, $3, 'chat', $4, 'audio/ogg', 8192, 38, \
                'task-12-contract-audio.ogg', \
                CASE WHEN $5 THEN 'confirmed' ELSE 'pending' END, \
                CASE WHEN $5 THEN stamped.now ELSE NULL END, \
                stamped.now + INTERVAL '1 hour', stamped.now \
         FROM stamped",
    )
    .bind(upload_id)
    .bind(user_id)
    .bind(format!("chat/{chatroom_id}/{upload_id}"))
    .bind(chatroom_id)
    .bind(confirmed)
    .execute(pool)
    .await?;
    Ok(upload_id)
}

fn require_error(response: &HttpResponse, status: StatusCode, code: &'static str) -> TestResult {
    let message = match code {
        "message_content_required" => "메시지 본문 또는 미디어가 필요합니다.",
        "media_not_available" => "미디어 메시지는 아직 사용할 수 없습니다.",
        "database_unavailable" => "데이터베이스를 사용할 수 없습니다.",
        _ => return Err(format!("missing exact golden message for {code}").into()),
    };
    assert_golden_error(
        response,
        "HTTP error envelope",
        GoldenError {
            status,
            code,
            message,
        },
    )
}

async fn invoke(
    router: Router,
    fixture: &PostgresFixture,
    boundary: Boundary,
) -> Result<HttpResponse, tower::BoxError> {
    let (method, path, body, actor, idempotency) = match boundary {
        Boundary::SendCore | Boundary::SendMedia | Boundary::SendNotification => (
            Method::POST,
            format!("/api/v1/chatrooms/{}/messages", fixture.topic_chatroom_id),
            format!(
                r#"{{"client_msg_id":"{}","body":null,"media":[{{"media_upload_id":"{}"}}]}}"#,
                fixture.send.message.client_msg_id, fixture.audio_upload_id
            ),
            fixture.send.message.sender_id,
            Some(fixture.send.message.client_msg_id),
        ),
        Boundary::TopicCore | Boundary::TopicNotification => (
            Method::POST,
            format!("/api/v1/groups/{}/topics", fixture.topic.topic.group_id),
            format!(r#"{{"title":"{}"}}"#, fixture.topic.topic.title),
            fixture.topic.topic.author_id,
            Some(fixture.topic.topic.idempotency_key),
        ),
        Boundary::ReadMarker | Boundary::ReadClear => (
            Method::POST,
            format!("/api/v1/chatrooms/{}/read", fixture.topic_chatroom_id),
            format!(r#"{{"cursor":"{}"}}"#, fixture.read.read.cursor),
            fixture.read.read.user_id,
            None,
        ),
    };
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header(CONTENT_TYPE, "application/json")
        .header(AUTHORIZATION, format!("Bearer {}", token(actor)?));
    if let Some(key) = idempotency {
        builder = builder.header("idempotency-key", key.to_string());
    }
    let response = router.oneshot(builder.body(Body::from(body))?).await?;
    let status = response.status();
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let body = to_bytes(response.into_body(), 64 * 1024).await?;
    Ok(HttpResponse {
        status,
        content_type,
        body: String::from_utf8_lossy(&body).into_owned(),
    })
}

struct ArmedFault {
    schema: String,
    sequence: String,
    function: String,
    trigger: String,
    table: &'static str,
}

impl ArmedFault {
    async fn witness_reached(&self, pool: &PgPool) -> TestResult<bool> {
        let statement = format!(
            "SELECT last_value, is_called FROM {}",
            qualified(&self.schema, &self.sequence)
        );
        let row: (i64, bool) = sqlx::query_as(AssertSqlSafe(statement))
            .fetch_one(pool)
            .await?;
        Ok(row == (1, true))
    }

    async fn disarm(&self, pool: &PgPool) -> TestResult {
        sqlx::query(AssertSqlSafe(format!(
            "DROP TRIGGER IF EXISTS {} ON public.{}",
            quote_identifier(&self.trigger),
            self.table
        )))
        .execute(pool)
        .await?;
        sqlx::query(AssertSqlSafe(format!(
            "DROP FUNCTION IF EXISTS {}()",
            qualified(&self.schema, &self.function)
        )))
        .execute(pool)
        .await?;
        sqlx::query(AssertSqlSafe(format!(
            "DROP SEQUENCE IF EXISTS {}",
            qualified(&self.schema, &self.sequence)
        )))
        .execute(pool)
        .await?;
        sqlx::query(AssertSqlSafe(format!(
            "DROP SCHEMA IF EXISTS {}",
            quote_identifier(&self.schema)
        )))
        .execute(pool)
        .await?;
        Ok(())
    }
}

async fn arm(
    pool: &PgPool,
    boundary: Boundary,
    fixture: &PostgresFixture,
) -> TestResult<ArmedFault> {
    let suffix = Uuid::new_v4().simple().to_string();
    let fault = ArmedFault {
        schema: format!("t12_fault_{suffix}"),
        sequence: "witness".to_owned(),
        function: "raise_boundary".to_owned(),
        trigger: format!("t12_trigger_{suffix}"),
        table: boundary.target().0,
    };
    let (_, event) = boundary.target();
    let condition = trigger_condition(boundary, fixture);
    let sequence = qualified(&fault.schema, &fault.sequence);
    let function = qualified(&fault.schema, &fault.function);
    sqlx::query(AssertSqlSafe(format!(
        "CREATE SCHEMA {}",
        quote_identifier(&fault.schema)
    )))
    .execute(pool)
    .await?;
    sqlx::query(AssertSqlSafe(format!("CREATE SEQUENCE {sequence} START 1")))
        .execute(pool)
        .await?;
    sqlx::query(AssertSqlSafe(format!("CREATE FUNCTION {function}() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF {condition} THEN PERFORM nextval({}::regclass); RAISE EXCEPTION USING ERRCODE = 'P0001', MESSAGE = 'task12 test-only cumulative boundary'; END IF; RETURN NEW; END; $$", sql_literal(&sequence)))).execute(pool).await?;
    sqlx::query(AssertSqlSafe(format!(
        "CREATE TRIGGER {} AFTER {event} ON public.{} FOR EACH ROW EXECUTE FUNCTION {function}()",
        quote_identifier(&fault.trigger),
        fault.table
    )))
    .execute(pool)
    .await?;
    Ok(fault)
}

fn trigger_condition(boundary: Boundary, fixture: &PostgresFixture) -> String {
    match boundary {
        Boundary::SendCore => format!(
            "NEW.event_type = 'message.created' AND NEW.aggregate_id = {}::uuid AND EXISTS (SELECT 1 FROM conversation_events event JOIN messages message ON message.id = (event.payload ->> 'id')::uuid WHERE event.id = NEW.conversation_event_id AND message.chatroom_id = NEW.aggregate_id AND message.client_msg_id = {}::uuid)",
            sql_literal(&fixture.topic_chatroom_id.to_string()),
            sql_literal(&fixture.send.message.client_msg_id.to_string())
        ),
        Boundary::SendMedia => format!(
            "NEW.media_upload_id = {}::uuid",
            sql_literal(&fixture.audio_upload_id.to_string())
        ),
        Boundary::SendNotification => format!(
            "NEW.recipient_user_id = {}::uuid AND NEW.push_installation_id = {}::uuid AND EXISTS (SELECT 1 FROM messages message WHERE message.id = NEW.source_message_id AND message.client_msg_id = {}::uuid)",
            sql_literal(&fixture.recipient_id.to_string()),
            sql_literal(&fixture.push_installation_id.to_string()),
            sql_literal(&fixture.send.message.client_msg_id.to_string())
        ),
        Boundary::TopicCore => format!(
            "NEW.event_type = 'message.created' AND NEW.aggregate_id = {}::uuid AND EXISTS (SELECT 1 FROM topics topic JOIN chatrooms topic_room ON topic_room.topic_id = topic.id AND topic_room.type = 'topic' WHERE topic.group_id = {}::uuid AND topic.idempotency_key = {}::uuid AND topic.title = {} AND topic_room.group_id = topic.group_id)",
            sql_literal(&fixture.main_chatroom_id.to_string()),
            sql_literal(&fixture.topic.topic.group_id.to_string()),
            sql_literal(&fixture.topic.topic.idempotency_key.to_string()),
            sql_literal(&fixture.topic.topic.title)
        ),
        Boundary::TopicNotification => format!(
            "NEW.recipient_user_id = {}::uuid AND NEW.push_installation_id = {}::uuid AND EXISTS (SELECT 1 FROM notifications notification JOIN topics topic ON topic.id = notification.topic_id WHERE notification.id = NEW.notification_id AND notification.type = 'new_topic' AND topic.group_id = {}::uuid AND topic.idempotency_key = {}::uuid AND topic.title = {})",
            sql_literal(&fixture.recipient_id.to_string()),
            sql_literal(&fixture.push_installation_id.to_string()),
            sql_literal(&fixture.topic.topic.group_id.to_string()),
            sql_literal(&fixture.topic.topic.idempotency_key.to_string()),
            sql_literal(&fixture.topic.topic.title)
        ),
        Boundary::ReadMarker => format!(
            "NEW.user_id = {}::uuid AND NEW.chatroom_id = {}::uuid AND NEW.last_read_cursor = {}",
            sql_literal(&fixture.recipient_id.to_string()),
            sql_literal(&fixture.topic_chatroom_id.to_string()),
            fixture.read.read.cursor
        ),
        Boundary::ReadClear => format!(
            "NEW.id = {}::uuid AND OLD.read_at IS NULL AND NEW.read_at IS NOT NULL",
            sql_literal(&fixture.seeded_notification_id.to_string())
        ),
    }
}

fn quote_identifier(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}
fn qualified(schema: &str, name: &str) -> String {
    format!("{}.{}", quote_identifier(schema), quote_identifier(name))
}
fn sql_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

#[derive(Clone, Debug, PartialEq)]
struct Snapshot {
    counts: DurableSnapshot,
    rows: Value,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DurableSnapshot {
    messages: i64,
    events: i64,
    outbox: i64,
    media: i64,
    topics: i64,
    chatrooms: i64,
    reads: i64,
    notifications: i64,
    unread: i64,
    push: i64,
    confirmed: i64,
    bound: i64,
}

impl DurableSnapshot {
    fn delta(self, before: Self) -> Self {
        Self {
            messages: self.messages - before.messages,
            events: self.events - before.events,
            outbox: self.outbox - before.outbox,
            media: self.media - before.media,
            topics: self.topics - before.topics,
            chatrooms: self.chatrooms - before.chatrooms,
            reads: self.reads - before.reads,
            notifications: self.notifications - before.notifications,
            unread: self.unread - before.unread,
            push: self.push - before.push,
            confirmed: self.confirmed - before.confirmed,
            bound: self.bound - before.bound,
        }
    }
}

impl Snapshot {
    async fn load(pool: &PgPool, upload: Uuid) -> TestResult<Self> {
        let row:(i64,i64,i64,i64,i64,i64,i64,i64,i64,i64,i64,i64) = sqlx::query_as("SELECT (SELECT count(*) FROM messages), (SELECT count(*) FROM conversation_events), (SELECT count(*) FROM outbox_events), (SELECT count(*) FROM message_media), (SELECT count(*) FROM topics), (SELECT count(*) FROM chatrooms), (SELECT count(*) FROM chatroom_reads), (SELECT count(*) FROM notifications), (SELECT count(*) FROM notifications WHERE read_at IS NULL), (SELECT count(*) FROM push_delivery_intents), (SELECT count(*) FROM media_uploads WHERE id = $1 AND status = 'confirmed' AND bound_message_id IS NULL), (SELECT count(*) FROM media_uploads WHERE id = $1 AND status = 'bound' AND bound_message_id IS NOT NULL AND consumed_at IS NOT NULL)").bind(upload).fetch_one(pool).await?;
        let rows: Json<Value> = sqlx::query_scalar("SELECT jsonb_build_object('messages', COALESCE((SELECT jsonb_agg(to_jsonb(message) ORDER BY message.id) FROM messages message), '[]'::jsonb), 'conversation_events', COALESCE((SELECT jsonb_agg(to_jsonb(event) ORDER BY event.id) FROM conversation_events event), '[]'::jsonb), 'outbox_events', COALESCE((SELECT jsonb_agg(to_jsonb(outbox) ORDER BY outbox.id) FROM outbox_events outbox), '[]'::jsonb), 'message_media', COALESCE((SELECT jsonb_agg(to_jsonb(media) ORDER BY media.message_id, media.media_upload_id) FROM message_media media), '[]'::jsonb), 'topics', COALESCE((SELECT jsonb_agg(to_jsonb(topic) ORDER BY topic.id) FROM topics topic), '[]'::jsonb), 'chatrooms', COALESCE((SELECT jsonb_agg(to_jsonb(chatroom) ORDER BY chatroom.id) FROM chatrooms chatroom), '[]'::jsonb), 'chatroom_reads', COALESCE((SELECT jsonb_agg(to_jsonb(marker) ORDER BY marker.id) FROM chatroom_reads marker), '[]'::jsonb), 'notifications', COALESCE((SELECT jsonb_agg(to_jsonb(notification) ORDER BY notification.id) FROM notifications notification), '[]'::jsonb), 'push_delivery_intents', COALESCE((SELECT jsonb_agg(to_jsonb(intent) ORDER BY intent.id) FROM push_delivery_intents intent), '[]'::jsonb), 'media_uploads', COALESCE((SELECT jsonb_agg(to_jsonb(upload) ORDER BY upload.id) FROM media_uploads upload), '[]'::jsonb))").fetch_one(pool).await?;
        Ok(Self {
            counts: DurableSnapshot {
                messages: row.0,
                events: row.1,
                outbox: row.2,
                media: row.3,
                topics: row.4,
                chatrooms: row.5,
                reads: row.6,
                notifications: row.7,
                unread: row.8,
                push: row.9,
                confirmed: row.10,
                bound: row.11,
            },
            rows: rows.0,
        })
    }
}
