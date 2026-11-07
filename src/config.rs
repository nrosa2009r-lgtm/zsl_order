use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::Path;

/// Konfiguracja serwera. Plik `config.json` tworzony jest automatycznie przy
/// pierwszym uruchomieniu. Nie zawiera żadnych danych osobowych ani sekretów.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    pub ip: String,
    pub port: u16,
    pub max_connections: u32,
    pub db_url: String,
    pub session_hours: i64,
    pub secure_cookies: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            ip: "0.0.0.0".to_string(),
            port: 3000,
            max_connections: 16,
            db_url: "sqlite://baza.db".to_string(),
            session_hours: 12,
            secure_cookies: false,
        }
    }
}

impl Config {
    pub fn load_or_create(path: &str) -> Result<Self, Box<dyn std::error::Error>> {
        if !Path::new(path).exists() {
            let cfg = Self::default();
            fs::write(path, serde_json::to_string_pretty(&cfg)?)?;
            return Ok(cfg);
        }

        let text = fs::read_to_string(path)?;
        let value: Value = serde_json::from_str(&text)?;

        // Odporne czytanie: brakujące/nieprawidłowe pola dostają bezpieczne
        // wartości domyślne, dzięki czemu stary plik config.json nie wywala
        // aplikacji.
        let defaults = Self::default();
        let cfg = Self {
            ip: get_string(&value, "ip", &defaults.ip),
            port: get_u64(&value, "port")
                .and_then(|v| u16::try_from(v).ok())
                .filter(|v| *v > 0)
                .unwrap_or(defaults.port),
            max_connections: get_u64(&value, "max_connections")
                .and_then(|v| u32::try_from(v).ok())
                .filter(|v| *v > 0 && *v <= 128)
                .unwrap_or(defaults.max_connections),
            db_url: get_string(&value, "db_url", &defaults.db_url),
            session_hours: get_u64(&value, "session_hours")
                .and_then(|v| i64::try_from(v).ok())
                .filter(|v| *v >= 1 && *v <= 24 * 30)
                .unwrap_or(defaults.session_hours),
            secure_cookies: get_bool(&value, "secure_cookies", defaults.secure_cookies),
        };

        // Uzupełnia plik o nowe pola, jeśli pochodzi ze starszej wersji.
        if let Ok(normalized) = serde_json::to_string_pretty(&cfg) {
            let _ = fs::write(path, normalized);
        }

        Ok(cfg)
    }

    pub fn bind_address(&self) -> String {
        format!("{}:{}", self.ip, self.port)
    }

    /// SQLite wymaga adresu w formacie `sqlite://sciezka`. Naprawiamy też
    /// najczęstszy błąd starych konfiguracji (`baza.db` bez prefiksu).
    pub fn sqlite_url(&self) -> String {
        if self.db_url.starts_with("sqlite:") {
            self.db_url.clone()
        } else if std::path::Path::new(&self.db_url).exists()
            || self.db_url.ends_with(".db")
            || self.db_url.ends_with(".sqlite")
        {
            format!("sqlite://{}", self.db_url)
        } else {
            self.db_url.clone()
        }
    }
}

fn get_string(value: &Value, key: &str, default: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| default.to_string())
}

fn get_u64(value: &Value, key: &str) -> Option<u64> {
    let entry = value.get(key)?;
    entry
        .as_u64()
        .or_else(|| entry.as_str()?.trim().parse::<u64>().ok())
}

fn get_bool(value: &Value, key: &str, default: bool) -> bool {
    match value.get(key) {
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) => matches!(
            s.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "tak" | "yes"
        ),
        Some(Value::Number(n)) => n.as_u64().unwrap_or(0) != 0,
        _ => default,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqliteConnectOptions;
    use std::str::FromStr;

    #[test]
    fn default_config_is_valid() {
        let cfg = Config::default();
        assert_eq!(cfg.port, 3000);
        assert!(SqliteConnectOptions::from_str(&cfg.sqlite_url()).is_ok());
    }

    #[test]
    fn parses_legacy_string_numbers() {
        let value: Value = serde_json::from_str(
            r#"{"ip":"127.0.0.1","port":"8080","max_connections":"20","db_url":"baza.db"}"#,
        )
        .unwrap();
        assert_eq!(get_u64(&value, "port"), Some(8080));
        assert_eq!(get_u64(&value, "max_connections"), Some(20));
    }
}
