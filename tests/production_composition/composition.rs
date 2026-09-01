//! Black-box RED coverage for the exact API and worker roots invoked by the binaries.

#[path = "http_uow.rs"]
mod http_uow;

use axum::{
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode},
};
use jamye_server::{
    config::{
        AppConfig, AppEnvironment, ConfigInput,
        account_deletion::{AccountDeletionConfig, AccountDeletionConfigInput},
        auth::{AuthConfig, AuthConfigInput},
        object_storage::{ObjectStorageConfig, ObjectStorageConfigInput},
        push::{PushConfig, PushConfigInput},
        rate_limit::RateLimitConfig,
    },
    transport::http::composition,
};
use tower::ServiceExt;

use crate::TestResult;

include!("composition/api_root_cases.rs");
include!("composition/surface_inventory.rs");
include!("composition/assertions.rs");
