#[tokio::test]
async fn http_prebridge_golden_error_envelopes_remain_exact_for_all_three_bridged_posts()
-> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let result: TestResult = async {
        let fixture = PostgresFixture::new(pool.clone()).await?;
        let router = production_router(&app_config_for(&pool).await?, &auth_config()?)?;
        let cases = [
            GoldenRequest {
                name: "messaging request validation",
                path: format!("/api/v1/chatrooms/{}/messages", fixture.topic_chatroom_id),
                idempotency_key: Some(Uuid::new_v4()),
                body: json!({"client_msg_id": "not-a-uuid", "body": "x"}),
                authenticated: true,
                expected: GoldenError::request_validation(),
            },
            GoldenRequest {
                name: "topic request validation",
                path: format!("/api/v1/groups/{}/topics", fixture.send.group_id),
                idempotency_key: Some(Uuid::new_v4()),
                body: json!({"title": 1}),
                authenticated: true,
                expected: GoldenError::request_validation(),
            },
            GoldenRequest {
                name: "read request validation",
                path: format!("/api/v1/chatrooms/{}/read", fixture.topic_chatroom_id),
                idempotency_key: None,
                body: json!({"cursor": 1}),
                authenticated: true,
                expected: GoldenError::request_validation(),
            },
            GoldenRequest {
                name: "messaging authentication failure",
                path: format!("/api/v1/chatrooms/{}/messages", fixture.topic_chatroom_id),
                idempotency_key: Some(Uuid::new_v4()),
                body: json!({"client_msg_id": Uuid::new_v4(), "body": "x"}),
                authenticated: false,
                expected: GoldenError::authentication_required(),
            },
            GoldenRequest {
                name: "topic authentication failure",
                path: format!("/api/v1/groups/{}/topics", fixture.send.group_id),
                idempotency_key: Some(Uuid::new_v4()),
                body: json!({"title": "topic"}),
                authenticated: false,
                expected: GoldenError::authentication_required(),
            },
            GoldenRequest {
                name: "read authentication failure",
                path: format!("/api/v1/chatrooms/{}/read", fixture.topic_chatroom_id),
                idempotency_key: None,
                body: json!({"cursor": "1"}),
                authenticated: false,
                expected: GoldenError::authentication_required(),
            },
        ];
        for case in cases {
            let response = if case.authenticated {
                post_json(
                    router.clone(),
                    fixture.send.message.sender_id,
                    &case.path,
                    case.idempotency_key,
                    case.body,
                )
                .await?
            } else {
                post_json_without_auth(router.clone(), &case.path, case.idempotency_key, case.body)
                    .await?
            };
            assert_golden_error(&response, case.name, case.expected)?;
        }

        let message_client_msg_id = Uuid::new_v4();
        let message = post_json(
            router.clone(),
            fixture.send.message.sender_id,
            &format!("/api/v1/chatrooms/{}/messages", fixture.topic_chatroom_id),
            Some(message_client_msg_id),
            json!({"client_msg_id": message_client_msg_id, "body": "Task-12 golden message"}),
        )
        .await?;
        assert_golden_body(
            &message,
            "messaging created",
            StatusCode::CREATED,
            r#"{"id":"<resource_uuid>","chatroom_id":"<resource_uuid>","sender_id":"<resource_uuid>","client_msg_id":"<resource_uuid>","body":"Task-12 golden message","type":"user","created_at":"<timestamp>","media":[]}"#,
        )?;

        let topic_key = Uuid::new_v4();
        let topic = post_json(
            router.clone(),
            fixture.topic.topic.author_id,
            &format!("/api/v1/groups/{}/topics", fixture.topic.topic.group_id),
            Some(topic_key),
            json!({"title": "Task-12 golden topic"}),
        )
        .await?;
        assert_golden_body(
            &topic,
            "topic created",
            StatusCode::CREATED,
            r#"{"id":"<resource_uuid>","group_id":"<resource_uuid>","author_id":"<resource_uuid>","author_nickname":"Task-12 author","author_avatar_url":null,"title":"Task-12 golden topic","body":null,"status":"seed","tags":[],"media":[],"chatroom_id":"<resource_uuid>","unread":false,"created_at":"<timestamp>","updated_at":"<timestamp>"}"#,
        )?;

        let read = post_json(
            router,
            fixture.recipient_id,
            &format!("/api/v1/chatrooms/{}/read", fixture.topic_chatroom_id),
            None,
            json!({"cursor": fixture.read.read.cursor.to_string()}),
        )
        .await?;
        assert_golden_body(
            &read,
            "read success",
            StatusCode::OK,
            &format!(
                r#"{{"chatroom_id":"<resource_uuid>","last_read_cursor":"{}","updated_at":"<timestamp>"}}"#,
                fixture.read.read.cursor
            ),
        )?;
        Ok(())
    }
    .await;
    pool.close().await;
    database.dispose().await?;
    result
}

async fn run(boundary: Boundary) -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let result: TestResult = async {
        let fixture = PostgresFixture::new(pool.clone()).await?;
        let baseline = Snapshot::load(&pool, fixture.audio_upload_id).await?;
        let router = production_router(&app_config_for(&pool).await?, &auth_config()?)?;

        let armed = arm(&pool, boundary, &fixture).await?;
        let response = invoke(router.clone(), &fixture, boundary).await?;
        let armed_phase: TestResult = async {
            if response.status == StatusCode::NOT_FOUND {
                return Err(format!(
                    "Task-12 RED: exact API composition is missing the HTTP entrypoint for {}",
                    boundary.name()
                )
                .into());
            }
            require_error(
                &response,
                StatusCode::SERVICE_UNAVAILABLE,
                "database_unavailable",
            )?;
            require(
                armed.witness_reached(&pool).await?,
                "durable trigger witness was not reached exactly once",
            )?;
            require(
                Snapshot::load(&pool, fixture.audio_upload_id).await? == baseline,
                "armed HTTP UoW left durable partial state",
            )
        }
        .await;

        armed.disarm(&pool).await?;
        armed_phase?;
        let retry = invoke(router, &fixture, boundary).await?;
        require(
            retry.status == boundary.success_status(),
            "clean HTTP retry did not return its normal status",
        )?;
        let after = Snapshot::load(&pool, fixture.audio_upload_id).await?;
        require(
            after.counts.delta(baseline.counts) == boundary.delta(),
            "clean HTTP retry delta differed or duplicated durable rows",
        )?;
        retry_relations(&pool, &fixture, boundary, &retry.body).await
    }
    .await;
    pool.close().await;
    database.dispose().await?;
    result
}

impl Boundary {
    fn name(self) -> &'static str {
        match self {
            Self::SendCore => "SendMessage message/event/outbox",
            Self::SendMedia => "SendMessage media binding",
            Self::SendNotification => "SendMessage notification/push",
            Self::TopicCore => "CreateTopic topic/chatroom/bootstrap/announcement/read",
            Self::TopicNotification => "CreateTopic notification/push",
            Self::ReadMarker => "MarkConversationRead marker",
            Self::ReadClear => "MarkConversationRead notification clear",
        }
    }
    fn target(self) -> (&'static str, &'static str) {
        match self {
            Self::SendCore => ("outbox_events", "INSERT"),
            Self::SendMedia => ("message_media", "INSERT"),
            Self::SendNotification | Self::TopicNotification => ("push_delivery_intents", "INSERT"),
            // Topic core's final concrete repository write is the announcement outbox row.
            Self::TopicCore => ("outbox_events", "INSERT"),
            Self::ReadMarker => ("chatroom_reads", "INSERT"),
            Self::ReadClear => ("notifications", "UPDATE OF read_at"),
        }
    }
    fn delta(self) -> DurableSnapshot {
        match self {
            Self::SendCore | Self::SendMedia | Self::SendNotification => DurableSnapshot {
                messages: 1,
                events: 1,
                outbox: 1,
                media: 1,
                topics: 0,
                chatrooms: 0,
                reads: 0,
                notifications: 1,
                unread: 1,
                push: 1,
                confirmed: -1,
                bound: 1,
            },
            Self::TopicCore | Self::TopicNotification => DurableSnapshot {
                messages: 1,
                events: 2,
                outbox: 2,
                media: 0,
                topics: 1,
                chatrooms: 1,
                reads: 1,
                notifications: 1,
                unread: 1,
                push: 1,
                confirmed: 0,
                bound: 0,
            },
            Self::ReadMarker | Self::ReadClear => DurableSnapshot {
                messages: 0,
                events: 0,
                outbox: 0,
                media: 0,
                topics: 0,
                chatrooms: 0,
                reads: 1,
                notifications: 0,
                unread: -1,
                push: 0,
                confirmed: 0,
                bound: 0,
            },
        }
    }
    fn success_status(self) -> StatusCode {
        match self {
            Self::SendCore
            | Self::SendMedia
            | Self::SendNotification
            | Self::TopicCore
            | Self::TopicNotification => StatusCode::CREATED,
            Self::ReadMarker | Self::ReadClear => StatusCode::OK,
        }
    }
}

struct HttpResponse {
    status: StatusCode,
    content_type: Option<String>,
    body: String,
}

struct GoldenRequest {
    name: &'static str,
    path: String,
    idempotency_key: Option<Uuid>,
    body: Value,
    authenticated: bool,
    expected: GoldenError,
}

#[derive(Clone, Copy)]
struct GoldenError {
    status: StatusCode,
    code: &'static str,
    message: &'static str,
}

impl GoldenError {
    const fn request_validation() -> Self {
        Self {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            code: "request_validation_failed",
            message: "요청 형식이 올바르지 않습니다.",
        }
    }

    const fn authentication_required() -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            code: "authentication_required",
            message: "인증이 필요합니다.",
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
struct DurableRows {
    messages: i64,
    media_bindings: i64,
    notifications: i64,
    pushes: i64,
}

impl DurableRows {
    async fn load(pool: &PgPool, chatroom_id: Uuid) -> TestResult<Self> {
        let (messages, media_bindings, notifications, pushes) = sqlx::query_as(
            "SELECT \
                 (SELECT count(*) FROM messages WHERE chatroom_id = $1), \
                 (SELECT count(*) FROM message_media binding \
                  JOIN messages message ON message.id = binding.message_id \
                  WHERE message.chatroom_id = $1), \
                 (SELECT count(*) FROM notifications WHERE conversation_id = $1), \
                 (SELECT count(*) FROM push_delivery_intents push \
                  JOIN conversation_events event ON event.id = push.source_event_id \
                  WHERE event.conversation_id = $1)",
        )
        .bind(chatroom_id)
        .fetch_one(pool)
        .await?;
        Ok(Self {
            messages,
            media_bindings,
            notifications,
            pushes,
        })
    }
}

async fn assert_bodyless_error(upload_ids: Vec<Uuid>, code: &'static str) -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let result: TestResult = async {
        let fixture = PostgresFixture::new(pool.clone()).await?;
        let before = DurableRows::load(&pool, fixture.topic_chatroom_id).await?;
        let response = send_message(
            production_router(&app_config_for(&pool).await?, &auth_config()?)?,
            fixture.send.message.sender_id,
            fixture.topic_chatroom_id,
            Uuid::new_v4(),
            None,
            upload_ids,
        )
        .await?;
        require_error(&response, StatusCode::UNPROCESSABLE_ENTITY, code)?;
        require(
            DurableRows::load(&pool, fixture.topic_chatroom_id).await? == before,
            "bodyless zero-upload rejection mutated durable state",
        )
    }
    .await;
    pool.close().await;
    database.dispose().await?;
    result
}

async fn send_message(
    router: Router,
    actor: Uuid,
    chatroom_id: Uuid,
    client_msg_id: Uuid,
    body: Option<&str>,
    upload_ids: Vec<Uuid>,
) -> Result<HttpResponse, tower::BoxError> {
    let payload = json!({
        "client_msg_id": client_msg_id,
        "body": body,
        "media": upload_ids
            .into_iter()
            .map(|media_upload_id| json!({ "media_upload_id": media_upload_id }))
            .collect::<Vec<_>>(),
    });
    let response = router
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(format!("/api/v1/chatrooms/{chatroom_id}/messages"))
                .header(CONTENT_TYPE, "application/json")
                .header(AUTHORIZATION, format!("Bearer {}", token(actor)?))
                .header("idempotency-key", client_msg_id.to_string())
                .body(Body::from(serde_json::to_vec(&payload)?))?,
        )
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
async fn post_json(
    router: Router,
    actor: Uuid,
    path: &str,
    idempotency_key: Option<Uuid>,
    payload: Value,
) -> Result<HttpResponse, tower::BoxError> {
    let mut builder = Request::builder()
        .method(Method::POST)
        .uri(path)
        .header(CONTENT_TYPE, "application/json")
        .header(AUTHORIZATION, format!("Bearer {}", token(actor)?));
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
