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
use std::time::{Duration, Instant};

/// Adres klienta ustalony na podstawie trybu wdrożenia i listy zaufanych
/// proxy. W trybie local nagłówki proxy są zawsze ignorowane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClientIp(pub IpAddr);

pub const CF_CONNECTING_IP: &str = "cf-connecting-ip";

/// Skąd pochodzi ustalony adres klienta. Pozwala odróżnić prawidłowe
/// przekazanie adresu od cichego powrotu do adresu proxy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClientIpSource {
    /// Adres z gniazda TCP (tryb local lub połączenie spoza zaufanych proxy).
    Socket,
    /// Poprawny `CF-Connecting-IP` od zaufanego proxy.
    Forwarded,
    /// Zaufane proxy nie przekazało nagłówka — pozostaje adres proxy.
    MissingForwardedHeader,
    /// Nagłówek był nieprawidłowy — pozostaje adres proxy.
    InvalidForwardedHeader,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResolvedClientIp {
    pub ip: IpAddr,
    pub source: ClientIpSource,
}

/// Ustala adres klienta. W trybie tunnel zaufane proxy musi przekazać
/// poprawny `CF-Connecting-IP`; brak lub błąd nagłówka kończy się
/// zachowaniem adresu proxy, ale jest jawnie sygnalizowany przez
/// `source` (middleware loguje ostrzeżenie).
pub fn resolve_client_ip(config: &Config, peer: IpAddr, headers: &HeaderMap) -> ResolvedClientIp {
    let from_socket = ResolvedClientIp {
        ip: peer,
        source: ClientIpSource::Socket,
    };

    if config.mode == DeployMode::Local {
        return from_socket;
    }

    if !config
        .trusted_proxies
        .iter()
        .any(|network| network.contains(&peer))
    {
        return from_socket;
    }

    let Some(header) = headers.get(CF_CONNECTING_IP) else {
        return ResolvedClientIp {
            ip: peer,
            source: ClientIpSource::MissingForwardedHeader,
        };
    };

    match header.to_str().ok().and_then(parse_single_ip) {
        Some(ip) => ResolvedClientIp {
            ip,
            source: ClientIpSource::Forwarded,
        },
        None => ResolvedClientIp {
            ip: peer,
            source: ClientIpSource::InvalidForwardedHeader,
        },
    }
}

fn parse_single_ip(value: &str) -> Option<IpAddr> {
    let value = value.trim();
    if value.is_empty() || value.contains(',') {
        return None;
    }
    value.parse::<IpAddr>().ok()
}

/// Ostrzega (maks. raz na minutę), gdy w trybie tunnel zaufane proxy nie
/// przekazało poprawnego adresu klienta. Bez tego wszyscy użytkownicy
/// trafialiby po cichu do jednego koszyka adresu proxy.
fn warn_about_broken_forwarding(state: &AppState, resolved: ResolvedClientIp) {
    const WARNING_INTERVAL: Duration = Duration::from_secs(60);

    if !state.cfg.is_tunnel()
        || !matches!(
            resolved.source,
            ClientIpSource::MissingForwardedHeader | ClientIpSource::InvalidForwardedHeader
        )
    {
        return;
    }

    let mut last = state
        .client_ip_warning
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let now = Instant::now();
    let due = last.is_none_or(|previous| now.duration_since(previous) >= WARNING_INTERVAL);
    if !due {
        return;
    }
    *last = Some(now);

    tracing::warn!(
        source = ?resolved.source,
        "Tryb tunnel: zaufane proxy nie przekazało poprawnego CF-Connecting-IP. \
         Używam adresu proxy, więc limity per IP będą wspólne. \
         Sprawdź konfigurację cloudflared i TRUSTED_PROXIES."
    );
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
        let resolved = resolve_client_ip(&state.cfg, peer, request.headers());
        warn_about_broken_forwarding(&state, resolved);
        request.extensions_mut().insert(ClientIp(resolved.ip));
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
                ..EnvConfig::default()
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
        let resolved = resolve_client_ip(&config, peer, &headers("203.0.113.7"));
        assert_eq!(resolved.ip, peer);
        assert_eq!(resolved.source, ClientIpSource::Socket);
    }

    #[test]
    fn tunnel_mode_honors_header_only_from_trusted_proxy() {
        let config = config(DeployMode::Tunnel);
        let trusted_peer = "127.0.0.1".parse().unwrap();
        let untrusted_peer = "192.168.1.50".parse().unwrap();

        let forwarded = resolve_client_ip(&config, trusted_peer, &headers("203.0.113.7"));
        assert_eq!(forwarded.ip, "203.0.113.7".parse::<IpAddr>().unwrap());
        assert_eq!(forwarded.source, ClientIpSource::Forwarded);

        let direct = resolve_client_ip(&config, untrusted_peer, &headers("203.0.113.7"));
        assert_eq!(direct.ip, untrusted_peer);
        assert_eq!(direct.source, ClientIpSource::Socket);
    }

    #[test]
    fn tunnel_mode_flags_multiple_or_invalid_addresses() {
        let config = config(DeployMode::Tunnel);
        let trusted_peer = "127.0.0.1".parse().unwrap();

        let multiple = resolve_client_ip(&config, trusted_peer, &headers("203.0.113.7, 10.0.0.1"));
        assert_eq!(multiple.ip, trusted_peer);
        assert_eq!(multiple.source, ClientIpSource::InvalidForwardedHeader);

        let invalid = resolve_client_ip(&config, trusted_peer, &headers("nie-adres"));
        assert_eq!(invalid.ip, trusted_peer);
        assert_eq!(invalid.source, ClientIpSource::InvalidForwardedHeader);
    }

    #[test]
    fn tunnel_mode_flags_missing_header() {
        let config = config(DeployMode::Tunnel);
        let trusted_peer = "127.0.0.1".parse().unwrap();
        let resolved = resolve_client_ip(&config, trusted_peer, &HeaderMap::new());
        assert_eq!(resolved.ip, trusted_peer);
        assert_eq!(resolved.source, ClientIpSource::MissingForwardedHeader);
    }

    #[test]
    fn source_is_never_forwarded_without_trusted_proxy() {
        let config = config(DeployMode::Tunnel);
        let untrusted_peer = "203.0.113.50".parse().unwrap();
        let resolved = resolve_client_ip(&config, untrusted_peer, &headers("198.51.100.9"));
        assert_eq!(resolved.source, ClientIpSource::Socket);
        assert_eq!(resolved.ip, untrusted_peer);
    }
}
