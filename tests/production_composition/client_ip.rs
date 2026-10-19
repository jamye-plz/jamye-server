//! Rate-limit keys at the real HTTP boundaries and through the production listener.

#[allow(dead_code)]
#[path = "../auth/helpers.rs"]
mod auth_helpers;
#[path = "../avatar/support.rs"]
mod avatar_support;
#[allow(dead_code)]
#[path = "../groups/helpers.rs"]
mod groups_helpers;

use std::{sync::Arc, time::Duration};

use axum::{
    Router,
    body::Body,
    http::{Method, Request, StatusCode},
};
use jamye_server::{
    adapters::postgres::{avatar::PostgresAvatarRepository, transactions::SqlxTransactionManager},
    application::{
        avatar::{AvatarDependencies, AvatarEndpointRateLimit, AvatarService, AvatarSettings},
        media::MediaEndpointRateLimit,
    },
    platform::shutdown::serve_with_graceful_shutdown,
    transport::http::{auth, avatar, groups},
};
use sqlx::postgres::PgPoolOptions;
use tokio::{net::TcpListener, sync::oneshot};
use tower::ServiceExt;
use uuid::Uuid;

use crate::{
    TestResult,
    client_ip_support::{router_with_trusted_proxies, with_peer_and_xff},
};

const TRUSTED: &str = "192.0.2.10";
const UNTRUSTED: &str = "192.0.2.11";
const CLIENT_A: &str = "198.51.100.20";
const CLIENT_B: &str = "2001:db8::20";
const CLIENT_B_PREFIX: &str = "2001:db8::";

#[derive(Clone, Copy)]
enum Route {
    Auth,
    Avatar,
    Groups,
}

const ROUTES: [Route; 3] = [Route::Auth, Route::Avatar, Route::Groups];

impl Route {
    fn endpoint(self) -> &'static str {
        match self {
            Self::Auth => "oauth_authorize",
            Self::Avatar => "avatar_public_read",
            Self::Groups => "invite_issue",
        }
    }

    fn method(self) -> Method {
        match self {
            Self::Avatar => Method::GET,
            Self::Auth | Self::Groups => Method::POST,
        }
    }

    fn path(self) -> String {
        match self {
            Self::Auth => "/api/v1/auth/oauth/kakao/authorize".to_owned(),
            Self::Avatar => "/api/v1/avatars/not-an-avatar".to_owned(),
            Self::Groups => format!("/api/v1/groups/{}/invites", Uuid::nil()),
        }
    }

    fn body(self) -> String {
        match self {
            Self::Auth => serde_json::json!({
                "redirect_uri": auth_helpers::KAKAO_REDIRECT,
                "code_challenge": "c".repeat(43),
                "code_challenge_method": "S256"
            })
            .to_string(),
            Self::Avatar | Self::Groups => "{}".to_owned(),
        }
    }

    fn expected_subject(self, actor: Uuid, ip: Option<&str>) -> String {
        match self {
            Self::Auth => format!("ip:{}", ip.unwrap_or("unavailable")),
            Self::Avatar => format!("ip:{}", ip.unwrap_or("unknown")),
            Self::Groups => format!("user:{actor}:ip:{}", ip.unwrap_or("unavailable")),
        }
    }
}

struct Fixture {
    router: Router,
    limiter: Arc<avatar_support::RecordingLimiter>,
    actor: Uuid,
}

impl Fixture {
    fn new(trusted_proxies: Option<&str>) -> TestResult<Self> {
        // The denied limiter stops each route before any database access.
        let pool = PgPoolOptions::new().connect_lazy("postgres://localhost/jamye_test")?;
        let limiter = Arc::new(avatar_support::RecordingLimiter::default());
        let auth_harness =
            auth_helpers::harness_with_rate_limiter(pool.clone(), None, limiter.clone())?;
        let auth_router = auth::router(auth::AuthHttpState::new(
            auth_harness.service,
            auth_harness.codec,
        ));
        let group_harness = groups_helpers::harness_with_limiter(pool.clone(), limiter.clone())?;
        let group_router = groups::router(groups::GroupsHttpState::new(
            group_harness.service,
            Arc::new(groups_helpers::TestAccessVerifier),
        ));
        let avatar_service = AvatarService::new(
            AvatarDependencies {
                transactions: Arc::new(SqlxTransactionManager::new(pool.clone())),
                repository: Arc::new(PostgresAvatarRepository::new(pool)),
                object_storage: Arc::new(avatar_support::FakeObjectStorage::default()),
                rate_limiter: limiter.clone(),
            },
            AvatarSettings {
                public_base_url: avatar_support::BASE.to_owned(),
                upload_rate_limit: MediaEndpointRateLimit {
                    limit: 2,
                    window: Duration::from_secs(60),
                },
                public_read_rate_limit: AvatarEndpointRateLimit {
                    limit: 3,
                    window: Duration::from_secs(10),
                },
            },
        )?;
        let avatar_router = avatar::router(avatar::AvatarHttpState::new(
            Arc::new(avatar_service),
            Arc::new(groups_helpers::TestAccessVerifier),
        ));
        let router = router_with_trusted_proxies(
            auth_router.merge(avatar_router).merge(group_router),
            trusted_proxies,
        )?;
        Ok(Self {
            router,
            limiter,
            actor: Uuid::new_v4(),
        })
    }

    fn assert_last_subject(&self, route: Route, previous_count: usize, expected: Option<&str>) {
        let requests = self.limiter.requests();
        assert_eq!(requests.len(), previous_count + 1);
        let request = &requests[previous_count];
        assert_eq!(request.endpoint, route.endpoint());
        assert_eq!(
            request.subject,
            route.expected_subject(self.actor, expected)
        );
    }

    async fn check(
        &self,
        peer: Option<&str>,
        headers: &[&str],
        expected: Option<&str>,
    ) -> TestResult {
        for route in ROUTES {
            self.limiter.deny(route.endpoint());
            let previous_count = self.limiter.requests().len();
            let builder = Request::builder()
                .method(route.method())
                .uri(route.path())
                .header("content-type", "application/json")
                .header("authorization", groups_helpers::bearer(self.actor));
            let request = builder.body(Body::from(route.body()))?;
            let request = with_peer_and_xff(request, peer, headers)?;
            let response = self.router.clone().oneshot(request).await?;
            assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
            self.assert_last_subject(route, previous_count, expected);
        }
        Ok(())
    }
}

#[tokio::test]
async fn all_three_limiters_trusted_peer_single_xff_gives_distinct_buckets() -> TestResult {
    let fixture = Fixture::new(Some("192.0.2.9, 192.0.2.10, 2001:db8::1"))?;
    fixture
        .check(Some(TRUSTED), &[CLIENT_A], Some(CLIENT_A))
        .await?;
    fixture
        .check(Some(TRUSTED), &["  2001:db8::20  "], Some(CLIENT_B_PREFIX))
        .await?;
    fixture
        .check(Some(TRUSTED), &["2001:db8::21"], Some(CLIENT_B_PREFIX))
        .await?;
    fixture
        .check(Some(TRUSTED), &["2001:db8:0:1::20"], Some("2001:db8:0:1::"))
        .await?;
    fixture
        .check(Some(TRUSTED), &["::ffff:198.51.100.20"], Some(CLIENT_A))
        .await?;
    Ok(())
}

#[tokio::test]
async fn all_three_limiters_untrusted_peer_xff_is_ignored_and_keys_on_peer() -> TestResult {
    let fixture = Fixture::new(Some(TRUSTED))?;
    fixture
        .check(Some(UNTRUSTED), &[CLIENT_A], Some(UNTRUSTED))
        .await?;
    fixture
        .check(Some(UNTRUSTED), &[CLIENT_B], Some(UNTRUSTED))
        .await?;
    fixture.check(None, &[CLIENT_A], None).await?;
    Ok(())
}

#[tokio::test]
async fn all_three_limiters_unset_or_empty_trusted_proxies_keys_on_peer_and_ignores_xff()
-> TestResult {
    for trusted in [None, Some(""), Some(" \t ")] {
        Fixture::new(trusted)?
            .check(Some(TRUSTED), &[CLIENT_A], Some(TRUSTED))
            .await?;
    }
    Ok(())
}

#[tokio::test]
async fn all_three_limiters_multi_value_or_malformed_xff_falls_back_to_peer() -> TestResult {
    let fixture = Fixture::new(Some(TRUSTED))?;
    for headers in [
        vec![],
        vec![""],
        vec!["not-an-ip"],
        vec!["198.51.100.20:443"],
        vec!["198.51.100.20, 2001:db8::20"],
        vec![CLIENT_A, CLIENT_B],
        vec![CLIENT_A, CLIENT_A],
    ] {
        fixture
            .check(Some(TRUSTED), &headers, Some(TRUSTED))
            .await?;
    }
    Ok(())
}

#[tokio::test]
async fn connect_info_reaches_all_three_limiters_over_tcp() -> TestResult {
    let fixture = Fixture::new(None)?;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let server = tokio::spawn(serve_with_graceful_shutdown(
        listener,
        fixture.router.clone(),
        async move {
            let _ = shutdown_rx.await;
        },
        Duration::from_secs(2),
    ));
    let client = reqwest::Client::builder().no_proxy().build()?;
    for route in ROUTES {
        fixture.limiter.deny(route.endpoint());
        let previous_count = fixture.limiter.requests().len();
        let response = client
            .request(route.method(), format!("http://{address}{}", route.path()))
            .header("content-type", "application/json")
            .header("authorization", groups_helpers::bearer(fixture.actor))
            .body(route.body())
            .send()
            .await?;
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        fixture.assert_last_subject(route, previous_count, Some("127.0.0.1"));
    }
    shutdown_tx
        .send(())
        .map_err(|_| "server stopped before shutdown")?;
    tokio::time::timeout(Duration::from_secs(5), server).await???;
    Ok(())
}
