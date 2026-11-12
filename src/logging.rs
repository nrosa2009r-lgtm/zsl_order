use axum::{extract::Request, middleware::Next, response::Response};
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
/// nagłówków, ciasteczek, tokenów CSRF, ciał żądań ani adresów IP.
pub async fn request_log(request: Request, next: Next) -> Response {
    let method = request.method().clone();
    let path = request.uri().path().to_string();
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
                "Żądanie API"
            );
        } else if status >= 400 {
            tracing::warn!(
                event = "http_request",
                method = %method,
                path = %path,
                status,
                latency_ms,
                "Żądanie API"
            );
        } else {
            tracing::info!(
                event = "http_request",
                method = %method,
                path = %path,
                status,
                latency_ms,
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
            "Żądanie HTTP"
        );
    } else {
        tracing::debug!(
            event = "http_request",
            method = %method,
            path = %path,
            status,
            latency_ms,
            "Żądanie HTTP"
        );
    }

    response
}
