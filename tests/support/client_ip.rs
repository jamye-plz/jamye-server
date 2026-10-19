use std::net::SocketAddr;

use axum::{
    Router,
    extract::ConnectInfo,
    http::{HeaderValue, Request, header::HeaderName},
};
use jamye_server::{
    config::{AppConfig, ConfigInput},
    transport::http::client_ip,
};

use crate::TestResult;

pub fn router_with_trusted_proxies(
    router: Router,
    trusted_proxies: Option<&str>,
) -> TestResult<Router> {
    let config = AppConfig::try_from(ConfigInput {
        environment: Some("test".to_owned()),
        database_url: Some("postgres://localhost/jamye_test".to_owned()),
        trusted_proxies: trusted_proxies.map(ToOwned::to_owned),
        ..ConfigInput::default()
    })?;
    Ok(client_ip::with_trusted_proxies(router, &config))
}

pub fn with_peer_and_xff<B>(
    mut request: Request<B>,
    peer: Option<&str>,
    forwarded_for: &[&str],
) -> TestResult<Request<B>> {
    let header = HeaderName::from_static("x-forwarded-for");
    for value in forwarded_for {
        request
            .headers_mut()
            .append(header.clone(), HeaderValue::from_str(value)?);
    }
    if let Some(peer) = peer {
        request
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::new(peer.parse()?, 4000)));
    }
    Ok(request)
}
