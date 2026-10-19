//! Resolve client addresses only across explicitly trusted, single-hop proxies.

use std::{
    net::{IpAddr, Ipv6Addr, SocketAddr},
    sync::Arc,
};

use axum::{
    Extension, Router,
    extract::ConnectInfo,
    http::{header::HeaderName, request::Parts},
};

use crate::config::AppConfig;

#[derive(Clone)]
struct TrustedProxyPeers(Arc<[IpAddr]>);

/// Install the same validated proxy set on all HTTP routes.
pub fn with_trusted_proxies(router: Router, config: &AppConfig) -> Router {
    router.layer(Extension(TrustedProxyPeers(Arc::from(
        config.trusted_proxies(),
    ))))
}

/// Return a forwarded address only when exactly one trusted hop supplied it.
pub fn forwarded_client_ip(parts: &Parts) -> Option<IpAddr> {
    let peer = parts.extensions.get::<ConnectInfo<SocketAddr>>()?.0.ip();
    let trusted = parts.extensions.get::<TrustedProxyPeers>()?;
    if !trusted.0.contains(&peer) {
        return None;
    }
    let mut values = parts
        .headers
        .get_all(HeaderName::from_static("x-forwarded-for"))
        .iter();
    let value = values.next()?;
    if values.next().is_some() {
        return None;
    }
    value.to_str().ok()?.trim().parse().ok()
}

/// Prefer a validated forwarded address, then the TCP peer, then no address.
pub fn client_ip(parts: &Parts) -> Option<IpAddr> {
    forwarded_client_ip(parts).or_else(|| {
        parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .map(|peer| peer.0.ip())
    })
}

/// Key rate limits on the IPv6 /64 a host controls, so rotating addresses inside it
/// does not open new buckets; IPv4 and IPv4-mapped addresses stay exact.
pub fn rate_limit_address(address: IpAddr) -> IpAddr {
    match address {
        IpAddr::V4(_) => address,
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => IpAddr::V4(v4),
            None => {
                let [a, b, c, d, ..] = v6.segments();
                IpAddr::V6(Ipv6Addr::new(a, b, c, d, 0, 0, 0, 0))
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{TrustedProxyPeers, forwarded_client_ip, rate_limit_address};
    use std::{
        error::Error,
        net::{IpAddr, SocketAddr},
        sync::Arc,
    };

    use axum::{
        extract::ConnectInfo,
        http::{HeaderValue, Request},
    };

    #[test]
    fn forwarded_resolver_rejects_untrusted_missing_and_invalid_headers()
    -> Result<(), Box<dyn Error>> {
        let trusted: SocketAddr = "192.0.2.10:4000".parse()?;
        let mut request = Request::new(());
        request
            .extensions_mut()
            .insert(TrustedProxyPeers(Arc::from(vec![trusted.ip()])));
        request.extensions_mut().insert(ConnectInfo(trusted));
        let (mut parts, _) = request.into_parts();

        assert_eq!(forwarded_client_ip(&parts), None);
        parts.headers.insert(
            "x-forwarded-for",
            HeaderValue::from_static(" 2001:db8::20 "),
        );
        assert_eq!(
            forwarded_client_ip(&parts),
            Some("2001:db8::20".parse::<std::net::IpAddr>()?)
        );
        parts
            .headers
            .append("x-forwarded-for", HeaderValue::from_static("198.51.100.20"));
        assert_eq!(forwarded_client_ip(&parts), None);
        for value in ["", "bad", "198.51.100.20, 192.0.2.1", "192.0.2.1/24"] {
            parts
                .headers
                .insert("x-forwarded-for", HeaderValue::from_str(value)?);
            assert_eq!(forwarded_client_ip(&parts), None);
        }
        parts
            .headers
            .insert("x-forwarded-for", HeaderValue::from_bytes(&[0xff])?);
        assert_eq!(forwarded_client_ip(&parts), None);
        parts
            .headers
            .insert("x-forwarded-for", HeaderValue::from_static("198.51.100.20"));
        parts
            .extensions
            .insert(ConnectInfo("192.0.2.11:4000".parse::<SocketAddr>()?));
        assert_eq!(forwarded_client_ip(&parts), None);
        parts.extensions.remove::<ConnectInfo<SocketAddr>>();
        assert_eq!(forwarded_client_ip(&parts), None);
        Ok(())
    }

    #[test]
    fn rate_limit_address_groups_ipv6_by_prefix_and_keeps_ipv4_exact() -> Result<(), Box<dyn Error>>
    {
        for (address, expected) in [
            ("198.51.100.20", "198.51.100.20"),
            ("::ffff:198.51.100.20", "198.51.100.20"),
            ("2001:db8::20", "2001:db8::"),
            ("2001:db8::1:2:3:4", "2001:db8::"),
            ("2001:db8:0:1::20", "2001:db8:0:1::"),
            ("::1", "::"),
        ] {
            assert_eq!(
                rate_limit_address(address.parse::<IpAddr>()?),
                expected.parse::<IpAddr>()?,
                "{address}"
            );
        }
        Ok(())
    }
}
