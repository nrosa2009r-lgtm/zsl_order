use crate::auth::{self, AuthUser};
use crate::client_ip::ClientIp;
use crate::db;
use crate::error::AppError;
use crate::security;
use crate::state::AppState;
use crate::util;
use axum::{
    Extension, Json,
    extract::State,
    http::{HeaderMap, header},
};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use std::net::IpAddr;
use std::sync::Arc;

#[derive(Deserialize)]
pub struct LoginRequest {
    pub login: String,
    pub password: String,
}

#[derive(Clone, Serialize)]
pub struct UserView {
    pub login: String,
    pub role: String,
    pub display_code: String,
    pub must_change_password: bool,
}

#[derive(Serialize)]
pub struct LoginResponse {
    pub user: UserView,
    pub csrf_token: String,
}

#[derive(Serialize)]
pub struct MeResponse {
    pub user: UserView,
    pub csrf_token: String,
}

#[derive(Serialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub token_type: &'static str,
    pub expires_in: i64,
    pub user: UserView,
}

#[derive(FromRow)]
struct LoginRow {
    id: i64,
    login: String,
    password_hash: String,
    role: String,
    display_code: String,
    active: bool,
    must_change_password: bool,
}

pub async fn hash_password_async(password: String) -> Result<String, AppError> {
    tokio::task::spawn_blocking(move || security::hash_password(&password))
        .await
        .map_err(|error| AppError::internal(format!("spawn_blocking: {error}")))?
        .map_err(AppError::from)
}

pub async fn verify_password_async(password: String, hash: String) -> bool {
    tokio::task::spawn_blocking(move || security::verify_password(&password, &hash))
        .await
        .unwrap_or(false)
}

/// Wspólna weryfikacja danych logowania dla sesji cookie i tokenu JWT.
/// Stosuje ten sam limiter, wyrównanie czasu i komunikaty błędów.
async fn authenticate(
    state: &AppState,
    ip: IpAddr,
    login: &str,
    password: &str,
) -> Result<LoginRow, AppError> {
    let login = login.trim().to_string();
    let limiter_login: String = login.chars().take(64).collect();

    if let Err(retry_after) = state.login_limiter.check(ip, &limiter_login) {
        tracing::warn!(login = %limiter_login, retry_after, "Logowanie zablokowane przez limiter");
        return Err(AppError::TooManyRequests {
            retry_after,
            message: "Zbyt wiele prób logowania. Spróbuj ponownie później.".to_string(),
        });
    }

    if login.is_empty() || login.len() > 64 || password.is_empty() || password.len() > 256 {
        state.login_limiter.failure(ip, &limiter_login);
        tracing::warn!(login = %limiter_login, "Nieudane logowanie: nieprawidłowe dane wejściowe");
        return Err(AppError::unauthorized("Nieprawidłowy login lub hasło."));
    }

    let row = sqlx::query_as::<_, LoginRow>(
        "SELECT id, login, password_hash, role, display_code, active, must_change_password
         FROM users WHERE login = ?",
    )
    .bind(&login)
    .fetch_optional(&state.db)
    .await?;

    let Some(row) = row else {
        // Ten sam koszt obliczeniowy co dla istniejącego konta.
        let _ = verify_password_async(password.to_string(), state.dummy_hash.clone()).await;
        state.login_limiter.failure(ip, &limiter_login);
        tracing::warn!(login = %limiter_login, "Nieudane logowanie: konto nie istnieje");
        return Err(AppError::unauthorized("Nieprawidłowy login lub hasło."));
    };

    let valid = verify_password_async(password.to_string(), row.password_hash.clone()).await;
    if !valid || !row.active {
        state.login_limiter.failure(ip, &limiter_login);
        tracing::warn!(login = %limiter_login, active = row.active, "Nieudane logowanie: błędne hasło lub konto nieaktywne");
        return Err(AppError::unauthorized("Nieprawidłowy login lub hasło."));
    }

    state.login_limiter.success(ip, &limiter_login);
    Ok(row)
}

pub async fn login(
    State(state): State<Arc<AppState>>,
    Extension(client): Extension<ClientIp>,
    Json(body): Json<LoginRequest>,
) -> Result<(HeaderMap, Json<LoginResponse>), AppError> {
    let row = authenticate(&state, client.0, &body.login, &body.password).await?;

    let token = security::random_hex(32);
    let csrf_token = security::random_hex(32);
    let token_hash = security::hash_token(&token);
    let now = util::now();
    let expires_at = now + state.cfg.session_hours.max(1) * 3600;

    sqlx::query("DELETE FROM sessions WHERE expires_at < ?")
        .bind(now)
        .execute(&state.db)
        .await?;

    sqlx::query(
        "INSERT INTO sessions
         (token_hash, user_id, csrf_token, created_at, expires_at, last_seen_at)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&token_hash)
    .bind(row.id)
    .bind(&csrf_token)
    .bind(now)
    .bind(expires_at)
    .bind(now)
    .execute(&state.db)
    .await?;

    sqlx::query("UPDATE users SET last_login_at = ? WHERE id = ?")
        .bind(now)
        .bind(row.id)
        .execute(&state.db)
        .await?;

    db::audit(
        &state.db,
        Some(row.id),
        "login",
        "Logowanie zakończone sukcesem",
    )
    .await;
    tracing::info!(user_id = row.id, role = %row.role, "Logowanie zakończone sukcesem");

    let mut headers = HeaderMap::new();
    headers.insert(
        header::SET_COOKIE,
        auth::session_cookie(&token, state.cfg.session_hours, state.cfg.secure_cookies)?,
    );

    Ok((
        headers,
        Json(LoginResponse {
            user: UserView {
                login: row.login,
                role: row.role,
                display_code: row.display_code,
                must_change_password: row.must_change_password,
            },
            csrf_token,
        }),
    ))
}

/// Wydaje krótkotrwały token JWT do uwierzytelniania API przez
/// `Authorization: Bearer`. Token jest powiązany z rekordem sesji, więc
/// można go natychmiast unieważnić (logout, zmiana hasła, dezaktywacja).
pub async fn token(
    State(state): State<Arc<AppState>>,
    Extension(client): Extension<ClientIp>,
    Json(body): Json<LoginRequest>,
) -> Result<Json<TokenResponse>, AppError> {
    let row = authenticate(&state, client.0, &body.login, &body.password).await?;

    let now = util::now();
    let expires_at = now + auth::JWT_TTL_SECONDS;
    let (access_token, jti) = auth::encode_access_token(
        &state.jwt_secret,
        row.id,
        &row.login,
        &row.role,
        &row.display_code,
        now,
        expires_at,
    )?;
    let token_hash = auth::jwt_session_hash(&jti);

    sqlx::query("DELETE FROM sessions WHERE expires_at < ?")
        .bind(now)
        .execute(&state.db)
        .await?;

    // Pusty CSRF jest celowy: token Bearer nie jest wysyłany automatycznie
    // przez przeglądarkę, więc CSRF nie ma zastosowania.
    sqlx::query(
        "INSERT INTO sessions
         (token_hash, user_id, csrf_token, created_at, expires_at, last_seen_at)
         VALUES (?, ?, '', ?, ?, ?)",
    )
    .bind(&token_hash)
    .bind(row.id)
    .bind(now)
    .bind(expires_at)
    .bind(now)
    .execute(&state.db)
    .await?;

    sqlx::query("UPDATE users SET last_login_at = ? WHERE id = ?")
        .bind(now)
        .bind(row.id)
        .execute(&state.db)
        .await?;

    db::audit(
        &state.db,
        Some(row.id),
        "token_issued",
        "Wydano token dostępu JWT",
    )
    .await;
    tracing::info!(user_id = row.id, role = %row.role, "Wydano token dostępu JWT");

    Ok(Json(TokenResponse {
        access_token,
        token_type: "Bearer",
        expires_in: auth::JWT_TTL_SECONDS,
        user: UserView {
            login: row.login,
            role: row.role,
            display_code: row.display_code,
            must_change_password: row.must_change_password,
        },
    }))
}

pub async fn me(Extension(user): Extension<AuthUser>) -> Json<MeResponse> {
    Json(MeResponse {
        user: UserView {
            login: user.login,
            role: user.role,
            display_code: user.display_code,
            must_change_password: user.must_change_password,
        },
        csrf_token: user.csrf_token,
    })
}

pub async fn logout(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
) -> Result<(HeaderMap, Json<serde_json::Value>), AppError> {
    sqlx::query("DELETE FROM sessions WHERE token_hash = ?")
        .bind(&user.session_hash)
        .execute(&state.db)
        .await?;

    db::audit(&state.db, Some(user.id), "logout", "Wylogowanie").await;

    let mut headers = HeaderMap::new();
    headers.insert(
        header::SET_COOKIE,
        auth::clear_session_cookie(state.cfg.secure_cookies)?,
    );

    Ok((headers, Json(serde_json::json!({ "ok": true }))))
}

#[derive(Deserialize)]
pub struct ChangePasswordRequest {
    pub current_password: String,
    pub new_password: String,
}

pub async fn change_password(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Json(body): Json<ChangePasswordRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    validate_new_password(&body.new_password, &body.current_password)?;

    let hash: Option<String> = sqlx::query_scalar("SELECT password_hash FROM users WHERE id = ?")
        .bind(user.id)
        .fetch_optional(&state.db)
        .await?;
    let hash = hash.ok_or_else(|| AppError::unauthorized("Sesja wygasła."))?;

    if !verify_password_async(body.current_password, hash).await {
        db::audit(
            &state.db,
            Some(user.id),
            "password_change_failed",
            "Błędne hasło bieżące",
        )
        .await;
        return Err(AppError::bad("Obecne hasło jest nieprawidłowe."));
    }

    let new_hash = hash_password_async(body.new_password).await?;

    sqlx::query("UPDATE users SET password_hash = ?, must_change_password = 0 WHERE id = ?")
        .bind(new_hash)
        .bind(user.id)
        .execute(&state.db)
        .await?;

    // Wylogowanie pozostałych urządzeń / sesji.
    sqlx::query("DELETE FROM sessions WHERE user_id = ? AND token_hash <> ?")
        .bind(user.id)
        .bind(&user.session_hash)
        .execute(&state.db)
        .await?;

    if user.role == "admin" {
        db::remove_admin_start_file();
    }

    db::audit(
        &state.db,
        Some(user.id),
        "password_changed",
        "Hasło zostało zmienione",
    )
    .await;
    tracing::info!(user_id = user.id, role = %user.role, "Hasło zostało zmienione");

    Ok(Json(serde_json::json!({ "ok": true })))
}

fn validate_new_password(new_password: &str, current_password: &str) -> Result<(), AppError> {
    let length = new_password.chars().count();
    if !(10..=128).contains(&length) {
        return Err(AppError::bad("Nowe hasło musi mieć od 10 do 128 znaków."));
    }
    if !new_password.chars().any(|c| c.is_ascii_alphabetic())
        || !new_password.chars().any(|c| c.is_ascii_digit())
    {
        return Err(AppError::bad(
            "Nowe hasło musi zawierać co najmniej jedną literę i jedną cyfrę.",
        ));
    }
    if new_password == current_password {
        return Err(AppError::bad("Nowe hasło musi różnić się od obecnego."));
    }
    Ok(())
}
