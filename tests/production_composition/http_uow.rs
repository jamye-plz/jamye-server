//! Durable HTTP-boundary RED fixtures for the three Task-12 UoWs.
//!
//! Each fault is a disposable-database trigger. Its `nextval` witness is
//! nontransactional, so the intended cumulative write is observable even when
//! the real handler transaction rolls back. No production callback or port is
//! introduced.

use std::{env, io};

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{
        Method, Request, StatusCode,
        header::{AUTHORIZATION, CONTENT_TYPE},
    },
};
use jamye_server::{
    adapters::oauth::{ProductionTokenCodec, ProductionTokenConfigError},
    config::{
        AppConfig, AppEnvironment, ConfigInput,
        auth::{AuthConfig, AuthConfigInput},
        object_storage::{ObjectStorageConfig, ObjectStorageConfigInput},
        rate_limit::RateLimitConfig,
    },
    ports::auth::AccessTokenIssuer,
    transport::http::composition,
};
use serde_json::{Value, json};
use sqlx::{AssertSqlSafe, PgPool, types::Json};

use time::{Duration, OffsetDateTime};
use tower::ServiceExt;
use url::Url;
use uuid::Uuid;

use crate::{TestResult, postgres::PostgresFixture, postgres_support::TestDatabase};

include!("http_uow/http_cases.rs");
include!("http_uow/composition_faults.rs");
include!("http_uow/http_requests.rs");
include!("http_uow/retry_relations.rs");
include!("http_uow/configuration_support.rs");
