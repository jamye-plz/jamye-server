//! Server-issued request identifier boundary.

use axum::{
    extract::Request,
    middleware::Next,
    response::Response,
};

pub const REQUEST_ID_HEADER: &str = "x-request-id";

/// Removes caller-controlled request IDs before the generation layer runs.
pub async fn strip_external_request_id(mut request: Request, next: Next) -> Response {
    request.headers_mut().remove(REQUEST_ID_HEADER);
    next.run(request).await
}
