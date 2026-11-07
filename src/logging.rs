use axum::{extract::Request, middleware::Next, response::Response};
use std::time::Instant;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

/// Strażnik wątku zapisującego logi. Musi żyć przez cały czas działania
/// aplikacji — po jego upuszczeniu bufor logów może nie zostać opróżniony.
pub struct LogGuard {
    _file_guard: tracing_appender::non_blocking::WorkerGuard,
}

/// Inicjalizuje logowanie na standardowe wyjście oraz do plików w katalogu
/// `logs/`. Pliki są rotowane codziennie i mają nazwy
/// `logs/zsl_order.log.RRRR-MM-DD`.
///
/// Poziom można nadpisać zmienną środowiskową `RUST_LOG`, np.
/// `RUST_LOG=debug` lub `RUST_LOG=info,sqlx=warn`.
pub fn init() -> Result<LogGuard, Box<dyn std::error::Error>> {
    std::fs::create_dir_all("logs")?;

    let file_appender = tracing_appender::rolling::daily("logs", "zsl_order.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,sqlx=warn,hyper=warn,axum=warn"));

    tracing_subscriber::registry()
        .with(filter)
        .with(
            tracing_subscriber::fmt::layer()
                .with_target(false)
                .with_writer(std::io::stdout),
        )
        .with(
            tracing_subscriber::fmt::layer()
                .with_target(false)
                .with_ansi(false)
                .with_writer(non_blocking),
        )
        .init();

    Ok(LogGuard { _file_guard: guard })
}

/// Loguje żądania HTTP. Zapisywany jest wyłącznie czas, metoda, ścieżka bez
/// parametrów zapytania, status i czas odpowiedzi. Nie logujemy nagłówków,
/// ciasteczek, tokenów CSRF, ciał żądań ani adresów IP.
pub async fn request_log(request: Request, next: Next) -> Response {
    let method = request.method().clone();
    let path = request.uri().path().to_string();
    let started = Instant::now();

    let response = next.run(request).await;

    let status = response.status().as_u16();
    let latency_ms = started.elapsed().as_millis();

    if path.starts_with("/api/") {
        if status >= 500 {
            tracing::error!(method = %method, path = %path, status, latency_ms, "Żądanie API");
        } else if status >= 400 {
            tracing::warn!(method = %method, path = %path, status, latency_ms, "Żądanie API");
        } else {
            tracing::info!(method = %method, path = %path, status, latency_ms, "Żądanie API");
        }
    } else if status >= 500 {
        tracing::error!(method = %method, path = %path, status, latency_ms, "Żądanie HTTP");
    } else {
        tracing::debug!(method = %method, path = %path, status, latency_ms, "Żądanie HTTP");
    }

    response
}
