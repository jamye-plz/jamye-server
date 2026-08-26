//! Conservative configurable fixed-window defaults.

use std::{env, fmt, time::Duration};

use crate::application::{
    auth::{AuthRateLimitPolicy, EndpointRateLimit},
    groups::{GroupsEndpointRateLimit, GroupsRateLimitPolicy},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RateLimitConfig {
    pub auth: AuthRateLimitPolicy,
    pub groups: GroupsRateLimitPolicy,
}

impl RateLimitConfig {
    pub fn from_env() -> Result<Self, RateLimitConfigError> {
        Self::try_from(RateLimitConfigInput::from_env())
    }
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        let one_minute = Duration::from_secs(60);
        Self {
            auth: AuthRateLimitPolicy {
                authorize: EndpointRateLimit {
                    limit: 10,
                    window: one_minute,
                },
                exchange: EndpointRateLimit {
                    limit: 20,
                    window: one_minute,
                },
                refresh: EndpointRateLimit {
                    limit: 30,
                    window: one_minute,
                },
                logout: EndpointRateLimit {
                    limit: 30,
                    window: one_minute,
                },
            },
            groups: GroupsRateLimitPolicy {
                invite_issue: GroupsEndpointRateLimit {
                    limit: 10,
                    window: one_minute,
                },
                invite_redeem: GroupsEndpointRateLimit {
                    limit: 20,
                    window: one_minute,
                },
            },
        }
    }
}

#[derive(Clone, Default)]
pub struct RateLimitConfigInput {
    pub authorize_limit: Option<String>,
    pub authorize_window_seconds: Option<String>,
    pub exchange_limit: Option<String>,
    pub exchange_window_seconds: Option<String>,
    pub refresh_limit: Option<String>,
    pub refresh_window_seconds: Option<String>,
    pub logout_limit: Option<String>,
    pub logout_window_seconds: Option<String>,
    pub invite_issue_limit: Option<String>,
    pub invite_issue_window_seconds: Option<String>,
    pub invite_redeem_limit: Option<String>,
    pub invite_redeem_window_seconds: Option<String>,
}

impl RateLimitConfigInput {
    fn from_env() -> Self {
        Self {
            authorize_limit: env::var("JAMYE_RATE_LIMIT_OAUTH_AUTHORIZE_LIMIT").ok(),
            authorize_window_seconds: env::var("JAMYE_RATE_LIMIT_OAUTH_AUTHORIZE_WINDOW_SECONDS")
                .ok(),
            exchange_limit: env::var("JAMYE_RATE_LIMIT_OAUTH_EXCHANGE_LIMIT").ok(),
            exchange_window_seconds: env::var("JAMYE_RATE_LIMIT_OAUTH_EXCHANGE_WINDOW_SECONDS")
                .ok(),
            refresh_limit: env::var("JAMYE_RATE_LIMIT_AUTH_REFRESH_LIMIT").ok(),
            refresh_window_seconds: env::var("JAMYE_RATE_LIMIT_AUTH_REFRESH_WINDOW_SECONDS").ok(),
            logout_limit: env::var("JAMYE_RATE_LIMIT_AUTH_LOGOUT_LIMIT").ok(),
            logout_window_seconds: env::var("JAMYE_RATE_LIMIT_AUTH_LOGOUT_WINDOW_SECONDS").ok(),
            invite_issue_limit: env::var("JAMYE_RATE_LIMIT_INVITE_ISSUE_LIMIT").ok(),
            invite_issue_window_seconds: env::var("JAMYE_RATE_LIMIT_INVITE_ISSUE_WINDOW_SECONDS")
                .ok(),
            invite_redeem_limit: env::var("JAMYE_RATE_LIMIT_INVITE_REDEEM_LIMIT").ok(),
            invite_redeem_window_seconds: env::var("JAMYE_RATE_LIMIT_INVITE_REDEEM_WINDOW_SECONDS")
                .ok(),
        }
    }
}

impl TryFrom<RateLimitConfigInput> for RateLimitConfig {
    type Error = RateLimitConfigError;

    fn try_from(input: RateLimitConfigInput) -> Result<Self, Self::Error> {
        Ok(Self {
            auth: AuthRateLimitPolicy {
                authorize: endpoint(
                    "JAMYE_RATE_LIMIT_OAUTH_AUTHORIZE_LIMIT",
                    input.authorize_limit,
                    10,
                    "JAMYE_RATE_LIMIT_OAUTH_AUTHORIZE_WINDOW_SECONDS",
                    input.authorize_window_seconds,
                )?,
                exchange: endpoint(
                    "JAMYE_RATE_LIMIT_OAUTH_EXCHANGE_LIMIT",
                    input.exchange_limit,
                    20,
                    "JAMYE_RATE_LIMIT_OAUTH_EXCHANGE_WINDOW_SECONDS",
                    input.exchange_window_seconds,
                )?,
                refresh: endpoint(
                    "JAMYE_RATE_LIMIT_AUTH_REFRESH_LIMIT",
                    input.refresh_limit,
                    30,
                    "JAMYE_RATE_LIMIT_AUTH_REFRESH_WINDOW_SECONDS",
                    input.refresh_window_seconds,
                )?,
                logout: endpoint(
                    "JAMYE_RATE_LIMIT_AUTH_LOGOUT_LIMIT",
                    input.logout_limit,
                    30,
                    "JAMYE_RATE_LIMIT_AUTH_LOGOUT_WINDOW_SECONDS",
                    input.logout_window_seconds,
                )?,
            },
            groups: GroupsRateLimitPolicy {
                invite_issue: groups_endpoint(
                    "JAMYE_RATE_LIMIT_INVITE_ISSUE_LIMIT",
                    input.invite_issue_limit,
                    10,
                    "JAMYE_RATE_LIMIT_INVITE_ISSUE_WINDOW_SECONDS",
                    input.invite_issue_window_seconds,
                )?,
                invite_redeem: groups_endpoint(
                    "JAMYE_RATE_LIMIT_INVITE_REDEEM_LIMIT",
                    input.invite_redeem_limit,
                    20,
                    "JAMYE_RATE_LIMIT_INVITE_REDEEM_WINDOW_SECONDS",
                    input.invite_redeem_window_seconds,
                )?,
            },
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RateLimitConfigError {
    key: &'static str,
}

impl RateLimitConfigError {
    pub fn key(&self) -> &'static str {
        self.key
    }
}

impl fmt::Display for RateLimitConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "invalid rate-limit configuration for {}",
            self.key
        )
    }
}

impl std::error::Error for RateLimitConfigError {}

fn endpoint(
    limit_key: &'static str,
    limit: Option<String>,
    default_limit: u32,
    window_key: &'static str,
    window: Option<String>,
) -> Result<EndpointRateLimit, RateLimitConfigError> {
    let limit = match limit {
        Some(value) => value
            .parse::<u32>()
            .ok()
            .filter(|limit| (1..=10_000).contains(limit))
            .ok_or(RateLimitConfigError { key: limit_key })?,
        None => default_limit,
    };
    let window = match window {
        Some(value) => value
            .parse::<u64>()
            .ok()
            .filter(|window| (1..=86_400).contains(window))
            .ok_or(RateLimitConfigError { key: window_key })?,
        None => 60,
    };
    Ok(EndpointRateLimit {
        limit,
        window: Duration::from_secs(window),
    })
}

fn groups_endpoint(
    limit_key: &'static str,
    limit: Option<String>,
    default_limit: u32,
    window_key: &'static str,
    window: Option<String>,
) -> Result<GroupsEndpointRateLimit, RateLimitConfigError> {
    endpoint(limit_key, limit, default_limit, window_key, window).map(|policy| {
        GroupsEndpointRateLimit {
            limit: policy.limit,
            window: policy.window,
        }
    })
}
