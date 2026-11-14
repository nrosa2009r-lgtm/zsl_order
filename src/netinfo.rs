// SPDX-License-Identifier: MIT
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};

/// Adresy, pod którymi serwer jest osiągalny z sieci lokalnej.
///
/// Dla jawnego adresu zwracamy go bez zmian. Dla `0.0.0.0` ustalamy adres
/// podstawowego interfejsu (trasa domyślna). Dla `::` nie zgadujemy adresu —
/// puste pole oznacza „wszystkie interfejsy”, a użytkownik może sprawdzić
/// adres poleceniem `ip addr`.
pub fn access_urls(bind: SocketAddr) -> Vec<String> {
    let port = bind.port();

    match bind.ip() {
        IpAddr::V4(ip) if !ip.is_unspecified() => vec![format!("http://{ip}:{port}")],
        IpAddr::V4(_) => primary_ipv4()
            .map(|ip| vec![format!("http://{ip}:{port}")])
            .unwrap_or_default(),
        IpAddr::V6(ip) if !ip.is_unspecified() => vec![format!("http://[{ip}]:{port}")],
        IpAddr::V6(_) => Vec::new(),
    }
}

/// Adres IPv4 interfejsu, przez który wychodzi ruch domyślny.
///
/// `UdpSocket::connect` nie wysyła pakietów — służy tylko do tego, aby jądro
/// wybrało adres źródłowy trasy. Dzięki temu działa też bez internetu.
fn primary_ipv4() -> Option<Ipv4Addr> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    // TEST-NET-1 (RFC 5737) — adres nigdy nie jest celem realnego ruchu.
    socket.connect("192.0.2.1:9").ok()?;

    match socket.local_addr().ok()?.ip() {
        IpAddr::V4(ip) if !ip.is_loopback() && !ip.is_unspecified() => Some(ip),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_ipv4_address_is_reported_verbatim() {
        let addr: SocketAddr = "192.168.0.124:3000".parse().unwrap();
        assert_eq!(access_urls(addr), vec!["http://192.168.0.124:3000"]);
    }

    #[test]
    fn explicit_ipv6_address_is_bracketed() {
        let addr: SocketAddr = "[fd00::1]:8080".parse().unwrap();
        assert_eq!(access_urls(addr), vec!["http://[fd00::1]:8080"]);
    }

    #[test]
    fn loopback_is_reported_as_is() {
        let addr: SocketAddr = "127.0.0.1:3000".parse().unwrap();
        assert_eq!(access_urls(addr), vec!["http://127.0.0.1:3000"]);
    }

    #[test]
    fn wildcard_ipv4_never_yields_a_wildcard_url() {
        let addr: SocketAddr = "0.0.0.0:3000".parse().unwrap();
        for url in access_urls(addr) {
            assert!(!url.contains("0.0.0.0"), "niepoprawny adres: {url}");
        }
    }

    #[test]
    fn wildcard_ipv6_returns_no_guess() {
        let addr: SocketAddr = "[::]:3000".parse().unwrap();
        assert!(access_urls(addr).is_empty());
    }
}
