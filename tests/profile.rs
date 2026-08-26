use std::{error::Error, sync::Arc};

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
use sqlx::PgPool;
use time::OffsetDateTime;
use tower::ServiceExt;
use uuid::Uuid;

#[path = "support/postgres.rs"]
mod postgres_support;
use postgres_support::TestDatabase;

type TestResult<T = ()> = Result<T, Box<dyn Error + Send + Sync>>;

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

    pool.close().await;
    database.dispose().await
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
