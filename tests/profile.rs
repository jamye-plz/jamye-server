use std::{error::Error, fs, io, sync::Arc};

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header::AUTHORIZATION},
};
use jamye_server::{
    adapters::{
        oauth::{OsCredentialSource, ProductionTokenCodec},
        postgres::{auth::PostgresAuthRepository, transactions::SqlxTransactionManager},
    },
    application::users::{PatchValue, UserError, UserPatch, UserService},
    ports::{
        auth::{
            AccessTokenIssuer, AuthRepository, CredentialSource, NewProviderIdentity,
            NewRefreshSession,
        },
        transactions::TransactionManager,
    },
    transport::http::users::{UserHttpState, router as user_router},
};
use serde_json::{Value, json};
use sqlx::{Connection, PgPool};
use time::OffsetDateTime;
use tower::ServiceExt;
use uuid::Uuid;

#[path = "support/postgres.rs"]
mod postgres_support;
use postgres_support::TestDatabase;

type TestResult<T = ()> = Result<T, Box<dyn Error + Send + Sync>>;

const HTTPS_AVATAR_MIGRATION: &str = "migrations/0013_https_avatar_urls.sql";

#[tokio::test]
async fn profile_patch_preserves_omitted_and_null_fields_and_empty_avatar_clears() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let (user_id, _, service, _) = profile_fixture(pool.clone()).await?;

    let original = service.get(user_id).await?;
    let unchanged = service
        .update(
            user_id,
            UserPatch {
                nickname: PatchValue::Omitted,
                avatar_url: PatchValue::Null,
            },
        )
        .await?;
    assert_eq!(unchanged, original);
    let renamed = service
        .update(
            user_id,
            UserPatch {
                nickname: PatchValue::Value("새 별명".to_owned()),
                avatar_url: PatchValue::Value(String::new()),
            },
        )
        .await?;
    assert_eq!(renamed.nickname, "새 별명");
    assert_eq!(renamed.avatar_url, None);
    let repeated = service
        .update(
            user_id,
            UserPatch {
                nickname: PatchValue::Value("새 별명".to_owned()),
                avatar_url: PatchValue::Omitted,
            },
        )
        .await?;
    assert_eq!(repeated, renamed);
    assert_eq!(
        service
            .update(
                user_id,
                UserPatch {
                    nickname: PatchValue::Value(String::new()),
                    avatar_url: PatchValue::Omitted,
                },
            )
            .await,
        Err(UserError::RequestValidation)
    );
    let changed_avatar = service
        .update(
            user_id,
            UserPatch {
                nickname: PatchValue::Omitted,
                avatar_url: PatchValue::Value("https://images.example/new.png".to_owned()),
            },
        )
        .await?;
    assert_eq!(
        changed_avatar.avatar_url.as_deref(),
        Some("https://images.example/new.png")
    );
    for invalid in [
        "http://images.example/avatar.png".to_owned(),
        "HTTPS://images.example/avatar.png".to_owned(),
        "ws://images.example/avatar.png".to_owned(),
        "file:///tmp/avatar.png".to_owned(),
        "data:image/png;base64,AAAA".to_owned(),
        "/avatar.png".to_owned(),
        "not a url".to_owned(),
        " https://images.example/avatar.png".to_owned(),
        "https://images.example/avatar.png ".to_owned(),
        "https://images.example/avatar\n.png".to_owned(),
        "https:///avatar.png".to_owned(),
        "https://images.example/".to_owned() + &"a".repeat(512),
    ] {
        assert_eq!(
            service
                .update(
                    user_id,
                    UserPatch {
                        nickname: PatchValue::Omitted,
                        avatar_url: PatchValue::Value(invalid),
                    },
                )
                .await,
            Err(UserError::RequestValidation)
        );
    }

    pool.close().await;
    database.dispose().await
}

#[tokio::test]
async fn get_and_patch_me_use_the_shared_production_bearer_extractor() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let (user_id, session_id, service, codec) = profile_fixture(pool.clone()).await?;
    let now = OffsetDateTime::now_utc();
    let token = codec.issue(user_id, session_id, now, now + time::Duration::minutes(15))?;
    let router = user_router(UserHttpState::new(service, codec));

    let unauthorized = router
        .clone()
        .oneshot(Request::get("/api/v1/me").body(Body::empty())?)
        .await?;
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

    let get_response = router
        .clone()
        .oneshot(
            Request::get("/api/v1/me")
                .header(AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(get_response.status(), StatusCode::OK);
    let get_body: Value = serde_json::from_slice(&to_bytes(get_response.into_body(), 4096).await?)?;
    assert_eq!(get_body["id"], user_id.to_string());
    assert_eq!(get_body["provider"], "kakao");

    let patch_response = router
        .clone()
        .oneshot(
            Request::patch("/api/v1/me")
                .header(AUTHORIZATION, format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&json!({
                    "nickname": "HTTP 별명",
                    "avatar_url": ""
                }))?))?,
        )
        .await?;
    assert_eq!(patch_response.status(), StatusCode::OK);
    let patch_body: Value =
        serde_json::from_slice(&to_bytes(patch_response.into_body(), 4096).await?)?;
    assert_eq!(patch_body["nickname"], "HTTP 별명");
    assert!(patch_body["avatar_url"].is_null());

    let invalid_avatar = router
        .oneshot(
            Request::patch("/api/v1/me")
                .header(AUTHORIZATION, format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&json!({
                    "avatar_url": "http://images.example/plain.png"
                }))?))?,
        )
        .await?;
    assert_eq!(invalid_avatar.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let invalid_body: Value =
        serde_json::from_slice(&to_bytes(invalid_avatar.into_body(), 4096).await?)?;
    assert_eq!(invalid_body["error"]["code"], "request_validation_failed");

    pool.close().await;
    database.dispose().await
}

#[test]
fn https_avatar_migration_is_forward_only_and_updates_only_http_urls() -> TestResult {
    let sql = fs::read_to_string(HTTPS_AVATAR_MIGRATION).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            io::Error::other(format!(
                "RED: {HTTPS_AVATAR_MIGRATION} is absent; S4 must normalize stored avatar URLs"
            ))
        } else {
            error
        }
    })?;
    for required in [
        "-- migration: 0013_https_avatar_urls",
        "-- prerequisite: 0012_remove_topic_media.sql",
        "-- reversibility: forward-only",
        "-- recovery: docs/adr/0003-forward-only-sqlx-migrations.md",
        "UPDATE users",
        "SET avatar_url = 'https://' || substring(avatar_url from 8)",
        "WHERE avatar_url LIKE 'http://%'",
    ] {
        assert!(
            sql.contains(required),
            "0013 migration is missing: {required}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn https_avatar_migration_upgrades_0012_predecessor_and_is_idempotent() -> TestResult {
    let database = TestDatabase::migrated_to(12).await?;
    let mut connection = database.connection().await?;
    let http_id = Uuid::new_v4();
    let https_id = Uuid::new_v4();
    let null_id = Uuid::new_v4();
    sqlx::query("INSERT INTO users (id, nickname, avatar_url) VALUES ($1, 'http avatar', $2)")
        .bind(http_id)
        .bind("http://images.example/avatar.png")
        .execute(&mut connection)
        .await?;
    sqlx::query("INSERT INTO users (id, nickname, avatar_url) VALUES ($1, 'https avatar', $2)")
        .bind(https_id)
        .bind("https://images.example/existing.png")
        .execute(&mut connection)
        .await?;
    sqlx::query("INSERT INTO users (id, nickname, avatar_url) VALUES ($1, 'null avatar', NULL)")
        .bind(null_id)
        .execute(&mut connection)
        .await?;

    let migrator = sqlx::migrate::Migrator::new(std::path::Path::new("migrations")).await?;
    migrator.run_to(13, &mut connection).await?;

    assert_eq!(
        avatar_url(&mut connection, http_id).await?.as_deref(),
        Some("https://images.example/avatar.png")
    );
    assert_eq!(
        avatar_url(&mut connection, https_id).await?.as_deref(),
        Some("https://images.example/existing.png")
    );
    assert_eq!(avatar_url(&mut connection, null_id).await?, None);
    let repeated = sqlx::query(
        "UPDATE users \
         SET avatar_url = 'https://' || substring(avatar_url from 8) \
         WHERE avatar_url LIKE 'http://%'",
    )
    .execute(&mut connection)
    .await?;
    assert_eq!(repeated.rows_affected(), 0);

    connection.close().await?;
    database.dispose().await
}

async fn avatar_url(
    connection: &mut sqlx::PgConnection,
    user_id: Uuid,
) -> TestResult<Option<String>> {
    Ok(
        sqlx::query_scalar("SELECT avatar_url FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_one(connection)
            .await?,
    )
}

async fn profile_fixture(
    pool: PgPool,
) -> TestResult<(Uuid, Uuid, Arc<UserService>, Arc<ProductionTokenCodec>)> {
    let repository = Arc::new(PostgresAuthRepository::new(pool.clone()));
    let transactions = Arc::new(SqlxTransactionManager::new(pool));
    let credential = OsCredentialSource.generate()?;
    let session_id = Uuid::new_v4();
    let now = OffsetDateTime::now_utc();
    let mut transaction = transactions.begin().await?;
    let issued = repository
        .create_session(
            transaction.as_mut(),
            &NewProviderIdentity {
                provider: "kakao".to_owned(),
                provider_id: "profile-fixture".to_owned(),
                nickname: "기존 별명".to_owned(),
                avatar_url: Some("https://images.example/original.png".to_owned()),
            },
            &NewRefreshSession {
                id: session_id,
                family_id: Uuid::new_v4(),
                parent_session_id: None,
                token_hash: credential.digest,
                expires_at: now + time::Duration::days(30),
            },
        )
        .await?;
    transactions.commit(transaction).await?;
    let codec = Arc::new(ProductionTokenCodec::new(
        b"task-5-profile-token-secret-32-bytes-minimum",
        "https://api.jamye.test",
        "jamye-mobile",
    )?);
    Ok((
        issued.user_id,
        issued.session_id,
        Arc::new(UserService::new(transactions, repository)),
        codec,
    ))
}

const HOSTED_AVATAR_BASE: &str = "https://avatars.example.test/api/v1/avatars";

/// Seed one hosted-avatar row; an active row also becomes the user's `avatar_url`.
async fn seed_avatar_row(pool: &PgPool, user_id: Uuid, status: &str) -> TestResult<(Uuid, String)> {
    let upload_id = Uuid::new_v4();
    let object_key = format!("avatar/{user_id}/{upload_id}");
    sqlx::query(
        "INSERT INTO user_avatar_uploads \
             (id, user_id, object_key, byte_size, status, expires_at, released_at) \
         VALUES ($1, $2, $3, 2048, $4, \
                 clock_timestamp() + interval '15 minutes', \
                 CASE WHEN $5 THEN clock_timestamp() ELSE NULL END)",
    )
    .bind(upload_id)
    .bind(user_id)
    .bind(&object_key)
    .bind(status)
    .bind(status == "released")
    .execute(pool)
    .await?;
    if status == "active" {
        sqlx::query("UPDATE users SET avatar_url = $2 WHERE id = $1")
            .bind(user_id)
            .bind(format!("{HOSTED_AVATAR_BASE}/{upload_id}"))
            .execute(pool)
            .await?;
    }
    Ok((upload_id, object_key))
}

async fn avatar_row_state(pool: &PgPool, upload_id: Uuid) -> TestResult<(String, bool)> {
    Ok(sqlx::query_as::<_, (String, bool)>(
        "SELECT status, released_at IS NOT NULL FROM user_avatar_uploads WHERE id = $1",
    )
    .bind(upload_id)
    .fetch_one(pool)
    .await?)
}

async fn queued_object_keys(pool: &PgPool) -> TestResult<Vec<String>> {
    Ok(sqlx::query_scalar::<_, String>(
        "SELECT object_key FROM account_object_deletion_intents ORDER BY object_key",
    )
    .fetch_all(pool)
    .await?)
}

#[tokio::test]
async fn clearing_the_avatar_releases_the_active_hosted_row_and_queues_its_object() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let result: TestResult = async {
        let (user_id, _, service, _) = profile_fixture(pool.clone()).await?;
        let (upload_id, object_key) = seed_avatar_row(&pool, user_id, "active").await?;
        let (pending_id, _) = seed_avatar_row(&pool, user_id, "pending").await?;

        let cleared = service
            .update(
                user_id,
                UserPatch {
                    nickname: PatchValue::Omitted,
                    avatar_url: PatchValue::Value(String::new()),
                },
            )
            .await?;
        assert_eq!(cleared.avatar_url, None);
        assert_eq!(
            avatar_row_state(&pool, upload_id).await?,
            ("released".to_owned(), true)
        );
        assert_eq!(queued_object_keys(&pool).await?, vec![object_key.clone()]);
        // An in-flight pending upload is not a hosted avatar and stays usable.
        assert_eq!(
            avatar_row_state(&pool, pending_id).await?,
            ("pending".to_owned(), false)
        );

        // Repeating the clear finds no active row and queues nothing more.
        service
            .update(
                user_id,
                UserPatch {
                    nickname: PatchValue::Omitted,
                    avatar_url: PatchValue::Value(String::new()),
                },
            )
            .await?;
        assert_eq!(queued_object_keys(&pool).await?, vec![object_key]);
        Ok(())
    }
    .await;
    pool.close().await;
    let cleanup = database.dispose().await;
    result?;
    cleanup
}

#[tokio::test]
async fn setting_another_https_avatar_releases_the_active_hosted_row_and_queues_its_object()
-> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let result: TestResult = async {
        let (user_id, _, service, _) = profile_fixture(pool.clone()).await?;
        let (upload_id, object_key) = seed_avatar_row(&pool, user_id, "active").await?;

        let replaced = service
            .update(
                user_id,
                UserPatch {
                    nickname: PatchValue::Omitted,
                    avatar_url: PatchValue::Value("https://images.example/other.png".to_owned()),
                },
            )
            .await?;
        assert_eq!(
            replaced.avatar_url.as_deref(),
            Some("https://images.example/other.png")
        );
        assert_eq!(
            avatar_row_state(&pool, upload_id).await?,
            ("released".to_owned(), true)
        );
        assert_eq!(queued_object_keys(&pool).await?, vec![object_key]);
        Ok(())
    }
    .await;
    pool.close().await;
    let cleanup = database.dispose().await;
    result?;
    cleanup
}

#[tokio::test]
async fn null_or_omitted_avatar_keeps_the_active_hosted_row_and_queues_nothing() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let result: TestResult = async {
        let (user_id, _, service, _) = profile_fixture(pool.clone()).await?;
        let (upload_id, _) = seed_avatar_row(&pool, user_id, "active").await?;
        let hosted_url = format!("{HOSTED_AVATAR_BASE}/{upload_id}");

        for avatar_url in [PatchValue::Null, PatchValue::Omitted] {
            let unchanged = service
                .update(
                    user_id,
                    UserPatch {
                        nickname: PatchValue::Value("바뀐 별명".to_owned()),
                        avatar_url,
                    },
                )
                .await?;
            assert_eq!(unchanged.avatar_url.as_deref(), Some(hosted_url.as_str()));
            assert_eq!(unchanged.nickname, "바뀐 별명");
            assert_eq!(
                avatar_row_state(&pool, upload_id).await?,
                ("active".to_owned(), false)
            );
            assert!(queued_object_keys(&pool).await?.is_empty());
        }

        // An invalid avatar is rejected before any release happens.
        assert_eq!(
            service
                .update(
                    user_id,
                    UserPatch {
                        nickname: PatchValue::Omitted,
                        avatar_url: PatchValue::Value("http://images.example/a.png".to_owned()),
                    },
                )
                .await,
            Err(UserError::RequestValidation)
        );
        assert_eq!(
            avatar_row_state(&pool, upload_id).await?,
            ("active".to_owned(), false)
        );
        assert!(queued_object_keys(&pool).await?.is_empty());
        Ok(())
    }
    .await;
    pool.close().await;
    let cleanup = database.dispose().await;
    result?;
    cleanup
}

#[tokio::test]
async fn clearing_an_external_avatar_queues_nothing() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let result: TestResult = async {
        let (user_id, _, service, _) = profile_fixture(pool.clone()).await?;
        let (released_id, _) = seed_avatar_row(&pool, user_id, "released").await?;
        let cleared = service
            .update(
                user_id,
                UserPatch {
                    nickname: PatchValue::Omitted,
                    avatar_url: PatchValue::Value(String::new()),
                },
            )
            .await?;
        assert_eq!(cleared.avatar_url, None);
        assert!(queued_object_keys(&pool).await?.is_empty());
        assert_eq!(
            avatar_row_state(&pool, released_id).await?,
            ("released".to_owned(), true)
        );
        Ok(())
    }
    .await;
    pool.close().await;
    let cleanup = database.dispose().await;
    result?;
    cleanup
}
