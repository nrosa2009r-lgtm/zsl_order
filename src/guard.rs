// SPDX-License-Identifier: MIT
use crate::config::{Config, PublicUrl, authority_eq, is_loopback_authority};

/// W trybie tunnel akceptujemy wyłącznie publiczny Host oraz adresy
/// pętlowe (diagnostyka lokalna). W trybie local Host jest przyjmowany
/// tak, jak go widzi serwer — bezpieczeństwo stanu zapewnia porównanie
/// Origin z Host i token CSRF.
pub fn host_allowed(config: &Config, host: &str) -> bool {
    if !config.is_tunnel() {
        return true;
    }

    if is_loopback_authority(host) {
        return true;
    }

    config
        .public_url
        .as_ref()
        .is_some_and(|public| authority_eq(public.host_header(), host))
}

/// Sprawdza, czy nagłówek Origin pochodzi z dozwolonego miejsca.
/// W trybie tunnel jedynym dozwolonym originem jest PUBLIC_URL.
/// W trybie local akceptujemy PUBLIC_URL oraz bieżący Host (HTTP lub
/// HTTPS, np. za lokalnym reverse proxy) i adresy pętlowe.
pub fn origin_allowed(config: &Config, host: &str, origin: &str) -> bool {
    let Ok(origin_url) = PublicUrl::parse(origin) else {
        return false;
    };

    if let Some(public) = &config.public_url
        && public.origin().eq_ignore_ascii_case(origin_url.origin())
    {
        return true;
    }

    if !config.is_tunnel() {
        return authority_eq(origin_url.host_header(), host)
            || is_loopback_authority(origin_url.host_header());
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{DeployMode, EnvConfig};

    fn config(mode: DeployMode, public_url: Option<&str>) -> Config {
        Config::from_file_and_env(
            Default::default(),
            EnvConfig {
                deploy_mode: Some(mode.as_str().to_string()),
                bind_addr: Some("127.0.0.1:3000".to_string()),
                trusted_proxies: None,
                public_url: public_url.map(str::to_string),
                cookie_secure: Some("auto".to_string()),
                ..EnvConfig::default()
            },
        )
        .unwrap()
    }

    #[test]
    fn tunnel_accepts_only_public_or_loopback_host() {
        let config = config(DeployMode::Tunnel, Some("https://zsl.example.com"));

        assert!(host_allowed(&config, "zsl.example.com"));
        assert!(host_allowed(&config, "ZSL.Example.com:443"));
        assert!(host_allowed(&config, "127.0.0.1:3000"));
        assert!(host_allowed(&config, "localhost:3000"));
        assert!(!host_allowed(&config, "attacker.example"));
        assert!(!host_allowed(&config, ""));
    }

    #[test]
    fn tunnel_accepts_only_public_origin() {
        let config = config(DeployMode::Tunnel, Some("https://zsl.example.com"));

        assert!(origin_allowed(
            &config,
            "zsl.example.com",
            "https://zsl.example.com"
        ));
        assert!(!origin_allowed(
            &config,
            "zsl.example.com",
            "https://attacker.example"
        ));
        assert!(!origin_allowed(
            &config,
            "127.0.0.1:3000",
            "http://127.0.0.1:3000"
        ));
        assert!(!origin_allowed(&config, "zsl.example.com", "null"));
    }

    #[test]
    fn local_accepts_public_and_current_host() {
        let config = config(DeployMode::Local, Some("https://zsl.example.com"));

        assert!(origin_allowed(
            &config,
            "192.168.1.10:3000",
            "http://192.168.1.10:3000"
        ));
        assert!(origin_allowed(
            &config,
            "zsl.example.com",
            "https://zsl.example.com"
        ));
        assert!(origin_allowed(
            &config,
            "localhost:3000",
            "http://localhost:3000"
        ));
        assert!(!origin_allowed(
            &config,
            "192.168.1.10:3000",
            "http://attacker.example"
        ));
        assert!(!origin_allowed(&config, "192.168.1.10:3000", "null"));
    }

    #[test]
    fn local_does_not_restrict_host_header() {
        let config = config(DeployMode::Local, None);
        assert!(host_allowed(&config, "anything.local:3000"));
    }
}
