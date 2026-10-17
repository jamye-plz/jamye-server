use std::sync::Arc;

use jamye_server::application::auth::{AccessTokenVerifier, AuthError};
use tokio::sync::Barrier;

use crate::{
    TestResult,
    auth_helpers::{authorize, exchange, harness},
    postgres_support::TestDatabase,
};

mod refresh_ip {
    use std::{
        net::SocketAddr,
        sync::{
            Arc, Mutex,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
        time::Duration,
    };

    use axum::{
        Router,
        body::{Body, to_bytes},
        extract::ConnectInfo,
        http::{Request, StatusCode, header::RETRY_AFTER},
        response::Response,
    };
    use jamye_server::{
        adapters::postgres::auth::PostgresAuthRepository,
        config::{AppConfig, ConfigInput},
        ports::{
            auth::{
                AuthRepository, AuthRepositoryFuture, CredentialDigest, IssuedSession,
                NewProviderIdentity, NewRefreshSession, NewRotatedSession, ProfilePatch,
                RotationOutcome, UserProfile,
            },
            rate_limit::{
                RateLimitError, RateLimitFuture, RateLimitOutcome, RateLimitRequest, RateLimiter,
            },
            transactions::TransactionHandle,
        },
        transport::http::{auth, client_ip},
    };
    use serde_json::{Value, json};
    use sqlx::PgPool;
    use time::OffsetDateTime;
    use tower::ServiceExt;
    use uuid::Uuid;

    use crate::{
        TestResult,
        auth_helpers::{authorize, exchange, harness_with_repository_and_rate_limiter},
        postgres_support::TestDatabase,
    };

    const TRUSTED: &str = "192.0.2.10";
    const CLIENT: &str = "198.51.100.20";

    #[derive(Clone)]
    struct Call {
        endpoint: &'static str,
        subject: String,
        limit: u32,
        window: Duration,
    }

    #[derive(Default)]
    struct Recorder {
        calls: Mutex<Vec<Call>>,
        ip_checks: AtomicUsize,
        ip_unavailable: AtomicBool,
    }

    impl Recorder {
        fn push(&self, call: Call) {
            self.calls
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(call);
        }

        fn take(&self) -> Vec<Call> {
            std::mem::take(&mut *self.calls.lock().unwrap_or_else(|e| e.into_inner()))
        }
    }

    impl RateLimiter for Recorder {
        fn check<'a>(&'a self, request: &'a RateLimitRequest) -> RateLimitFuture<'a> {
            Box::pin(async move {
                self.push(Call {
                    endpoint: request.endpoint,
                    subject: request.subject.clone(),
                    limit: request.limit,
                    window: request.window,
                });
                if request.endpoint == "auth_refresh_ip" {
                    if self.ip_unavailable.load(Ordering::SeqCst) {
                        return Err(RateLimitError);
                    }
                    if self.ip_checks.fetch_add(1, Ordering::SeqCst) >= request.limit as usize {
                        return Ok(RateLimitOutcome::Denied {
                            retry_after: Duration::from_secs(7),
                        });
                    }
                }
                Ok(RateLimitOutcome::Allowed)
            })
        }
    }

    struct RecordingRepository {
        inner: PostgresAuthRepository,
        recorder: Arc<Recorder>,
    }

    impl AuthRepository for RecordingRepository {
        fn create_session<'a>(
            &'a self,
            transaction: &'a mut dyn TransactionHandle,
            identity: &'a NewProviderIdentity,
            session: &'a NewRefreshSession,
        ) -> AuthRepositoryFuture<'a, IssuedSession> {
            self.inner.create_session(transaction, identity, session)
        }

        fn restore_deleted_identity<'a>(
            &'a self,
            transaction: &'a mut dyn TransactionHandle,
            identity: &'a NewProviderIdentity,
            session: &'a NewRefreshSession,
        ) -> AuthRepositoryFuture<'a, Option<IssuedSession>> {
            self.inner
                .restore_deleted_identity(transaction, identity, session)
        }

        fn rotate_session<'a>(
            &'a self,
            transaction: &'a mut dyn TransactionHandle,
            token_hash: &'a CredentialDigest,
            child: &'a NewRotatedSession,
            now: OffsetDateTime,
        ) -> AuthRepositoryFuture<'a, RotationOutcome> {
            self.recorder.push(Call {
                endpoint: "rotate_session",
                subject: String::new(),
                limit: 0,
                window: Duration::ZERO,
            });
            self.inner
                .rotate_session(transaction, token_hash, child, now)
        }

        fn revoke_session<'a>(
            &'a self,
            transaction: &'a mut dyn TransactionHandle,
            session_id: Uuid,
            now: OffsetDateTime,
        ) -> AuthRepositoryFuture<'a, ()> {
            self.inner.revoke_session(transaction, session_id, now)
        }

        fn account_is_active(&self, user_id: Uuid) -> AuthRepositoryFuture<'_, bool> {
            self.inner.account_is_active(user_id)
        }

        fn profile(&self, user_id: Uuid) -> AuthRepositoryFuture<'_, Option<UserProfile>> {
            self.inner.profile(user_id)
        }

        fn update_profile<'a>(
            &'a self,
            transaction: &'a mut dyn TransactionHandle,
            user_id: Uuid,
            patch: &'a ProfilePatch,
        ) -> AuthRepositoryFuture<'a, Option<UserProfile>> {
            self.inner.update_profile(transaction, user_id, patch)
        }
    }

    async fn fixture(pool: PgPool) -> TestResult<(auth::AuthHttpState, Arc<Recorder>, String)> {
        let recorder = Arc::new(Recorder::default());
        let repository = Arc::new(RecordingRepository {
            inner: PostgresAuthRepository::new(pool.clone()),
            recorder: recorder.clone(),
        });
        let fixture = harness_with_repository_and_rate_limiter(pool, repository, recorder.clone())?;
        let pair = exchange(&fixture.service, authorize(&fixture.service).await?).await?;
        recorder.take();
        Ok((
            auth::AuthHttpState::new(fixture.service, fixture.codec),
            recorder,
            pair.refresh_token,
        ))
    }

    fn router(state: auth::AuthHttpState, trusted: Option<&str>) -> TestResult<Router> {
        let config = AppConfig::try_from(ConfigInput {
            environment: Some("test".to_owned()),
            database_url: Some("postgres://localhost/jamye_test".to_owned()),
            trusted_proxies: trusted.map(ToOwned::to_owned),
            ..ConfigInput::default()
        })?;
        Ok(client_ip::with_trusted_proxies(
            auth::router(state),
            &config,
        ))
    }

    async fn refresh(
        router: &Router,
        token: &str,
        peer: &str,
        headers: &[&str],
    ) -> TestResult<Response> {
        let mut builder =
            Request::post("/api/v1/auth/refresh").header("content-type", "application/json");
        for header in headers {
            builder = builder.header("x-forwarded-for", *header);
        }
        let mut request = builder.body(Body::from(json!({"refresh_token": token}).to_string()))?;
        request
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::new(peer.parse()?, 4000)));
        Ok(router.clone().oneshot(request).await?)
    }

    async fn body(response: Response) -> TestResult<Value> {
        Ok(serde_json::from_slice(
            &to_bytes(response.into_body(), 16 * 1024).await?,
        )?)
    }

    fn assert_calls(calls: &[Call], expected: &[&str]) {
        let actual = calls.iter().map(|call| call.endpoint).collect::<Vec<_>>();
        assert_eq!(actual.as_slice(), expected);
    }

    #[tokio::test]
    async fn verified_ip_budget_blocks_rotating_tokens_before_repository() -> TestResult {
        let database = TestDatabase::migrated().await?;
        let pool = database.pool()?;
        let (state, recorder, mut token) = fixture(pool.clone()).await?;
        let router = router(state, Some(TRUSTED))?;

        let invalid = refresh(&router, "invalid", TRUSTED, &[CLIENT]).await?;
        assert_eq!(invalid.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert!(recorder.take().is_empty());

        for _ in 0..10 {
            let response = refresh(&router, &token, TRUSTED, &[CLIENT]).await?;
            assert_eq!(response.status(), StatusCode::OK);
            let calls = recorder.take();
            assert_calls(
                &calls,
                &["auth_refresh_ip", "auth_refresh", "rotate_session"],
            );
            assert_eq!(calls[0].subject, "ip:198.51.100.20");
            assert_eq!(
                (calls[0].limit, calls[0].window),
                (10, Duration::from_secs(60))
            );
            assert_eq!(
                (calls[1].limit, calls[1].window),
                (10, Duration::from_secs(60))
            );
            assert!(calls[1].subject == token);
            let next = body(response).await?["refresh_token"]
                .as_str()
                .ok_or("missing refresh token")?
                .to_owned();
            assert!(next != token);
            token = next;
        }

        let other = if token == "x".repeat(43) {
            "y".repeat(43)
        } else {
            "x".repeat(43)
        };
        for candidate in [token, other] {
            let response = refresh(&router, &candidate, TRUSTED, &[CLIENT]).await?;
            assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
            assert_eq!(
                response
                    .headers()
                    .get(RETRY_AFTER)
                    .and_then(|v| v.to_str().ok()),
                Some("7")
            );
            assert_eq!(
                body(response).await?["error"]["code"],
                "rate_limit_exceeded"
            );
            let calls = recorder.take();
            assert_calls(&calls, &["auth_refresh_ip"]);
            assert_eq!(calls[0].subject, "ip:198.51.100.20");
        }
        pool.close().await;
        database.dispose().await
    }

    #[tokio::test]
    async fn unverified_ip_skips_ip_limit_and_preserves_token_then_repository() -> TestResult {
        let database = TestDatabase::migrated().await?;
        let pool = database.pool()?;
        let (state, recorder, mut token) = fixture(pool.clone()).await?;
        for (trusted, peer, headers) in [
            (Some(TRUSTED), TRUSTED, vec![]),
            (Some(TRUSTED), TRUSTED, vec!["malformed"]),
            (Some(TRUSTED), TRUSTED, vec![CLIENT, CLIENT]),
            (Some(TRUSTED), "192.0.2.11", vec![CLIENT]),
            (None, TRUSTED, vec![CLIENT]),
        ] {
            let router = router(state.clone(), trusted)?;
            let response = refresh(&router, &token, peer, &headers).await?;
            assert_eq!(response.status(), StatusCode::OK);
            let calls = recorder.take();
            assert_calls(&calls, &["auth_refresh", "rotate_session"]);
            assert!(calls[0].subject == token);
            assert!(!calls.iter().any(|call| call.subject.starts_with("ip:")));
            token = body(response).await?["refresh_token"]
                .as_str()
                .ok_or("missing refresh token")?
                .to_owned();
        }
        pool.close().await;
        database.dispose().await
    }

    #[tokio::test]
    async fn verified_ip_limiter_unavailable_preserves_503_before_repository() -> TestResult {
        let database = TestDatabase::migrated().await?;
        let pool = database.pool()?;
        let (state, recorder, token) = fixture(pool.clone()).await?;
        recorder.ip_unavailable.store(true, Ordering::SeqCst);
        let response = refresh(&router(state, Some(TRUSTED))?, &token, TRUSTED, &[CLIENT]).await?;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            body(response).await?["error"]["code"],
            "rate_limit_unavailable"
        );
        let calls = recorder.take();
        assert_calls(&calls, &["auth_refresh_ip"]);
        assert_eq!(calls[0].subject, "ip:198.51.100.20");
        pool.close().await;
        database.dispose().await
    }
}

#[tokio::test]
async fn concurrent_callbacks_converge_on_one_identity_and_user() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let fixture = harness(pool.clone(), Some(Arc::new(Barrier::new(2))))?;
    let first_state = authorize(&fixture.service).await?;
    let second_state = authorize(&fixture.service).await?;

    let (first, second) = tokio::join!(
        exchange(&fixture.service, first_state),
        exchange(&fixture.service, second_state)
    );
    assert!(first.is_ok());
    assert!(second.is_ok());
    assert_eq!(fixture.attempts.len()?, 0);

    let user_count: i64 = sqlx::query_scalar("SELECT count(*) FROM users")
        .fetch_one(&pool)
        .await?;
    let identity_count: i64 = sqlx::query_scalar("SELECT count(*) FROM auth_identities")
        .fetch_one(&pool)
        .await?;
    let session_count: i64 = sqlx::query_scalar("SELECT count(*) FROM refresh_sessions")
        .fetch_one(&pool)
        .await?;
    assert_eq!(user_count, 1);
    assert_eq!(identity_count, 1);
    assert_eq!(session_count, 2);

    pool.close().await;
    database.dispose().await
}

#[tokio::test]
async fn concurrent_refresh_allows_one_child_then_reuse_revokes_the_family() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let fixture = harness(pool.clone(), None)?;
    let state = authorize(&fixture.service).await?;
    let initial = exchange(&fixture.service, state).await?;

    let (first, second) = tokio::join!(
        fixture.service.refresh(&initial.refresh_token),
        fixture.service.refresh(&initial.refresh_token)
    );
    let child = match (first, second) {
        (Ok(child), Err(AuthError::RefreshTokenReused))
        | (Err(AuthError::RefreshTokenReused), Ok(child)) => child,
        outcomes => return Err(format!("unexpected refresh race outcomes: {outcomes:?}").into()),
    };
    assert_eq!(
        fixture.service.refresh(&child.refresh_token).await,
        Err(AuthError::RefreshTokenInvalid)
    );

    let row = sqlx::query_as::<_, (i64, i64, i64)>(
        "SELECT count(*), \
                count(*) FILTER (WHERE parent_session_id IS NOT NULL), \
                count(*) FILTER (WHERE revoked_at IS NOT NULL) \
         FROM refresh_sessions",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(row, (2, 1, 2));

    pool.close().await;
    database.dispose().await
}

#[tokio::test]
async fn failed_child_insert_rolls_back_parent_consumption_and_allows_retry() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let fixture = harness(pool.clone(), None)?;
    let initial = exchange(&fixture.service, authorize(&fixture.service).await?).await?;

    sqlx::query(
        "ALTER TABLE refresh_sessions \
         ADD CONSTRAINT task_5_force_child_failure CHECK (parent_session_id IS NULL)",
    )
    .execute(&pool)
    .await?;
    assert_eq!(
        fixture.service.refresh(&initial.refresh_token).await,
        Err(AuthError::DatabaseUnavailable)
    );
    let parent = sqlx::query_as::<_, (bool, bool)>(
        "SELECT consumed_at IS NOT NULL, revoked_at IS NOT NULL \
         FROM refresh_sessions WHERE parent_session_id IS NULL",
    )
    .fetch_one(&pool)
    .await?;
    let session_count: i64 = sqlx::query_scalar("SELECT count(*) FROM refresh_sessions")
        .fetch_one(&pool)
        .await?;
    assert_eq!(parent, (false, false));
    assert_eq!(session_count, 1);

    sqlx::query("ALTER TABLE refresh_sessions DROP CONSTRAINT task_5_force_child_failure")
        .execute(&pool)
        .await?;
    assert!(
        fixture
            .service
            .refresh(&initial.refresh_token)
            .await
            .is_ok()
    );

    pool.close().await;
    database.dispose().await
}

#[tokio::test]
async fn logout_revokes_only_refresh_authority_and_access_remains_valid_to_expiry() -> TestResult {
    let database = TestDatabase::migrated().await?;
    let pool = database.pool()?;
    let fixture = harness(pool.clone(), None)?;
    let first = exchange(&fixture.service, authorize(&fixture.service).await?).await?;
    let second = exchange(&fixture.service, authorize(&fixture.service).await?).await?;
    let first_identity = fixture.codec.verify(&first.access_token)?;
    let second_identity = fixture.codec.verify(&second.access_token)?;
    assert_eq!(first_identity.user_id, second_identity.user_id);
    assert_ne!(first_identity.session_id, second_identity.session_id);

    fixture
        .service
        .logout(first_identity.session_id, "user:fixture")
        .await?;
    fixture
        .service
        .logout(first_identity.session_id, "user:fixture")
        .await?;
    assert_eq!(
        fixture.service.refresh(&first.refresh_token).await,
        Err(AuthError::RefreshTokenInvalid)
    );
    assert!(fixture.codec.verify(&first.access_token).is_ok());
    assert!(fixture.service.refresh(&second.refresh_token).await.is_ok());

    let revoked: Vec<(uuid::Uuid, bool)> = sqlx::query_as(
        "SELECT id, revoked_at IS NOT NULL FROM refresh_sessions \
         WHERE id IN ($1, $2) ORDER BY id",
    )
    .bind(first_identity.session_id)
    .bind(second_identity.session_id)
    .fetch_all(&pool)
    .await?;
    assert_eq!(revoked.len(), 2);
    assert!(
        revoked
            .iter()
            .any(|(id, is_revoked)| *id == first_identity.session_id && *is_revoked)
    );
    assert!(
        revoked
            .iter()
            .any(|(id, is_revoked)| *id == second_identity.session_id && !*is_revoked)
    );

    pool.close().await;
    database.dispose().await
}
