// SPDX-License-Identifier: MIT
use crate::client_ip::ClientIp;
use axum::{extract::Request, middleware::Next, response::Response};
use std::sync::OnceLock;
use std::time::Instant;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

/// Inicjalizuje logowanie strukturalne (JSON) na standardowe wyjście.
/// W systemd stdout trafia do journald, więc aplikacja nie tworzy ani nie
/// rotuje własnych plików logów. Poziom można nadpisać zmienną `RUST_LOG`,
/// np. `RUST_LOG=debug` lub `RUST_LOG=info,sqlx=warn`.
pub fn init() -> Result<(), Box<dyn std::error::Error>> {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,sqlx=warn,hyper=warn,axum=warn"));

    tracing_subscriber::registry()
        .with(filter)
        .with(
            tracing_subscriber::fmt::layer()
                .json()
                .with_target(false)
                .with_ansi(false)
                .with_current_span(false)
                .with_span_list(false)
                .with_writer(std::io::stdout),
        )
        .try_init()?;

    Ok(())
}

/// Loguje żądania HTTP jako JSON. Zapisywany jest wyłącznie czas, metoda,
/// ścieżka bez parametrów zapytania, status i czas odpowiedzi. Nie logujemy
/// nagłówków, ciasteczek, tokenów CSRF ani ciał żądań.
///
/// Adres klienta jest dołączany tylko po jawnym włączeniu `LOG_CLIENT_IP=1`
/// (domyślnie `-`). Służy do diagnozy „czy ruch z sieci lokalnej w ogóle
/// dociera do serwera” bez stałego przechowywania adresów IP.
pub async fn request_log(request: Request, next: Next) -> Response {
    let method = request.method().clone();
    let path = request.uri().path().to_string();
    let client_ip = logged_client_ip(&request);
    let started = Instant::now();

    let response = next.run(request).await;

    let status = response.status().as_u16();
    let latency_ms = started.elapsed().as_millis();

    if path.starts_with("/api/") {
        if status >= 500 {
            tracing::error!(
                event = "http_request",
                method = %method,
                path = %path,
                status,
                latency_ms,
                client_ip = %client_ip,
                "Żądanie API"
            );
        } else if status >= 400 {
            tracing::warn!(
                event = "http_request",
                method = %method,
                path = %path,
                status,
                latency_ms,
                client_ip = %client_ip,
                "Żądanie API"
            );
        } else {
            tracing::info!(
                event = "http_request",
                method = %method,
                path = %path,
                status,
                latency_ms,
                client_ip = %client_ip,
                "Żądanie API"
            );
        }
    } else if status >= 500 {
        tracing::error!(
            event = "http_request",
            method = %method,
            path = %path,
            status,
            latency_ms,
            client_ip = %client_ip,
            "Żądanie HTTP"
        );
    } else {
        tracing::debug!(
            event = "http_request",
            method = %method,
            path = %path,
            status,
            latency_ms,
            client_ip = %client_ip,
            "Żądanie HTTP"
        );
    }

    response
}

/// Czy do logów dopisywać adres klienta. Domyślnie nie — adres IP jest
/// danymi osobowymi i do codziennej pracy nie jest potrzebny.
fn log_client_ip_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| parse_flag(std::env::var("LOG_CLIENT_IP").ok().as_deref()))
}

fn parse_flag(value: Option<&str>) -> bool {
    matches!(
        value.map(|raw| raw.trim().to_ascii_lowercase()),
        Some(flag) if flag == "1" || flag == "true" || flag == "tak" || flag == "yes"
    )
}

/// Adres klienta do logu albo `-`, gdy diagnostyka jest wyłączona.
fn logged_client_ip(request: &Request) -> String {
    if !log_client_ip_enabled() {
        return "-".to_string();
    }

    request
        .extensions()
        .get::<ClientIp>()
        .map(|ip| ip.0.to_string())
        .unwrap_or_else(|| "-".to_string())
}

#[cfg(test)]
mod tests {
    use super::parse_flag;

    #[test]
    fn client_ip_logging_is_off_by_default() {
        assert!(!parse_flag(None));
        assert!(!parse_flag(Some("")));
        assert!(!parse_flag(Some("0")));
        assert!(!parse_flag(Some("nie")));
    }

    #[test]
    fn client_ip_logging_accepts_common_truthy_values() {
        for value in ["1", "true", "TRUE", " tak ", "yes"] {
            assert!(
                parse_flag(Some(value)),
                "wartość {value} powinna włączać log"
            );
        }
    }
}
