use crate::config::{Config, DeployMode};
use crate::state::AppState;
use axum::{
    extract::{ConnectInfo, Request, State},
    http::HeaderMap,
    middleware::Next,
    response::Response,
};
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

/// Adres klienta ustalony na podstawie trybu wdrożenia i listy zaufanych
/// proxy. W trybie local nagłówki proxy są zawsze ignorowane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClientIp(pub IpAddr);

pub const CF_CONNECTING_IP: &str = "cf-connecting-ip";

pub fn resolve_client_ip(config: &Config, peer: IpAddr, headers: &HeaderMap) -> IpAddr {
    if config.mode == DeployMode::Local {
        return peer;
    }

    if !config
        .trusted_proxies
        .iter()
        .any(|network| network.contains(&peer))
    {
        return peer;
    }

    headers
        .get(CF_CONNECTING_IP)
        .and_then(|value| value.to_str().ok())
        .and_then(parse_single_ip)
        .unwrap_or(peer)
}

fn parse_single_ip(value: &str) -> Option<IpAddr> {
    let value = value.trim();
    if value.is_empty() || value.contains(',') {
        return None;
    }
    value.parse::<IpAddr>().ok()
}

/// Wstawia `ClientIp` do rozszerzeń żądania przed handlerami i limiterami.
/// Adres gniazda pochodzi z `ConnectInfo` dodawanego przez axum. W testach
/// można wstawić `ConnectInfo` samodzielnie.
pub async fn client_ip_middleware(
    State(state): State<Arc<AppState>>,
    mut request: Request,
    next: Next,
) -> Response {
    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|address| address.0.ip())
        .or_else(|| request.extensions().get::<SocketAddr>().map(SocketAddr::ip))
        .or_else(|| request.extensions().get::<ClientIp>().map(|ip| ip.0));

    if let Some(peer) = peer {
        let ip = resolve_client_ip(&state.cfg, peer, request.headers());
        request.extensions_mut().insert(ClientIp(ip));
    }

    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::EnvConfig;
    use axum::http::HeaderValue;

    fn config(mode: DeployMode) -> Config {
        Config::from_file_and_env(
            Default::default(),
            EnvConfig {
                deploy_mode: Some(mode.as_str().to_string()),
                bind_addr: Some("127.0.0.1:3000".to_string()),
                trusted_proxies: Some("127.0.0.1,::1".to_string()),
                public_url: Some("https://zsl.example.com".to_string()),
                cookie_secure: Some("auto".to_string()),
            },
        )
        .unwrap()
    }

    fn headers(value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(CF_CONNECTING_IP, HeaderValue::from_str(value).unwrap());
        headers
    }

    #[test]
    fn local_mode_ignores_spoofed_header() {
        let config = config(DeployMode::Local);
        let peer = "192.168.1.50".parse().unwrap();
        assert_eq!(
            resolve_client_ip(&config, peer, &headers("203.0.113.7")),
            peer
        );
    }

    #[test]
    fn tunnel_mode_honors_header_only_from_trusted_proxy() {
        let config = config(DeployMode::Tunnel);
        let trusted_peer = "127.0.0.1".parse().unwrap();
        let untrusted_peer = "192.168.1.50".parse().unwrap();

        assert_eq!(
            resolve_client_ip(&config, trusted_peer, &headers("203.0.113.7")),
            "203.0.113.7".parse::<IpAddr>().unwrap()
        );
        assert_eq!(
            resolve_client_ip(&config, untrusted_peer, &headers("203.0.113.7")),
            untrusted_peer
        );
    }

    #[test]
    fn tunnel_mode_rejects_multiple_or_invalid_addresses() {
        let config = config(DeployMode::Tunnel);
        let trusted_peer = "127.0.0.1".parse().unwrap();

        assert_eq!(
            resolve_client_ip(&config, trusted_peer, &headers("203.0.113.7, 10.0.0.1")),
            trusted_peer
        );
        assert_eq!(
            resolve_client_ip(&config, trusted_peer, &headers("nie-adres")),
            trusted_peer
        );
    }

    #[test]
    fn tunnel_mode_without_header_uses_peer() {
        let config = config(DeployMode::Tunnel);
        let trusted_peer = "127.0.0.1".parse().unwrap();
        assert_eq!(
            resolve_client_ip(&config, trusted_peer, &HeaderMap::new()),
            trusted_peer
        );
    }
}
