use ipnet::IpNet;
use serde_json::Value;
use std::error::Error;
use std::fs;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::Path;
use std::str::FromStr;

pub const DEFAULT_BIND_ADDR: &str = "0.0.0.0:3000";
pub const DEFAULT_TRUSTED_PROXIES: &str = "127.0.0.1,::1";

/// Sposób wdrożenia. Te same funkcje, inna konfiguracja granicy zaufania.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeployMode {
    Local,
    Tunnel,
}

impl DeployMode {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value.trim().to_ascii_lowercase().as_str() {
            "" | "local" => Ok(Self::Local),
            "tunnel" => Ok(Self::Tunnel),
            other => Err(format!(
                "Nieprawidłowy DEPLOY_MODE „{other}”. Dozwolone wartości: local, tunnel."
            )),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Tunnel => "tunnel",
        }
    }

    pub fn is_tunnel(self) -> bool {
        self == Self::Tunnel
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CookieSecureSetting {
    Auto,
    Always,
    Never,
}

impl CookieSecureSetting {
    fn parse(value: &str) -> Result<Self, String> {
        match value.trim().to_ascii_lowercase().as_str() {
            "" | "auto" => Ok(Self::Auto),
            "true" | "1" | "tak" | "yes" => Ok(Self::Always),
            "false" | "0" | "nie" | "no" => Ok(Self::Never),
            other => Err(format!(
                "Nieprawidłowy COOKIE_SECURE „{other}”. Dozwolone wartości: auto, true, false."
            )),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Always => "true",
            Self::Never => "false",
        }
    }
}

/// Zweryfikowany publiczny adres aplikacji. Przechowujemy znormalizowane
/// origin i nagłówek Host, aby porównania nie zależały od wielkości liter
/// ani od jawnego zapisu domyślnego portu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublicUrl {
    scheme: String,
    host_header: String,
    origin: String,
}

impl PublicUrl {
    pub fn parse(raw: &str) -> Result<Self, String> {
        let raw = raw.trim();
        if raw.is_empty() {
            return Err("PUBLIC_URL nie może być pusty.".to_string());
        }

        let (scheme, rest) = raw
            .split_once("://")
            .ok_or_else(|| "PUBLIC_URL musi zaczynać się od http:// lub https://.".to_string())?;
        let scheme = scheme.to_ascii_lowercase();
        if scheme != "http" && scheme != "https" {
            return Err("PUBLIC_URL może używać tylko schematu http lub https.".to_string());
        }

        let rest = rest.trim_end_matches('/');
        if rest.is_empty() {
            return Err("PUBLIC_URL nie zawiera nazwy hosta.".to_string());
        }
        if rest.contains(['/', '?', '#', '@']) {
            return Err(
                "PUBLIC_URL nie może zawierać ścieżki, zapytania, fragmentu ani danych logowania."
                    .to_string(),
            );
        }

        let (host, port) = parse_authority(rest)?;
        let port = port.filter(|value| !is_default_port(&scheme, *value));
        let host_header = format_host_header(&host, port);
        let origin = format!("{scheme}://{host_header}");

        Ok(Self {
            scheme,
            host_header,
            origin,
        })
    }

    pub fn origin(&self) -> &str {
        &self.origin
    }

    pub fn host_header(&self) -> &str {
        &self.host_header
    }

    pub fn is_https(&self) -> bool {
        self.scheme == "https"
    }
}

fn parse_authority(authority: &str) -> Result<(String, Option<u16>), String> {
    if let Some(rest) = authority.strip_prefix('[') {
        let end = rest
            .find(']')
            .ok_or_else(|| "Nieprawidłowy adres IPv6 w PUBLIC_URL.".to_string())?;
        let host = &rest[..end];
        let parsed = host
            .parse::<IpAddr>()
            .map_err(|_| "Nieprawidłowy adres IPv6 w PUBLIC_URL.".to_string())?;
        if !parsed.is_ipv6() {
            return Err("Adres w nawiasach musi być adresem IPv6.".to_string());
        }

        let after = &rest[end + 1..];
        let port = if after.is_empty() {
            None
        } else {
            let value = after
                .strip_prefix(':')
                .ok_or_else(|| "Nieprawidłowy port w PUBLIC_URL.".to_string())?;
            Some(parse_port(value)?)
        };

        return Ok((host.to_ascii_lowercase(), port));
    }

    if authority.contains(':') {
        let (host, port) = authority
            .rsplit_once(':')
            .ok_or_else(|| "Nieprawidłowy adres w PUBLIC_URL.".to_string())?;
        if host.is_empty() || host.contains(':') || port.is_empty() {
            return Err("Nieprawidłowy adres w PUBLIC_URL.".to_string());
        }
        if !valid_host(host) {
            return Err("Nieprawidłowa nazwa hosta w PUBLIC_URL.".to_string());
        }
        return Ok((host.to_ascii_lowercase(), Some(parse_port(port)?)));
    }

    if !valid_host(authority) {
        return Err("Nieprawidłowa nazwa hosta w PUBLIC_URL.".to_string());
    }

    Ok((authority.to_ascii_lowercase(), None))
}

fn parse_port(value: &str) -> Result<u16, String> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("Port w PUBLIC_URL musi być liczbą.".to_string());
    }

    value
        .parse::<u16>()
        .ok()
        .filter(|port| *port > 0)
        .ok_or_else(|| "Port w PUBLIC_URL musi być w zakresie 1–65535.".to_string())
}

fn valid_host(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 253
        && host
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_'))
}

fn is_default_port(scheme: &str, port: u16) -> bool {
    (scheme == "http" && port == 80) || (scheme == "https" && port == 443)
}

fn format_host_header(host: &str, port: Option<u16>) -> String {
    let shown_host = match host.parse::<IpAddr>() {
        Ok(ip) if ip.is_ipv6() => format!("[{host}]"),
        _ => host.to_string(),
    };

    match port {
        Some(port) => format!("{shown_host}:{port}"),
        None => shown_host,
    }
}

/// Normalizuje nagłówek `Host` lub autorytet z `Origin` do pary
/// `(host, port)`. Domyślne porty 80/443 są usuwane.
pub fn normalized_authority(value: &str) -> Option<(String, Option<u16>)> {
    let value = value.trim().trim_end_matches('/');
    if value.is_empty() {
        return None;
    }

    let (host, port) = parse_authority(value).ok()?;
    let port = port.filter(|port| *port != 80 && *port != 443);
    Some((host, port))
}

pub fn authority_eq(left: &str, right: &str) -> bool {
    match (normalized_authority(left), normalized_authority(right)) {
        (Some((left_host, left_port)), Some((right_host, right_port))) => {
            left_host == right_host && left_port == right_port
        }
        _ => false,
    }
}

pub fn is_loopback_authority(value: &str) -> bool {
    let Some((host, _port)) = normalized_authority(value) else {
        return false;
    };

    host == "localhost"
        || host
            .parse::<IpAddr>()
            .map(|ip| ip.is_loopback())
            .unwrap_or(false)
}

/// Konfiguracja serwera. Część eksploatacyjna pochodzi z `config.json`,
/// a ustawienia wdrożeniowe z `.env`. Zmienne środowiskowe mają
/// pierwszeństwo nad zgodnościowymi polami starego `config.json`.
#[derive(Clone, Debug)]
pub struct Config {
    pub db_url: String,
    pub max_connections: u32,
    pub session_hours: i64,
    pub login_max_failures: u32,
    pub login_ip_max_failures: u32,
    pub login_window_minutes: i64,
    pub login_lockout_minutes: i64,
    pub pickup_max_failures: u32,
    pub pickup_ip_max_failures: u32,
    pub pickup_window_minutes: i64,
    pub pickup_lockout_minutes: i64,
    pub mode: DeployMode,
    pub bind_addr: SocketAddr,
    pub trusted_proxies: Vec<IpNet>,
    pub public_url: Option<PublicUrl>,
    pub cookie_secure_setting: CookieSecureSetting,
    pub secure_cookies: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            db_url: "sqlite://baza.db".to_string(),
            max_connections: 16,
            session_hours: 12,
            login_max_failures: 10,
            login_ip_max_failures: 300,
            login_window_minutes: 10,
            login_lockout_minutes: 15,
            pickup_max_failures: 8,
            pickup_ip_max_failures: 100,
            pickup_window_minutes: 10,
            pickup_lockout_minutes: 15,
            mode: DeployMode::Local,
            bind_addr: SocketAddr::from_str(DEFAULT_BIND_ADDR)
                .expect("domyślny adres nasłuchu jest poprawny"),
            trusted_proxies: default_trusted_proxies(),
            public_url: None,
            cookie_secure_setting: CookieSecureSetting::Auto,
            secure_cookies: false,
        }
    }
}

impl Config {
    pub fn load_or_create(path: &str) -> Result<Self, Box<dyn Error>> {
        let _ = dotenvy::dotenv();

        let file = if Path::new(path).exists() {
            let text = fs::read_to_string(path)?;
            let value: Value = serde_json::from_str(&text)?;
            FileConfig::from_value(&value)
        } else {
            let config = FileConfig::default();
            fs::write(path, config.to_json()?)?;
            config
        };

        Self::from_file_and_env(file, EnvConfig::from_process())
    }

    /// Buduje konfigurację z jawnie podanych źródeł. Wydzielone z
    /// `load_or_create`, aby testy nie zależały od środowiska procesu.
    pub fn from_file_and_env(file: FileConfig, env: EnvConfig) -> Result<Self, Box<dyn Error>> {
        let mode = match env.deploy_mode.as_deref() {
            Some(value) => DeployMode::parse(value)?,
            None => DeployMode::Local,
        };

        let bind_addr = resolve_bind_addr(&file, &env, mode)?;
        if mode.is_tunnel() && !bind_addr.ip().is_loopback() {
            return Err(format!(
                "W trybie tunnel BIND_ADDR musi wskazywać interfejs loopback (127.0.0.1 lub ::1), a podano {bind_addr}."
            )
            .into());
        }

        let trusted_proxies = parse_trusted_proxies(
            env.trusted_proxies
                .as_deref()
                .unwrap_or(DEFAULT_TRUSTED_PROXIES),
        )?;

        let public_url = match env
            .public_url
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            Some(raw) => Some(PublicUrl::parse(raw)?),
            None => None,
        };
        if mode.is_tunnel() && public_url.is_none() {
            return Err(
                "W trybie tunnel wymagany jest PUBLIC_URL, np. https://zsl.example.com.".into(),
            );
        }

        let cookie_secure_setting = match env.cookie_secure.as_deref() {
            Some(value) => CookieSecureSetting::parse(value)?,
            None => CookieSecureSetting::Auto,
        };
        let secure_cookies = match mode {
            // Za tunelem aplikacja widzi zwykłe HTTP, ale przeglądarka
            // łączy się z Cloudflare po HTTPS.
            DeployMode::Tunnel => true,
            DeployMode::Local => match cookie_secure_setting {
                CookieSecureSetting::Always => true,
                CookieSecureSetting::Never => false,
                CookieSecureSetting::Auto => public_url
                    .as_ref()
                    .map(PublicUrl::is_https)
                    .or(file.legacy_secure_cookies)
                    .unwrap_or(false),
            },
        };

        Ok(Self {
            db_url: file.db_url,
            max_connections: file.max_connections,
            session_hours: file.session_hours,
            login_max_failures: file.login_max_failures,
            login_ip_max_failures: file.login_ip_max_failures,
            login_window_minutes: file.login_window_minutes,
            login_lockout_minutes: file.login_lockout_minutes,
            pickup_max_failures: file.pickup_max_failures,
            pickup_ip_max_failures: file.pickup_ip_max_failures,
            pickup_window_minutes: file.pickup_window_minutes,
            pickup_lockout_minutes: file.pickup_lockout_minutes,
            mode,
            bind_addr,
            trusted_proxies,
            public_url,
            cookie_secure_setting,
            secure_cookies,
        })
    }

    pub fn is_tunnel(&self) -> bool {
        self.mode.is_tunnel()
    }

    pub fn bind_address(&self) -> SocketAddr {
        self.bind_addr
    }

    /// SQLite wymaga adresu w formacie `sqlite://sciezka`. Naprawiamy też
    /// najczęstszy błąd starych konfiguracji (`baza.db` bez prefiksu).
    pub fn sqlite_url(&self) -> String {
        if self.db_url.starts_with("sqlite:") {
            self.db_url.clone()
        } else if Path::new(&self.db_url).exists()
            || self.db_url.ends_with(".db")
            || self.db_url.ends_with(".sqlite")
        {
            format!("sqlite://{}", self.db_url)
        } else {
            self.db_url.clone()
        }
    }
}

fn resolve_bind_addr(
    file: &FileConfig,
    env: &EnvConfig,
    mode: DeployMode,
) -> Result<SocketAddr, Box<dyn Error>> {
    if let Some(raw) = env.bind_addr.as_deref() {
        return SocketAddr::from_str(raw.trim())
            .map_err(|error| format!("Nieprawidłowy BIND_ADDR „{raw}”: {error}.").into());
    }

    let port = file.legacy_port.unwrap_or(3000);
    if mode.is_tunnel() {
        return Ok(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port));
    }

    let ip = match file.legacy_ip.as_deref() {
        Some(value) => value
            .trim()
            .parse::<IpAddr>()
            .map_err(|error| format!("Nieprawidłowy adres ip w config.json: {error}."))?,
        None => IpAddr::V4(Ipv4Addr::UNSPECIFIED),
    };

    Ok(SocketAddr::new(ip, port))
}

fn parse_trusted_proxies(raw: &str) -> Result<Vec<IpNet>, Box<dyn Error>> {
    let mut networks = Vec::new();

    for part in raw.split([',', ' ', '\t', '\n']).map(str::trim) {
        if part.is_empty() {
            continue;
        }

        if let Ok(network) = part.parse::<IpNet>() {
            networks.push(network);
        } else if let Ok(ip) = part.parse::<IpAddr>() {
            networks.push(IpNet::from(ip));
        } else {
            return Err(format!(
                "Nieprawidłowy wpis w TRUSTED_PROXIES: „{part}”. Użyj adresu IP lub CIDR."
            )
            .into());
        }
    }

    if networks.is_empty() {
        return Err("TRUSTED_PROXIES nie może być puste.".into());
    }

    Ok(networks)
}

fn default_trusted_proxies() -> Vec<IpNet> {
    parse_trusted_proxies(DEFAULT_TRUSTED_PROXIES)
        .expect("domyślna lista zaufanych proxy jest poprawna")
}

#[derive(Clone, Debug)]
pub struct FileConfig {
    db_url: String,
    max_connections: u32,
    session_hours: i64,
    login_max_failures: u32,
    login_ip_max_failures: u32,
    login_window_minutes: i64,
    login_lockout_minutes: i64,
    pickup_max_failures: u32,
    pickup_ip_max_failures: u32,
    pickup_window_minutes: i64,
    pickup_lockout_minutes: i64,
    legacy_ip: Option<String>,
    legacy_port: Option<u16>,
    legacy_secure_cookies: Option<bool>,
}

impl Default for FileConfig {
    fn default() -> Self {
        Self {
            db_url: "sqlite://baza.db".to_string(),
            max_connections: 16,
            session_hours: 12,
            login_max_failures: 10,
            login_ip_max_failures: 300,
            login_window_minutes: 10,
            login_lockout_minutes: 15,
            pickup_max_failures: 8,
            pickup_ip_max_failures: 100,
            pickup_window_minutes: 10,
            pickup_lockout_minutes: 15,
            legacy_ip: None,
            legacy_port: None,
            legacy_secure_cookies: None,
        }
    }
}

impl FileConfig {
    fn from_value(value: &Value) -> Self {
        let defaults = Self::default();
        let legacy_ip = value
            .get("ip")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let legacy_secure_cookies = value.get("secure_cookies").map(|_| {
            get_bool(
                value,
                "secure_cookies",
                defaults.legacy_secure_cookies.unwrap_or(false),
            )
        });

        Self {
            db_url: get_string(value, "db_url", &defaults.db_url),
            max_connections: get_u64(value, "max_connections")
                .and_then(|value| u32::try_from(value).ok())
                .filter(|value| *value > 0 && *value <= 128)
                .unwrap_or(defaults.max_connections),
            session_hours: get_u64(value, "session_hours")
                .and_then(|value| i64::try_from(value).ok())
                .filter(|value| (1..=24 * 30).contains(value))
                .unwrap_or(defaults.session_hours),
            login_max_failures: get_u32_bounded(
                value,
                "login_max_failures",
                defaults.login_max_failures,
                3,
                50,
            ),
            login_ip_max_failures: get_u32_bounded(
                value,
                "login_ip_max_failures",
                defaults.login_ip_max_failures,
                10,
                100_000,
            ),
            login_window_minutes: get_i64_bounded(
                value,
                "login_window_minutes",
                defaults.login_window_minutes,
                1,
                1440,
            ),
            login_lockout_minutes: get_i64_bounded(
                value,
                "login_lockout_minutes",
                defaults.login_lockout_minutes,
                1,
                1440,
            ),
            pickup_max_failures: get_u32_bounded(
                value,
                "pickup_max_failures",
                defaults.pickup_max_failures,
                3,
                50,
            ),
            pickup_ip_max_failures: get_u32_bounded(
                value,
                "pickup_ip_max_failures",
                defaults.pickup_ip_max_failures,
                10,
                100_000,
            ),
            pickup_window_minutes: get_i64_bounded(
                value,
                "pickup_window_minutes",
                defaults.pickup_window_minutes,
                1,
                1440,
            ),
            pickup_lockout_minutes: get_i64_bounded(
                value,
                "pickup_lockout_minutes",
                defaults.pickup_lockout_minutes,
                1,
                1440,
            ),
            legacy_ip,
            legacy_port: get_u64(value, "port")
                .and_then(|value| u16::try_from(value).ok())
                .filter(|value| *value > 0),
            legacy_secure_cookies,
        }
    }

    fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(&serde_json::json!({
            "db_url": self.db_url,
            "max_connections": self.max_connections,
            "session_hours": self.session_hours,
            "login_max_failures": self.login_max_failures,
            "login_ip_max_failures": self.login_ip_max_failures,
            "login_window_minutes": self.login_window_minutes,
            "login_lockout_minutes": self.login_lockout_minutes,
            "pickup_max_failures": self.pickup_max_failures,
            "pickup_ip_max_failures": self.pickup_ip_max_failures,
            "pickup_window_minutes": self.pickup_window_minutes,
            "pickup_lockout_minutes": self.pickup_lockout_minutes,
        }))
    }
}

#[derive(Clone, Debug, Default)]
pub struct EnvConfig {
    pub deploy_mode: Option<String>,
    pub bind_addr: Option<String>,
    pub trusted_proxies: Option<String>,
    pub public_url: Option<String>,
    pub cookie_secure: Option<String>,
}

impl EnvConfig {
    pub fn from_process() -> Self {
        Self {
            deploy_mode: env_value("DEPLOY_MODE"),
            bind_addr: env_value("BIND_ADDR"),
            trusted_proxies: env_value("TRUSTED_PROXIES"),
            public_url: env_value("PUBLIC_URL"),
            cookie_secure: env_value("COOKIE_SECURE"),
        }
    }
}

fn env_value(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn get_string(value: &Value, key: &str, default: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| default.to_string())
}

fn get_u64(value: &Value, key: &str) -> Option<u64> {
    let entry = value.get(key)?;
    entry
        .as_u64()
        .or_else(|| entry.as_str()?.trim().parse::<u64>().ok())
}

fn get_u32_bounded(value: &Value, key: &str, default: u32, min: u64, max: u64) -> u32 {
    get_u64(value, key)
        .filter(|number| (min..=max).contains(number))
        .and_then(|number| u32::try_from(number).ok())
        .unwrap_or(default)
}

fn get_i64_bounded(value: &Value, key: &str, default: i64, min: u64, max: u64) -> i64 {
    get_u64(value, key)
        .filter(|number| (min..=max).contains(number))
        .and_then(|number| i64::try_from(number).ok())
        .unwrap_or(default)
}

fn get_bool(value: &Value, key: &str, default: bool) -> bool {
    match value.get(key) {
        Some(Value::Bool(value)) => *value,
        Some(Value::String(value)) => matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "tak" | "yes"
        ),
        Some(Value::Number(value)) => value.as_u64().unwrap_or(0) != 0,
        _ => default,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqliteConnectOptions;

    fn file_config() -> FileConfig {
        FileConfig::default()
    }

    fn env_with(
        deploy_mode: &str,
        bind_addr: &str,
        public_url: Option<&str>,
        cookie_secure: &str,
    ) -> EnvConfig {
        EnvConfig {
            deploy_mode: Some(deploy_mode.to_string()),
            bind_addr: Some(bind_addr.to_string()),
            trusted_proxies: None,
            public_url: public_url.map(str::to_string),
            cookie_secure: Some(cookie_secure.to_string()),
        }
    }

    #[test]
    fn default_config_is_valid() {
        let config = Config::default();
        assert_eq!(config.mode, DeployMode::Local);
        assert_eq!(config.bind_addr.port(), 3000);
        assert!(config.public_url.is_none());
        assert!(!config.secure_cookies);
        assert!(SqliteConnectOptions::from_str(&config.sqlite_url()).is_ok());
    }

    #[test]
    fn parses_legacy_string_numbers() {
        let value: Value = serde_json::from_str(
            r#"{"ip":"127.0.0.1","port":"8080","max_connections":"20","db_url":"baza.db"}"#,
        )
        .unwrap();
        let file = FileConfig::from_value(&value);
        assert_eq!(file.legacy_ip.as_deref(), Some("127.0.0.1"));
        assert_eq!(file.legacy_port, Some(8080));
        assert_eq!(file.max_connections, 20);
    }

    #[test]
    fn env_overrides_legacy_config_json() {
        let mut file = file_config();
        file.legacy_ip = Some("127.0.0.1".to_string());
        file.legacy_port = Some(3000);

        let config =
            Config::from_file_and_env(file, env_with("local", "0.0.0.0:8080", None, "false"))
                .unwrap();

        assert_eq!(config.bind_addr.to_string(), "0.0.0.0:8080");
        assert!(!config.secure_cookies);
    }

    #[test]
    fn tunnel_forces_loopback_and_secure_cookies() {
        let config = Config::from_file_and_env(
            file_config(),
            env_with(
                "tunnel",
                "127.0.0.1:3001",
                Some("https://zsl.example.com"),
                "false",
            ),
        )
        .unwrap();

        assert!(config.is_tunnel());
        assert!(config.bind_addr.ip().is_loopback());
        assert!(config.secure_cookies);
    }

    #[test]
    fn tunnel_rejects_non_loopback_bind() {
        let error = Config::from_file_and_env(
            file_config(),
            env_with(
                "tunnel",
                "0.0.0.0:3001",
                Some("https://zsl.example.com"),
                "auto",
            ),
        )
        .unwrap_err();

        assert!(error.to_string().contains("loopback"));
    }

    #[test]
    fn tunnel_requires_public_url() {
        let error = Config::from_file_and_env(
            file_config(),
            env_with("tunnel", "127.0.0.1:3001", None, "auto"),
        )
        .unwrap_err();

        assert!(error.to_string().contains("PUBLIC_URL"));
    }

    #[test]
    fn trusted_proxies_accept_ips_and_cidrs() {
        let networks = parse_trusted_proxies("127.0.0.1, ::1, 10.0.0.0/8").unwrap();
        assert_eq!(networks.len(), 3);
        assert!(networks[0].contains(&"127.0.0.1".parse::<IpAddr>().unwrap()));
        assert!(networks[2].contains(&"10.1.2.3".parse::<IpAddr>().unwrap()));
    }

    #[test]
    fn public_url_normalizes_default_ports() {
        let url = PublicUrl::parse("https://ZSL.Example.com:443/").unwrap();
        assert_eq!(url.origin(), "https://zsl.example.com");
        assert_eq!(url.host_header(), "zsl.example.com");
        assert!(url.is_https());
        assert_eq!(
            PublicUrl::parse("https://zsl.example.com")
                .unwrap()
                .origin(),
            url.origin()
        );
    }

    #[test]
    fn public_url_rejects_paths_and_credentials() {
        assert!(PublicUrl::parse("https://example.com/app").is_err());
        assert!(PublicUrl::parse("https://user:pass@example.com").is_err());
        assert!(PublicUrl::parse("ftp://example.com").is_err());
    }

    #[test]
    fn authority_comparison_ignores_default_ports_and_case() {
        assert!(authority_eq("Example.com:443", "example.com"));
        assert!(authority_eq("[::1]:3000", "[::1]:3000"));
        assert!(!authority_eq("example.com:3000", "example.com"));
        assert!(is_loopback_authority("localhost:8080"));
        assert!(is_loopback_authority("[::1]:3000"));
        assert!(!is_loopback_authority("192.168.1.10:3000"));
    }

    #[test]
    fn local_cookie_secure_auto_follows_public_url_scheme() {
        let secure = Config::from_file_and_env(
            file_config(),
            env_with(
                "local",
                "0.0.0.0:3000",
                Some("https://zsl.example.com"),
                "auto",
            ),
        )
        .unwrap();
        assert!(secure.secure_cookies);

        let plain = Config::from_file_and_env(
            file_config(),
            env_with(
                "local",
                "0.0.0.0:3000",
                Some("http://192.168.1.10:3000"),
                "auto",
            ),
        )
        .unwrap();
        assert!(!plain.secure_cookies);
    }
}
