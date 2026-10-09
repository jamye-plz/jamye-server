//! Explicit M0 API composition root.

use std::{error::Error, fmt, future::Future, iter::once, sync::Arc};

use axum::{
    Router,
    body::Body,
    extract::{MatchedPath, Request},
    http::{HeaderName, header::AUTHORIZATION},
    middleware,
};
use tower::ServiceBuilder;
use tower_http::{
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, RequestId, SetRequestIdLayer},
    sensitive_headers::SetSensitiveRequestHeadersLayer,
    trace::{DefaultOnResponse, TraceLayer},
};
use tracing::{Level, Span};

use crate::transport::push::composition as push_composition;
use crate::transport::realtime::composition as realtime_composition;
use crate::{
    adapters::{
        object_storage::account_deletion::S3AccountObjectDeletionProvider,
        object_storage::health::MinioHealthProbe, postgres::health::PostgresHealthProbe,
        redis::health::RedisHealthProbe,
    },
    config::{
        AppConfig, account_deletion::AccountDeletionConfig, app_links::AppLinksConfig,
        auth::AuthConfig, object_storage::ObjectStorageConfig, push::PushConfig,
    },
    platform::{
        readiness::{DependencyProbe, ReadinessService, UnconfiguredProbe},
        request_id::{REQUEST_ID_HEADER, strip_external_request_id},
    },
    transport::http::health::{self, HealthState},
};

use crate::{
    adapters::{
        oauth::{
            AppleOAuthConfig, AppleOAuthProvider, GoogleOAuthProvider, KakaoOAuthProvider,
            OAuthClientConfig, OsCredentialSource, ProductionTokenCodec,
        },
        object_storage::media::S3MediaObjectStorage,
        postgres::{
            account_deletion::PostgresAccountDeletionRepository, auth::PostgresAuthRepository,
            chatrooms::PostgresChatroomsRepository, groups::PostgresGroupsRepository,
            media::PostgresMediaRepository, messaging::PostgresMessagingRepository,
            notifications::PostgresNotificationsRepository, push::PostgresPushRepository,
            realtime::PostgresRealtimeRepository,
            realtime_revocations::PostgresRealtimeRevocations, runtime_pool,
            topics::PostgresTopicsRepository, transactions::SqlxTransactionManager,
        },
        redis::{
            oauth_attempt::RedisOAuthAttemptStore,
            rate_limit::RedisRateLimiter,
            realtime::{OsTicketCredentialSource, RedisRealtimeAdapter},
            realtime_control::RedisRealtimeControl,
        },
    },
    application::users::UserService,
    application::{
        account_deletion::{AccountDeletionDependencies, AccountDeletionService},
        auth::{
            AppleIdentityProviderSlot, AuthDependencies, AuthLifetimePolicy, AuthService,
            OAuthProviderSlot, SystemAuthClock,
        },
        chatrooms::ChatroomsService,
        groups::{GroupsDependencies, GroupsService, SystemGroupsClock},
        media::{
            MediaAccessDependencies, MediaAccessService, MediaDependencies,
            MediaFinalizeDependencies, MediaFinalizeService, MediaService,
        },
        messaging::MessagingService,
        notifications::{NotificationsDependencies, NotificationsService},
        push::{PushDependencies, PushService},
        realtime::{
            RealtimeTicketService, SystemClock, membership_revocation::MembershipRevocationService,
        },
        topics::{TopicsDependencies, TopicsService},
        transactions::{TransactionCompositionDependencies, TransactionCompositions},
    },
    ports::{oauth_provider::ProviderKind, realtime::RealtimePortError},
    transport::{
        http::{
            account_deletion::{AccountDeletionHttpState, router as account_deletion_router},
            app_links::{AppLinksHttpState, router as app_links_router},
            auth::{AuthHttpState, router as auth_router},
            chatrooms::{ChatroomsHttpState, router as chatrooms_router},
            groups::{GroupsHttpState, router as groups_router},
            legal::{LegalHttpState, LegalRenderError, router as legal_router},
            media::{
                MediaHttpState, MediaMutationHttpState, mutation_router as media_mutation_router,
                router as media_router,
            },
            messaging::{MessagingHttpState, router as messaging_router},
            notifications::{NotificationsHttpState, router as notifications_router},
            push::{PushHttpState, router as push_router},
            realtime::{RealtimeHttpState, router as realtime_router},
            topics::{TopicsHttpState, router as topics_router},
            users::{UserHttpState, router as user_router},
        },
        realtime::{
            LocalRealtimeHub,
            authorization::{AuthorizedRealtimeDelivery, RealtimeControlConsumer},
        },
    },
};

/// Builds the exact API root invoked by `src/bin/api.rs`.
///
/// `auth` is deliberately an explicit validated input. The compatibility
/// factory is retained for callers that only need the validated auth boundary;
/// production startup supplies the complete runtime inputs below.
pub fn router(config: &AppConfig, auth: &AuthConfig) -> Result<Router, CompositionError> {
    let rate_limits = crate::config::rate_limit::RateLimitConfig::default();
    let app_links = AppLinksConfig::from_env().map_err(|_| CompositionError::AppLinks)?;
    let object_storage = ObjectStorageConfig::from_env(config.environment())
        .map_err(|_| CompositionError::ObjectStorageNotConfigured)?;
    router_with_runtime(
        config,
        &app_links,
        auth,
        &rate_limits,
        object_storage.as_ref(),
    )
}

/// Builds the production router from already-validated feature configuration.
/// It deliberately performs no object-store I/O; API startup owns bucket ensure.
pub fn router_with_runtime(
    config: &AppConfig,
    app_links: &AppLinksConfig,
    auth: &AuthConfig,
    rate_limits: &crate::config::rate_limit::RateLimitConfig,
    object_storage: Option<&ObjectStorageConfig>,
) -> Result<Router, CompositionError> {
    let timeout = config.readiness_timeout();
    // Legal pages render once at startup from validated operator values; no values means the
    // four routes stay unmounted (plain 404) so the code can deploy before the values exist.
    let legal_routes = config
        .legal()
        .map(LegalHttpState::new)
        .transpose()
        .map_err(CompositionError::Legal)?;
    let postgres: Arc<dyn DependencyProbe> = Arc::new(
        PostgresHealthProbe::connect_lazy(config.database_url(), timeout)
            .map_err(|_| CompositionError::Postgres)?,
    );
    let redis = redis_probe(config, timeout)?;
    let minio = minio_probe(config, timeout)?;
    let readiness = ReadinessService::new(postgres, redis, minio);
    let object_storage = object_storage.ok_or(CompositionError::ObjectStorageNotConfigured)?;
    let pool = runtime_pool(config.database_url(), config.readiness_timeout())
        .map_err(|_| CompositionError::Postgres)?;
    let redis_url = config
        .redis_url()
        .ok_or(CompositionError::RedisNotConfigured)?;
    let verifier = Arc::new(
        ProductionTokenCodec::new(
            auth.access_token_secret.expose_secret().as_bytes(),
            &auth.access_token_issuer,
            &auth.access_token_audience,
        )
        .map_err(|_| CompositionError::Auth)?,
    );
    let transactions = Arc::new(SqlxTransactionManager::new(pool.clone()));
    let auth_repository = Arc::new(PostgresAuthRepository::new(pool.clone()));
    let groups_repository = Arc::new(PostgresGroupsRepository::new(pool.clone()));
    let chatrooms_repository = Arc::new(PostgresChatroomsRepository::new(pool.clone()));
    let topics_repository = Arc::new(PostgresTopicsRepository::new(pool.clone()));
    let messaging_repository = Arc::new(PostgresMessagingRepository::new(pool.clone()));
    let media_repository = Arc::new(PostgresMediaRepository::new(pool.clone()));
    let notifications_repository = Arc::new(PostgresNotificationsRepository::new(pool.clone()));
    let push_repository = Arc::new(PostgresPushRepository::new(pool.clone()));
    let realtime_revocations = PostgresRealtimeRevocations::new(pool.clone());
    let realtime_revocation_appender = Arc::new(realtime_revocations.clone());
    let rate_limiter =
        Arc::new(RedisRateLimiter::new(redis_url).map_err(|_| CompositionError::Redis)?);
    let attempts =
        Arc::new(RedisOAuthAttemptStore::new(redis_url).map_err(|_| CompositionError::Redis)?);
    let apple_provider = apple_provider(auth)?;
    let apple_identity_slot = apple_provider
        .as_ref()
        .map(|provider| {
            let provider: Arc<dyn crate::ports::apple_identity_provider::AppleIdentityProvider> =
                provider.clone();
            AppleIdentityProviderSlot::enabled(provider)
        })
        .unwrap_or_else(AppleIdentityProviderSlot::disabled);
    let auth_service = Arc::new(
        AuthService::new(
            AuthDependencies {
                transactions: transactions.clone(),
                repository: auth_repository.clone(),
                attempts,
                rate_limiter: rate_limiter.clone(),
                credentials: Arc::new(OsCredentialSource),
                token_issuer: verifier.clone(),
                clock: Arc::new(SystemAuthClock),
            },
            oauth_slot(&auth.kakao, auth.provider_timeout)?,
            oauth_slot(&auth.google, auth.provider_timeout)?,
            apple_identity_slot,
            AuthLifetimePolicy {
                access: auth.access_token_ttl,
                refresh: auth.refresh_token_ttl,
            },
            rate_limits.auth.clone(),
        )
        .map_err(|_| CompositionError::Auth)?,
    );
    let groups_service = Arc::new(
        GroupsService::new(
            GroupsDependencies {
                transactions: transactions.clone(),
                repository: groups_repository.clone(),
                rate_limiter: rate_limiter.clone(),
                credentials: Arc::new(OsCredentialSource),
                clock: Arc::new(SystemGroupsClock),
            },
            rate_limits.groups.clone(),
        )
        .map_err(|_| CompositionError::Groups)?,
    );
    let messaging_service = Arc::new(MessagingService::new(
        transactions.clone(),
        messaging_repository.clone(),
    ));
    let topics_service = Arc::new(TopicsService::new(TopicsDependencies {
        transactions: transactions.clone(),
        repository: topics_repository.clone(),
    }));
    let chatrooms_service = Arc::new(ChatroomsService::new(
        transactions.clone(),
        chatrooms_repository.clone(),
    ));
    let compositions = Arc::new(TransactionCompositions::new(
        TransactionCompositionDependencies {
            transactions: transactions.clone(),
            messaging: messaging_service.clone(),
            media: media_repository.clone(),
            topics: topics_service.clone(),
            chatrooms: chatrooms_service.clone(),
            notifications: notifications_repository.clone(),
        },
    ));
    let storage = Arc::new(S3MediaObjectStorage::new(object_storage));
    let media_service = Arc::new(
        MediaService::new(
            MediaDependencies {
                transactions: transactions.clone(),
                repository: media_repository.clone(),
                object_storage: storage.clone(),
                rate_limiter: rate_limiter.clone(),
            },
            rate_limits.media_upload_presign,
        )
        .map_err(|_| CompositionError::Media)?,
    );
    let media_finalize = Arc::new(MediaFinalizeService::new(MediaFinalizeDependencies {
        transactions: transactions.clone(),
        repository: media_repository.clone(),
        object_storage: storage.clone(),
    }));
    let media_access = Arc::new(MediaAccessService::new(MediaAccessDependencies {
        repository: media_repository.clone(),
        object_storage: storage.clone(),
    }));
    // Avatar hosting is gated by JAMYE_AVATAR_PUBLIC_BASE_URL: without it the three avatar
    // routes are not mounted at all (plain 404), so deploys can ship the code dark.
    let avatar_routes = config
        .avatar_public_base_url()
        .map(|public_base_url| {
            let service = crate::application::avatar::AvatarService::new(
                crate::application::avatar::AvatarDependencies {
                    transactions: transactions.clone(),
                    repository: Arc::new(
                        crate::adapters::postgres::avatar::PostgresAvatarRepository::new(
                            pool.clone(),
                        ),
                    ),
                    object_storage: storage.clone(),
                    rate_limiter: rate_limiter.clone(),
                },
                crate::application::avatar::AvatarSettings {
                    public_base_url: public_base_url.to_owned(),
                    upload_rate_limit: rate_limits.media_upload_presign,
                    public_read_rate_limit: rate_limits.avatar_public_read,
                },
            )
            .map_err(|_| CompositionError::Media)?;
            Ok::<_, CompositionError>(crate::transport::http::avatar::router(
                crate::transport::http::avatar::AvatarHttpState::new(
                    Arc::new(service),
                    verifier.clone(),
                ),
            ))
        })
        .transpose()?;
    let notifications_service = Arc::new(NotificationsService::new(NotificationsDependencies {
        transactions: transactions.clone(),
        repository: notifications_repository.clone(),
    }));
    let push_service = Arc::new(PushService::new(PushDependencies {
        transactions: transactions.clone(),
        repository: push_repository.clone(),
    }));
    let users = Arc::new(UserService::new(transactions.clone(), auth_repository));
    let membership_revocations = Arc::new(MembershipRevocationService::new(
        groups_service.clone(),
        transactions.clone(),
        realtime_revocation_appender,
        push_repository.clone(),
    ));
    let deletion = Arc::new(AccountDeletionService::new(AccountDeletionDependencies {
        transactions: transactions.clone(),
        groups: groups_service.clone(),
        push_privacy_fence: push_repository.clone(),
        repository: Arc::new(PostgresAccountDeletionRepository::new(pool.clone())),
        apple_identity_provider: apple_provider.as_ref().map(|provider| {
            let provider: Arc<dyn crate::ports::apple_identity_provider::AppleIdentityProvider> =
                provider.clone();
            provider
        }),
        apple_revocation_provider: apple_provider.as_ref().map(|provider| {
            let provider: Arc<dyn crate::ports::apple_identity_provider::AppleRevocationProvider> =
                provider.clone();
            provider
        }),
    }));
    let redis =
        Arc::new(RedisRealtimeAdapter::new(redis_url).map_err(|_| CompositionError::Redis)?);
    let redis_control =
        Arc::new(RedisRealtimeControl::new(redis_url).map_err(|_| CompositionError::Redis)?);
    let hub = LocalRealtimeHub::default();
    let realtime_delivery =
        AuthorizedRealtimeDelivery::new(hub.clone(), realtime_revocations.clone());
    spawn_redis_forwarder(redis.clone(), realtime_delivery);
    spawn_realtime_control_consumer(
        redis_control,
        RealtimeControlConsumer::new(hub.clone(), realtime_revocations.clone()),
    );
    let tickets = Arc::new(RealtimeTicketService::new(
        redis,
        Arc::new(OsTicketCredentialSource),
        Arc::new(SystemClock),
    ));
    let realtime_repository = Arc::new(PostgresRealtimeRepository::new(pool));
    let mut application = health::router(HealthState::new(readiness))
        .merge(app_links_router(AppLinksHttpState::new(app_links.clone())))
        .merge(auth_router(AuthHttpState::new(
            auth_service,
            verifier.clone(),
        )))
        .merge(user_router(UserHttpState::new(
            users.clone(),
            verifier.clone(),
        )))
        .merge(account_deletion_router(AccountDeletionHttpState::new(
            deletion,
            verifier.clone(),
        )))
        .merge(groups_router(
            GroupsHttpState::new(groups_service, verifier.clone())
                .with_membership_revocations(membership_revocations),
        ))
        .merge(chatrooms_router(
            ChatroomsHttpState::new(chatrooms_service, verifier.clone())
                .with_compositions(compositions.clone()),
        ))
        .merge(topics_router(
            TopicsHttpState::new(topics_service, verifier.clone())
                .with_compositions(compositions.clone()),
        ))
        .merge(messaging_router(
            MessagingHttpState::new(
                messaging_service,
                crate::transport::http::auth::AuthVerifierState::new(verifier.clone()),
            )
            .with_compositions(compositions),
        ))
        .merge(media_mutation_router(MediaMutationHttpState::new(
            media_service,
            media_finalize,
            verifier.clone(),
        )))
        .merge(media_router(MediaHttpState::new(
            media_access,
            verifier.clone(),
        )))
        .merge(notifications_router(NotificationsHttpState::new(
            notifications_service,
            verifier.clone(),
        )))
        .merge(push_router(PushHttpState::new(
            push_service,
            verifier.clone(),
        )))
        .merge(realtime_router(RealtimeHttpState::new(
            tickets,
            hub,
            realtime_repository,
            crate::transport::http::auth::AuthVerifierState::new(verifier),
            users,
        )));
    if let Some(avatar_routes) = avatar_routes {
        application = application.merge(avatar_routes);
    }
    if let Some(legal_state) = legal_routes {
        application = application.merge(legal_router(legal_state));
    }
    Ok(with_platform_layers(application))
}

fn oauth_slot(
    config: &crate::config::auth::ProviderConfig,
    timeout: std::time::Duration,
) -> Result<OAuthProviderSlot, CompositionError> {
    if !config.enabled {
        return Ok(OAuthProviderSlot::disabled(config.kind));
    }
    let client = OAuthClientConfig::new(
        config
            .client_id
            .as_ref()
            .ok_or(CompositionError::Auth)?
            .expose_secret(),
        config
            .client_secret
            .as_ref()
            .ok_or(CompositionError::Auth)?
            .expose_secret(),
        timeout,
    )
    .map_err(|_| CompositionError::Auth)?;
    match config.kind {
        ProviderKind::Kakao => OAuthProviderSlot::enabled(
            config.kind,
            config.redirect_uris.clone(),
            Arc::new(KakaoOAuthProvider::new(client).map_err(|_| CompositionError::Auth)?),
        )
        .map_err(|_| CompositionError::Auth),
        ProviderKind::Google => OAuthProviderSlot::enabled(
            config.kind,
            config.redirect_uris.clone(),
            Arc::new(GoogleOAuthProvider::new(client).map_err(|_| CompositionError::Auth)?),
        )
        .map_err(|_| CompositionError::Auth),
        ProviderKind::Apple => Err(CompositionError::Auth),
    }
}

fn apple_provider(auth: &AuthConfig) -> Result<Option<Arc<AppleOAuthProvider>>, CompositionError> {
    if !auth.apple.enabled {
        return Ok(None);
    }
    let config = AppleOAuthConfig::new(
        auth.apple.audiences.clone(),
        auth.apple
            .team_id
            .as_ref()
            .ok_or(CompositionError::Auth)?
            .expose_secret(),
        auth.apple
            .key_id
            .as_ref()
            .ok_or(CompositionError::Auth)?
            .expose_secret(),
        auth.apple
            .private_key_der
            .as_ref()
            .ok_or(CompositionError::Auth)?
            .expose_secret()
            .to_vec(),
        auth.provider_timeout,
    )
    .map_err(|_| CompositionError::Auth)?;
    Ok(Some(Arc::new(
        AppleOAuthProvider::new(config).map_err(|_| CompositionError::Auth)?,
    )))
}

/// Builds the exact fixed worker root invoked by `src/bin/worker.rs`.
///
/// This fixed root owns the existing realtime, Task-9 push, and Task-11
/// cleanup runners without a registry or duplicate feature lifecycle.
pub fn worker(
    config: &AppConfig,
    push: &PushConfig,
    object_storage: &ObjectStorageConfig,
    cleanup: &AccountDeletionConfig,
) -> Result<ProductionWorkerRuntime, WorkerCompositionError> {
    let realtime =
        realtime_composition::worker(config).map_err(WorkerCompositionError::Realtime)?;
    let push = push_composition::worker(config, push).map_err(WorkerCompositionError::Push)?;
    let (cleanup, purge) = account_cleanup_workers(config, object_storage, cleanup)?;
    Ok(ProductionWorkerRuntime {
        realtime,
        push,
        cleanup,
        purge,
    })
}

pub struct ProductionWorkerRuntime {
    realtime: realtime_composition::WorkerRuntime,
    push: push_composition::WorkerRuntime,
    cleanup: crate::application::account_deletion::cleanup::AccountObjectDeletionWorker,
    purge: crate::application::account_deletion::cleanup::AccountPurgeWorker,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkerRuntimeError {
    RunnerCompleted,
    RunnerPanicked,
}

impl fmt::Display for WorkerRuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a production worker runner stopped unexpectedly")
    }
}

impl Error for WorkerRuntimeError {}

impl ProductionWorkerRuntime {
    pub async fn run_until<F>(self, shutdown: F) -> Result<(), WorkerRuntimeError>
    where
        F: Future<Output = ()> + Send + 'static,
    {
        let ProductionWorkerRuntime {
            realtime,
            push,
            cleanup,
            purge,
        } = self;
        let (stop, receiver) = tokio::sync::watch::channel(());
        let realtime_stop = receiver.clone();
        let push_stop = receiver.clone();
        let cleanup_stop = receiver.clone();
        let purge_stop = receiver;
        let mut runners = tokio::task::JoinSet::new();
        runners.spawn(async move {
            realtime
                .run_until(async move {
                    let mut receiver = realtime_stop;
                    let _ = receiver.changed().await;
                })
                .await;
        });
        runners.spawn(async move {
            push.run_until(async move {
                let mut receiver = push_stop;
                let _ = receiver.changed().await;
            })
            .await;
        });
        runners.spawn(async move {
            run_cleanup_until(cleanup, async move {
                let mut receiver = cleanup_stop;
                let _ = receiver.changed().await;
            })
            .await;
        });
        runners.spawn(async move {
            run_purge_until(purge, async move {
                let mut receiver = purge_stop;
                let _ = receiver.changed().await;
            })
            .await;
        });
        supervise_runners(shutdown, &mut runners, stop).await
    }
}

/// Supervises all fixed worker runners. An early return or a panic is a
/// process failure: it stops siblings and joins every task before returning.
async fn supervise_runners<F>(
    shutdown: F,
    runners: &mut tokio::task::JoinSet<()>,
    stop: tokio::sync::watch::Sender<()>,
) -> Result<(), WorkerRuntimeError>
where
    F: Future<Output = ()> + Send,
{
    tokio::pin!(shutdown);
    let mut failure = tokio::select! {
        () = &mut shutdown => None,
        result = runners.join_next() => Some(match result {
            Some(Ok(())) | None => WorkerRuntimeError::RunnerCompleted,
            Some(Err(_)) => WorkerRuntimeError::RunnerPanicked,
        }),
    };
    let _ = stop.send(());
    while let Some(result) = runners.join_next().await {
        if result.is_err() {
            failure = Some(WorkerRuntimeError::RunnerPanicked);
        }
    }
    failure.map_or(Ok(()), Err)
}

fn account_cleanup_workers(
    config: &AppConfig,
    object_storage: &ObjectStorageConfig,
    cleanup: &AccountDeletionConfig,
) -> Result<
    (
        crate::application::account_deletion::cleanup::AccountObjectDeletionWorker,
        crate::application::account_deletion::cleanup::AccountPurgeWorker,
    ),
    WorkerCompositionError,
> {
    use crate::{
        adapters::postgres::{
            account_deletion::PostgresAccountDeletionRepository, runtime_pool,
            transactions::SqlxTransactionManager,
        },
        application::account_deletion::cleanup::{
            AccountObjectDeletionWorker, AccountObjectDeletionWorkerDependencies,
            AccountPurgeWorker, AccountPurgeWorkerDependencies,
        },
    };
    let pool = runtime_pool(config.database_url(), config.readiness_timeout())
        .map_err(|_| WorkerCompositionError::CleanupPostgres)?;
    let provider = S3AccountObjectDeletionProvider::new(object_storage, cleanup.credentials())
        .map_err(|_| WorkerCompositionError::CleanupStorage)?;
    let repository = Arc::new(PostgresAccountDeletionRepository::new(pool.clone()));
    let object_worker = AccountObjectDeletionWorker::new(
        AccountObjectDeletionWorkerDependencies {
            repository: repository.clone(),
            provider: Arc::new(provider),
        },
        cleanup.worker_config(),
    )
    .map_err(|_| WorkerCompositionError::CleanupWorker)?;
    let purge_worker = AccountPurgeWorker::new(
        AccountPurgeWorkerDependencies {
            transactions: Arc::new(SqlxTransactionManager::new(pool)),
            repository,
        },
        cleanup.purge_worker_config(),
    )
    .map_err(|_| WorkerCompositionError::CleanupWorker)?;
    Ok((object_worker, purge_worker))
}

async fn run_cleanup_until<F>(
    worker: crate::application::account_deletion::cleanup::AccountObjectDeletionWorker,
    shutdown: F,
) where
    F: Future<Output = ()> + Send,
{
    tokio::pin!(shutdown);
    loop {
        match worker.run_once().await {
            Ok(report) if report.claimed > 0 => tracing::info!(
                target: "jamye_server",
                event_kind = "account_object_deletion_worker_batch",
                claimed = report.claimed,
                succeeded = report.succeeded,
                retries = report.retries,
                failed = report.failed,
                dead_lettered = report.dead_lettered,
                stale_claims = report.stale_claims,
                "account object-deletion worker batch completed"
            ),
            Ok(_) => {}
            Err(_) => tracing::warn!(
                target: "jamye_server",
                dependency = "postgres",
                failure_kind = "worker_poll",
                "account object-deletion worker poll failed"
            ),
        }
        tokio::select! {
            () = &mut shutdown => break,
            () = tokio::time::sleep(worker.poll_interval()) => {},
        }
    }
}

async fn run_purge_until<F>(
    worker: crate::application::account_deletion::cleanup::AccountPurgeWorker,
    shutdown: F,
) where
    F: Future<Output = ()> + Send,
{
    tokio::pin!(shutdown);
    loop {
        match worker.run_once().await {
            Ok(report) if report.claimed > 0 => tracing::info!(
                target: "jamye_server",
                event_kind = "account_purge_worker_batch",
                claimed = report.claimed,
                purged = report.purged,
                failed = report.failed,
                released_claims = report.released_claims,
                stale_claims = report.stale_claims,
                "account purge worker batch completed"
            ),
            Ok(_) => {}
            Err(_) => tracing::warn!(
                target: "jamye_server",
                dependency = "postgres",
                failure_kind = "worker_poll",
                "account purge worker poll failed"
            ),
        }
        tokio::select! {
            () = &mut shutdown => break,
            () = tokio::time::sleep(worker.poll_interval()) => {},
        }
    }
}

pub fn router_with_readiness(readiness: ReadinessService) -> Router {
    with_platform_layers(health::router(HealthState::new(readiness)))
}

fn with_platform_layers(router: Router) -> Router {
    let request_id_header = HeaderName::from_static(REQUEST_ID_HEADER);
    let middleware = ServiceBuilder::new()
        .layer(middleware::from_fn(strip_external_request_id))
        .layer(SetSensitiveRequestHeadersLayer::new(once(AUTHORIZATION)))
        .layer(SetRequestIdLayer::new(
            request_id_header.clone(),
            MakeRequestUuid,
        ))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(request_span)
                .on_response(DefaultOnResponse::new().level(Level::INFO)),
        )
        .layer(PropagateRequestIdLayer::new(request_id_header));
    router.layer(middleware)
}

fn spawn_redis_forwarder(redis: Arc<RedisRealtimeAdapter>, delivery: AuthorizedRealtimeDelivery) {
    tokio::spawn(async move {
        loop {
            let mut subscriber = match redis.event_subscriber().await {
                Ok(subscriber) => subscriber,
                Err(_) => {
                    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                    continue;
                }
            };
            loop {
                let event = match subscriber.next_event().await {
                    Ok(Some(event)) => event,
                    Ok(None) | Err(RealtimePortError::Unavailable) => break,
                    Err(RealtimePortError::InvalidData) => {
                        tracing::warn!(
                            dependency = "redis",
                            failure_kind = "realtime_decode",
                            "dropping unrecognized realtime envelope"
                        );
                        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                        continue;
                    }
                };
                let conversation_id = event.conversation_id();
                match serde_json::to_string(&event) {
                    Ok(payload) => {
                        if delivery.publish(conversation_id, payload).await.is_err() {
                            tracing::warn!(
                                dependency = "postgres",
                                failure_kind = "realtime_delivery_authorization",
                                "realtime event delivery authorization failed"
                            );
                        }
                    }
                    Err(_) => tracing::warn!(
                        dependency = "serde_json",
                        failure_kind = "realtime_encode",
                        "realtime event could not be encoded"
                    ),
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
    });
}

fn spawn_realtime_control_consumer(
    redis: Arc<RedisRealtimeControl>,
    consumer: RealtimeControlConsumer,
) {
    tokio::spawn(async move {
        loop {
            let mut subscriber = match redis.subscriber().await {
                Ok(subscriber) => subscriber,
                Err(_) => {
                    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                    continue;
                }
            };
            while let Ok(Some(intent)) = subscriber.next_control().await {
                if consumer.apply(&intent).await.is_err() {
                    tracing::warn!(
                        dependency = "postgres",
                        failure_kind = "realtime_control",
                        "realtime control intent could not be applied"
                    );
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
    });
}

fn redis_probe(
    config: &AppConfig,
    timeout: std::time::Duration,
) -> Result<Arc<dyn DependencyProbe>, CompositionError> {
    match config.redis_url() {
        Some(url) => RedisHealthProbe::new(url, timeout)
            .map(|probe| Arc::new(probe) as Arc<dyn DependencyProbe>)
            .map_err(|_| CompositionError::Redis),
        None => Ok(Arc::new(UnconfiguredProbe)),
    }
}

fn minio_probe(
    config: &AppConfig,
    timeout: std::time::Duration,
) -> Result<Arc<dyn DependencyProbe>, CompositionError> {
    match config.minio_health_url() {
        Some(url) => MinioHealthProbe::new(url, timeout)
            .map(|probe| Arc::new(probe) as Arc<dyn DependencyProbe>)
            .map_err(|_| CompositionError::Minio),
        None => Ok(Arc::new(UnconfiguredProbe)),
    }
}

fn request_span(request: &Request<Body>) -> Span {
    let request_id = request
        .extensions()
        .get::<RequestId>()
        .and_then(|request_id| request_id.header_value().to_str().ok())
        .unwrap_or("missing");
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map(MatchedPath::as_str)
        .unwrap_or("unmatched");
    tracing::info_span!(
        "http.request",
        request_id = %request_id,
        method = %request.method(),
        route
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompositionError {
    Postgres,
    Redis,
    RedisNotConfigured,
    Minio,
    AppLinks,
    Auth,
    Groups,
    Media,
    ObjectStorageNotConfigured,
    Legal(LegalRenderError),
}

#[derive(Debug)]
pub enum WorkerCompositionError {
    Realtime(realtime_composition::WorkerCompositionError),
    Push(push_composition::WorkerCompositionError),
    CleanupPostgres,
    CleanupStorage,
    CleanupWorker,
}

impl fmt::Display for WorkerCompositionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Realtime(_) => formatter.write_str("failed to initialize the realtime worker"),
            Self::Push(_) => formatter.write_str("failed to initialize the push worker"),
            Self::CleanupPostgres => formatter.write_str("failed to initialize cleanup PostgreSQL"),
            Self::CleanupStorage => {
                formatter.write_str("failed to initialize cleanup object storage")
            }
            Self::CleanupWorker => {
                formatter.write_str("failed to initialize account cleanup worker")
            }
        }
    }
}

impl Error for WorkerCompositionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Realtime(error) => Some(error),
            Self::Push(error) => Some(error),
            Self::CleanupPostgres | Self::CleanupStorage | Self::CleanupWorker => None,
        }
    }
}

impl fmt::Display for CompositionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let component = match self {
            Self::Postgres => "PostgreSQL",
            Self::Redis | Self::RedisNotConfigured => "Redis",
            Self::Minio => "MinIO",
            Self::AppLinks => "app-link configuration",
            Self::Auth => "authentication composition",
            Self::Groups => "groups composition",
            Self::Media => "media composition",
            Self::ObjectStorageNotConfigured => "object-storage configuration",
            Self::Legal(error) => {
                return write!(formatter, "failed to initialize legal pages: {error}");
            }
        };
        write!(formatter, "failed to initialize {component}")
    }
}

impl Error for CompositionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Legal(error) => Some(error),
            _ => None,
        }
    }
}

#[cfg(test)]
mod supervisor_tests {
    use std::future;

    use tokio::{
        sync::{oneshot, watch},
        task::JoinSet,
    };

    use super::{WorkerRuntimeError, supervise_runners};

    #[tokio::test]
    async fn early_runner_completion_stops_and_joins_the_remaining_runner() {
        let (stop, receiver) = watch::channel(());
        let (stopped, observed_stop) = oneshot::channel();
        let mut runners = JoinSet::new();
        runners.spawn(async {});
        runners.spawn(async move {
            let mut receiver = receiver;
            let _ = receiver.changed().await;
            let _ = stopped.send(());
        });

        let result = supervise_runners(future::pending(), &mut runners, stop).await;

        assert_eq!(result, Err(WorkerRuntimeError::RunnerCompleted));
        assert!(
            observed_stop.await.is_ok(),
            "early completion must stop its sibling"
        );
    }

    #[tokio::test]
    async fn runner_panic_stops_and_joins_the_remaining_runner() {
        let (stop, receiver) = watch::channel(());
        let (stopped, observed_stop) = oneshot::channel();
        let mut runners = JoinSet::new();
        runners.spawn(async { panic!("supervisor test panic") });
        runners.spawn(async move {
            let mut receiver = receiver;
            let _ = receiver.changed().await;
            let _ = stopped.send(());
        });

        let result = supervise_runners(future::pending(), &mut runners, stop).await;

        assert_eq!(result, Err(WorkerRuntimeError::RunnerPanicked));
        assert!(
            observed_stop.await.is_ok(),
            "runner panic must stop its sibling"
        );
    }

    #[tokio::test]
    async fn shutdown_drain_reports_a_runner_panic() {
        let (stop, receiver) = watch::channel(());
        let mut runners = JoinSet::new();
        runners.spawn(async move {
            let mut receiver = receiver;
            let _ = receiver.changed().await;
            panic!("supervisor drain panic");
        });

        let result = supervise_runners(future::ready(()), &mut runners, stop).await;

        assert_eq!(result, Err(WorkerRuntimeError::RunnerPanicked));
    }
}
