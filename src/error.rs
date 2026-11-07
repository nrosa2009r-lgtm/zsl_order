use axum::{
    Json,
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde_json::json;

/// Wspólny typ błędów API. Odpowiedzi nie ujawniają szczegółów wewnętrznych
/// (ścieżek, SQL, stack trace), tylko czytelny komunikat dla użytkownika.
#[derive(Debug)]
pub enum AppError {
    BadRequest(String),
    Unauthorized(String),
    Forbidden(String),
    NotFound(String),
    Conflict(String),
    TooManyRequests { retry_after: u64, message: String },
    PasswordChangeRequired,
    Internal,
}

impl AppError {
    pub fn bad(message: impl Into<String>) -> Self {
        Self::BadRequest(message.into())
    }

    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::Unauthorized(message.into())
    }

    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::Forbidden(message.into())
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::NotFound(message.into())
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self::Conflict(message.into())
    }

    pub fn internal<E: std::fmt::Display>(error: E) -> Self {
        tracing::error!("Błąd wewnętrzny: {error}");
        Self::Internal
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, code, message, retry_after) = match self {
            Self::BadRequest(message) => (StatusCode::BAD_REQUEST, "bad_request", message, None),
            Self::Unauthorized(message) => {
                (StatusCode::UNAUTHORIZED, "unauthorized", message, None)
            }
            Self::Forbidden(message) => (StatusCode::FORBIDDEN, "forbidden", message, None),
            Self::NotFound(message) => (StatusCode::NOT_FOUND, "not_found", message, None),
            Self::Conflict(message) => (StatusCode::CONFLICT, "conflict", message, None),
            Self::TooManyRequests {
                retry_after,
                message,
            } => (
                StatusCode::TOO_MANY_REQUESTS,
                "too_many_requests",
                message,
                Some(retry_after),
            ),
            Self::PasswordChangeRequired => (
                StatusCode::FORBIDDEN,
                "password_change_required",
                "Najpierw zmień hasło startowe.".to_string(),
                None,
            ),
            Self::Internal => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "Wystąpił błąd serwera. Spróbuj ponownie.".to_string(),
                None,
            ),
        };

        let body = Json(json!({ "error": message, "code": code }));
        let mut response = (status, body).into_response();
        let headers = response.headers_mut();
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json; charset=utf-8"),
        );
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        if let Some(seconds) = retry_after {
            if let Ok(value) = HeaderValue::from_str(&seconds.to_string()) {
                headers.insert(header::RETRY_AFTER, value);
            }
        }
        response
    }
}

impl From<sqlx::Error> for AppError {
    fn from(error: sqlx::Error) -> Self {
        Self::internal(format!("SQLite: {error}"))
    }
}

impl From<argon2::password_hash::Error> for AppError {
    fn from(error: argon2::password_hash::Error) -> Self {
        Self::internal(format!("Argon2: {error}"))
    }
}

impl From<std::io::Error> for AppError {
    fn from(error: std::io::Error) -> Self {
        Self::internal(format!("IO: {error}"))
    }
}
