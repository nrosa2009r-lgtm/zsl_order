// SPDX-License-Identifier: MIT
use crate::config::Config;
use crate::error::AppError;
use crate::security;
use crate::state::AppState;
use crate::util;
use axum::{
    extract::{Request, State},
    http::{HeaderMap, HeaderValue, Method, header},
    middleware::Next,
    response::Response,
};
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const SESSION_COOKIE: &str = "zsl_session";
pub const JWT_ISSUER: &str = "zsl-order";
pub const JWT_AUDIENCE: &str = "zsl-order-api";
/// Krótki czas życia tokenu dostępowego (15 minut). Odświeżanie odbywa się
/// przez ponowne uwierzytelnienie, a token można natychmiast unieważnić,
/// usuwając jego rekord sesji z bazy.
pub const JWT_TTL_SECONDS: i64 = 15 * 60;
const JWT_MAX_LENGTH: usize = 4096;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JwtClaims {
    pub sub: String,
    pub login: String,
    pub role: String,
    pub code: String,
    pub jti: String,
    pub iss: String,
    pub aud: String,
    pub iat: i64,
    pub exp: i64,
}

#[derive(Clone, Debug, Serialize)]
pub struct AuthUser {
    pub id: i64,
    pub login: String,
    pub role: String,
    pub display_code: String,
    pub csrf_token: String,
    pub session_hash: String,
    pub must_change_password: bool,
}

#[derive(FromRow)]
struct SessionRow {
    csrf_token: String,
    expires_at: i64,
    last_seen_at: i64,
    id: i64,
    login: String,
    role: String,
    display_code: String,
    active: bool,
    must_change_password: bool,
}

pub fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    let raw = headers.get(header::COOKIE)?.to_str().ok()?;
    for part in raw.split(';') {
        let mut parts = part.trim().splitn(2, '=');
        if parts.next() == Some(name) {
            return parts.next().map(|value| value.trim().to_string());
        }
    }
    None
}

pub fn session_cookie(token: &str, hours: i64, secure: bool) -> Result<HeaderValue, AppError> {
    let max_age = hours.max(1) * 3600;
    let mut value =
        format!("{SESSION_COOKIE}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age={max_age}");
    if secure {
        value.push_str("; Secure");
    }
    HeaderValue::from_str(&value)
        .map_err(|_| AppError::internal("Nie udało się ustawić ciasteczka"))
}

pub fn clear_session_cookie(secure: bool) -> Result<HeaderValue, AppError> {
    let mut value = format!("{SESSION_COOKIE}=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0");
    if secure {
        value.push_str("; Secure");
    }
    HeaderValue::from_str(&value).map_err(|_| AppError::internal("Nie udało się usunąć ciasteczka"))
}

/// Odczytuje token z nagłówka `Authorization: Bearer <token>`.
pub fn bearer_token(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, value) = raw.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("Bearer") {
        return None;
    }
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

/// Tworzy podpisany token JWT. Zwraca także `jti`, który jest zapisywany
/// w tabeli sesji — dzięki temu token można natychmiast unieważnić.
pub fn encode_access_token(
    secret: &[u8],
    user_id: i64,
    login: &str,
    role: &str,
    display_code: &str,
    now: i64,
    expires_at: i64,
) -> Result<(String, String), AppError> {
    let jti = security::random_hex(16);
    let claims = JwtClaims {
        sub: user_id.to_string(),
        login: login.to_string(),
        role: role.to_string(),
        code: display_code.to_string(),
        jti: jti.clone(),
        iss: JWT_ISSUER.to_string(),
        aud: JWT_AUDIENCE.to_string(),
        iat: now,
        exp: expires_at,
    };

    let token = encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(secret),
    )
    .map_err(|error| AppError::internal(format!("Nie udało się podpisać tokenu JWT: {error}")))?;

    Ok((token, jti))
}

/// Weryfikuje podpis, algorytm, wystawcę, odbiorcę i czas ważności tokenu.
/// Nie ufa żadnym danym z tokenu do momentu poprawnej weryfikacji.
pub fn decode_access_token(secret: &[u8], token: &str) -> Result<JwtClaims, AppError> {
    if token.len() > JWT_MAX_LENGTH {
        return Err(AppError::unauthorized(
            "Nieprawidłowy lub wygasły token dostępu.",
        ));
    }

    let mut validation = Validation::new(Algorithm::HS256);
    validation.set_issuer(&[JWT_ISSUER]);
    validation.set_audience(&[JWT_AUDIENCE]);
    validation.leeway = 30;

    decode::<JwtClaims>(token, &DecodingKey::from_secret(secret), &validation)
        .map(|data| data.claims)
        .map_err(|error| {
            tracing::warn!("Odrzucono token JWT: {error}");
            AppError::unauthorized("Nieprawidłowy lub wygasły token dostępu.")
        })
}

/// Tokeny JWT są powiązane z rekordem sesji po hashu `jti`, więc logout,
/// zmiana hasła i dezaktywacja konta działają natychmiast.
pub fn jwt_session_hash(jti: &str) -> String {
    security::hash_token(&format!("jwt:{jti}"))
}

fn normalized_api_path(path: &str) -> &str {
    path.strip_prefix("/api").unwrap_or(path)
}

async fn load_session_row(
    state: &AppState,
    token_hash: &str,
) -> Result<Option<SessionRow>, AppError> {
    Ok(sqlx::query_as::<_, SessionRow>(
        "SELECT s.csrf_token, s.expires_at, s.last_seen_at,
                u.id, u.login, u.role, u.display_code, u.active, u.must_change_password
         FROM sessions s
         JOIN users u ON u.id = s.user_id
         WHERE s.token_hash = ?",
    )
    .bind(token_hash)
    .fetch_optional(&state.db)
    .await?)
}

async fn invalidate_session(state: &AppState, token_hash: &str) {
    let _ = sqlx::query("DELETE FROM sessions WHERE token_hash = ?")
        .bind(token_hash)
        .execute(&state.db)
        .await;
}

/// Blokuje całe API poza zmianą hasła, `me` i wylogowaniem, dopóki konto
/// nie zmieni hasła startowego.
fn check_password_change_window(path: &str, must_change_password: bool) -> Result<(), AppError> {
    if !must_change_password {
        return Ok(());
    }

    let path = normalized_api_path(path);
    let allowed = path == "/auth/change-password" || path == "/auth/me" || path == "/auth/logout";
    if allowed {
        Ok(())
    } else {
        Err(AppError::PasswordChangeRequired)
    }
}

async fn touch_session(state: &AppState, token_hash: &str, now: i64, last_seen_at: i64) {
    // Odświeżanie czasu aktywności maksymalnie raz na 5 minut (mniej zapisów).
    if now.saturating_sub(last_seen_at) > 300 {
        let _ = sqlx::query("UPDATE sessions SET last_seen_at = ? WHERE token_hash = ?")
            .bind(now)
            .bind(token_hash)
            .execute(&state.db)
            .await;
    }
}

async fn authenticate_bearer(
    state: &AppState,
    token: &str,
    path: &str,
) -> Result<AuthUser, AppError> {
    let claims = decode_access_token(&state.jwt_secret, token)?;
    let token_hash = jwt_session_hash(&claims.jti);

    let Some(row) = load_session_row(state, &token_hash).await? else {
        return Err(AppError::unauthorized(
            "Token został unieważniony. Zaloguj się ponownie.",
        ));
    };

    let now = util::now();
    if row.expires_at <= now || !row.active {
        invalidate_session(state, &token_hash).await;
        return Err(AppError::unauthorized(
            "Token wygasł. Zaloguj się ponownie.",
        ));
    }

    // Token musi nadal odpowiadać aktualnemu stanowi konta. Zmiana roli,
    // loginu lub dezaktywacja unieważnia wcześniejsze tokeny.
    if claims.sub != row.id.to_string()
        || claims.login != row.login
        || claims.role != row.role
        || claims.code != row.display_code
    {
        invalidate_session(state, &token_hash).await;
        return Err(AppError::unauthorized(
            "Token został unieważniony. Zaloguj się ponownie.",
        ));
    }

    check_password_change_window(path, row.must_change_password)?;
    touch_session(state, &token_hash, now, row.last_seen_at).await;

    Ok(AuthUser {
        id: row.id,
        login: row.login,
        role: row.role,
        display_code: row.display_code,
        // Bearer nie jest wysyłany automatycznie przez przeglądarkę,
        // więc CSRF nie dotyczy tej ścieżki.
        csrf_token: String::new(),
        session_hash: token_hash,
        must_change_password: row.must_change_password,
    })
}

/// Ładowanie sesji do rozszerzeń requestu + weryfikacja CSRF + blokada
/// konta z hasłem startowym. Obsługuje dwa warianty:
///
/// - ciasteczko `zsl_session` (przeglądarka, token nieprzezroczysty),
/// - `Authorization: Bearer <JWT>` (klienci API, token krótkotrwały).
///
/// W obu wariantach w bazie istnieje rekord sesji, więc wylogowanie,
/// zmiana hasła i dezaktywacja konta unieważniają token natychmiast.
pub async fn auth_middleware(
    State(state): State<Arc<AppState>>,
    mut request: Request,
    next: Next,
) -> Result<Response, AppError> {
    let path = request.uri().path().to_string();

    if request.headers().contains_key(header::AUTHORIZATION) {
        let token = bearer_token(request.headers()).ok_or_else(|| {
            AppError::unauthorized(
                "Nieprawidłowy nagłówek Authorization. Wymagany format: Bearer <token>.",
            )
        })?;
        let user = authenticate_bearer(&state, &token, &path).await?;
        request.extensions_mut().insert(user);
        return Ok(next.run(request).await);
    }

    let token = cookie_value(request.headers(), SESSION_COOKIE)
        .ok_or_else(|| AppError::unauthorized("Wymagane zalogowanie."))?;

    if token.len() != 64 || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(AppError::unauthorized("Nieprawidłowa sesja."));
    }

    let token_hash = security::hash_token(&token);
    let now = util::now();

    let Some(row) = load_session_row(&state, &token_hash).await? else {
        return Err(AppError::unauthorized(
            "Sesja wygasła. Zaloguj się ponownie.",
        ));
    };

    if row.expires_at <= now || !row.active {
        invalidate_session(&state, &token_hash).await;
        return Err(AppError::unauthorized(
            "Sesja wygasła. Zaloguj się ponownie.",
        ));
    }

    if !is_safe_method(request.method()) {
        let provided = request
            .headers()
            .get("x-csrf-token")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("");
        if !security::constant_time_eq(provided.as_bytes(), row.csrf_token.as_bytes()) {
            return Err(AppError::forbidden("Nieprawidłowy token bezpieczeństwa."));
        }
    }

    check_password_change_window(&path, row.must_change_password)?;
    touch_session(&state, &token_hash, now, row.last_seen_at).await;

    request.extensions_mut().insert(AuthUser {
        id: row.id,
        login: row.login,
        role: row.role,
        display_code: row.display_code,
        csrf_token: row.csrf_token,
        session_hash: token_hash,
        must_change_password: row.must_change_password,
    });

    Ok(next.run(request).await)
}

fn is_safe_method(method: &Method) -> bool {
    matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS)
}

async fn role_guard(request: Request, next: Next, allowed: &[&str]) -> Result<Response, AppError> {
    let user = request
        .extensions()
        .get::<AuthUser>()
        .cloned()
        .ok_or_else(|| AppError::unauthorized("Wymagane zalogowanie."))?;

    if !allowed.iter().any(|role| *role == user.role) {
        return Err(AppError::forbidden("Brak uprawnień do tej operacji."));
    }

    Ok(next.run(request).await)
}

pub async fn require_student(request: Request, next: Next) -> Result<Response, AppError> {
    role_guard(request, next, &["student"]).await
}

pub async fn require_staff(request: Request, next: Next) -> Result<Response, AppError> {
    role_guard(request, next, &["kitchen", "admin"]).await
}

pub async fn require_admin(request: Request, next: Next) -> Result<Response, AppError> {
    role_guard(request, next, &["admin"]).await
}

/// Górny limit liczby kluczy w mapie limitera. Chroni pamięć procesu przed
/// nieograniczonym wzrostem przy wielu unikalnych loginach/adresach.
/// Po przekroczeniu usuwane są wpisy wygasłe, a jeśli wszystkie są aktywne —
/// najstarsze. Dzięki temu mapa nigdy nie rośnie bez ograniczeń.
const MAX_TRACKED_KEYS: usize = 10_000;

struct Attempt {
    failures: u32,
    window_start: Instant,
    locked_until: Option<Instant>,
}

/// Ogólny limiter nieudanych prób. `primary` to zwykle konto (niższy limit),
/// a `secondary` adres IP (wyższy limit, bo sieć szkolna współdzieli jeden
/// publiczny adres). Wpisy są pseudonimowe i trzymane wyłącznie w pamięci.
pub struct AttemptLimiter {
    inner: Mutex<HashMap<String, Attempt>>,
    primary_max_failures: u32,
    secondary_max_failures: u32,
    window: Duration,
    lockout: Duration,
}

impl AttemptLimiter {
    pub fn new(
        primary_max_failures: u32,
        secondary_max_failures: u32,
        window_minutes: i64,
        lockout_minutes: i64,
    ) -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            primary_max_failures: primary_max_failures.max(1),
            secondary_max_failures: secondary_max_failures.max(1),
            window: Duration::from_secs(window_minutes.max(1) as u64 * 60),
            lockout: Duration::from_secs(lockout_minutes.max(1) as u64 * 60),
        }
    }

    /// Usuwa wygasłe wpisy, a gdy mapa nadal jest pełna — najstarsze.
    /// Wywoływane tylko po osiągnięciu `MAX_TRACKED_KEYS`, więc normalne
    /// logowanie nie płaci kosztu skanowania całej mapy.
    fn prune(map: &mut HashMap<String, Attempt>, now: Instant, window: Duration) {
        if map.len() < MAX_TRACKED_KEYS {
            return;
        }

        map.retain(|_, attempt| {
            attempt.locked_until.is_some_and(|until| until > now)
                || now.saturating_duration_since(attempt.window_start) < window
        });

        if map.len() >= MAX_TRACKED_KEYS {
            let mut oldest = map
                .iter()
                .map(|(key, attempt)| (attempt.window_start, key.clone()))
                .collect::<Vec<_>>();
            oldest.sort_unstable_by_key(|(start, _)| *start);
            let excess = map.len() - MAX_TRACKED_KEYS + 1;
            for (_, key) in oldest.into_iter().take(excess) {
                map.remove(&key);
            }
        }
    }

    pub fn check(&self, primary: &str, secondary: &str) -> Result<(), u64> {
        let now = Instant::now();
        let mut map = self
            .inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());

        Self::prune(&mut map, now, self.window);

        let mut retry_after = 0u64;
        for key in [primary, secondary] {
            if let Some(attempt) = map.get(key) {
                if let Some(until) = attempt.locked_until
                    && until > now
                {
                    retry_after =
                        retry_after.max(until.saturating_duration_since(now).as_secs().max(1));
                    continue;
                }
                if now.saturating_duration_since(attempt.window_start) >= self.window {
                    map.remove(key);
                }
            }
        }

        if retry_after > 0 {
            Err(retry_after)
        } else {
            Ok(())
        }
    }

    pub fn failure(&self, primary: &str, secondary: &str) {
        self.failure_for_key(primary, self.primary_max_failures);
        self.failure_for_key(secondary, self.secondary_max_failures);
    }

    fn failure_for_key(&self, key: &str, max_failures: u32) {
        let now = Instant::now();
        let mut map = self
            .inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        Self::prune(&mut map, now, self.window);
        let attempt = map.entry(key.to_string()).or_insert(Attempt {
            failures: 0,
            window_start: now,
            locked_until: None,
        });

        if now.saturating_duration_since(attempt.window_start) >= self.window {
            attempt.failures = 0;
            attempt.window_start = now;
            attempt.locked_until = None;
        }

        attempt.failures = attempt.failures.saturating_add(1);
        if attempt.failures >= max_failures {
            attempt.locked_until = Some(now + self.lockout);
        }
    }

    pub fn success(&self, primary: &str, secondary: &str) {
        let mut map = self
            .inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        map.remove(primary);
        map.remove(secondary);
    }
}

/// Limiter logowania: limit **konto + IP** (żeby obca osoba z innej sieci nie
/// mogła zablokować admina/kuchni) oraz wysoki limit per IP.
pub struct LoginLimiter {
    limiter: AttemptLimiter,
}

impl LoginLimiter {
    pub fn from_config(config: &Config) -> Self {
        Self {
            limiter: AttemptLimiter::new(
                config.login_max_failures,
                config.login_ip_max_failures,
                config.login_window_minutes,
                config.login_lockout_minutes,
            ),
        }
    }

    fn ip_key(ip: IpAddr) -> String {
        format!("ip:{ip}")
    }

    /// Klucz blokady to para konto + IP. Atakujący z innego adresu nie
    /// zablokuje konta prawdziwemu użytkownikowi; chroni to konta
    /// administracyjne i kuchenne przed celowym „lockoutem”.
    fn login_key(login: &str, ip: IpAddr) -> String {
        format!("login:{}@{ip}", login.to_ascii_lowercase())
    }

    pub fn check(&self, ip: IpAddr, login: &str) -> Result<(), u64> {
        self.limiter
            .check(&Self::login_key(login, ip), &Self::ip_key(ip))
    }

    pub fn failure(&self, ip: IpAddr, login: &str) {
        self.limiter
            .failure(&Self::login_key(login, ip), &Self::ip_key(ip));
    }

    pub fn success(&self, ip: IpAddr, login: &str) {
        self.limiter
            .success(&Self::login_key(login, ip), &Self::ip_key(ip));
    }
}

impl Default for LoginLimiter {
    fn default() -> Self {
        Self {
            limiter: AttemptLimiter::new(10, 300, 10, 15),
        }
    }
}

/// Limiter odczytu kodu odbioru: główny limit per konto personelu,
/// wysoki limit per IP.
pub struct PickupLimiter {
    limiter: AttemptLimiter,
}

impl PickupLimiter {
    pub fn from_config(config: &Config) -> Self {
        Self {
            limiter: AttemptLimiter::new(
                config.pickup_max_failures,
                config.pickup_ip_max_failures,
                config.pickup_window_minutes,
                config.pickup_lockout_minutes,
            ),
        }
    }

    fn user_key(user_id: i64) -> String {
        format!("pickup-user:{user_id}")
    }

    fn ip_key(ip: IpAddr) -> String {
        format!("pickup-ip:{ip}")
    }

    pub fn check(&self, user_id: i64, ip: IpAddr) -> Result<(), u64> {
        self.limiter
            .check(&Self::user_key(user_id), &Self::ip_key(ip))
    }

    pub fn failure(&self, user_id: i64, ip: IpAddr) {
        self.limiter
            .failure(&Self::user_key(user_id), &Self::ip_key(ip));
    }

    pub fn success(&self, user_id: i64, ip: IpAddr) {
        self.limiter
            .success(&Self::user_key(user_id), &Self::ip_key(ip));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jwt_roundtrip_and_claims() {
        let secret = b"test-secret-test-secret-test-secret";
        let now = util::now();
        let (token, jti) =
            encode_access_token(secret, 7, "s-test", "student", "UCZ-TEST1", now, now + 600)
                .unwrap();
        let claims = decode_access_token(secret, &token).unwrap();
        assert_eq!(claims.sub, "7");
        assert_eq!(claims.login, "s-test");
        assert_eq!(claims.role, "student");
        assert_eq!(claims.code, "UCZ-TEST1");
        assert_eq!(claims.jti, jti);
        assert_eq!(claims.iss, JWT_ISSUER);
        assert_eq!(claims.aud, JWT_AUDIENCE);
    }

    #[test]
    fn jwt_rejects_wrong_secret_and_expired_token() {
        let secret = b"test-secret-test-secret-test-secret";
        let other = b"other-secret-other-secret-other-secret";
        let now = util::now();

        let (token, _) =
            encode_access_token(secret, 7, "s-test", "student", "UCZ-TEST1", now, now + 600)
                .unwrap();
        assert!(decode_access_token(other, &token).is_err());

        let (expired, _) = encode_access_token(
            secret,
            7,
            "s-test",
            "student",
            "UCZ-TEST1",
            now - 3600,
            now - 1800,
        )
        .unwrap();
        assert!(decode_access_token(secret, &expired).is_err());
    }

    #[test]
    fn bearer_header_parsing() {
        let mut headers = HeaderMap::new();
        assert_eq!(bearer_token(&headers), None);

        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer abc.def"),
        );
        assert_eq!(bearer_token(&headers).as_deref(), Some("abc.def"));

        headers.insert(header::AUTHORIZATION, HeaderValue::from_static("Basic abc"));
        assert_eq!(bearer_token(&headers), None);
    }

    #[test]
    fn login_limiter_locks_account_before_shared_ip() {
        let limiter = LoginLimiter {
            limiter: AttemptLimiter::new(3, 100, 10, 15),
        };
        let ip: IpAddr = "198.51.100.10".parse().unwrap();

        limiter.failure(ip, "s-uczen");
        limiter.failure(ip, "s-uczen");
        assert!(limiter.check(ip, "s-uczen").is_ok());
        limiter.failure(ip, "s-uczen");

        assert!(limiter.check(ip, "s-uczen").is_err());
        // Inny uczeń z tego samego (szkolnego) adresu IP nadal może się logować.
        assert!(limiter.check(ip, "s-inny").is_ok());

        limiter.success(ip, "s-uczen");
        assert!(limiter.check(ip, "s-uczen").is_ok());
    }

    #[test]
    fn login_limiter_cannot_lock_account_from_another_ip() {
        let limiter = LoginLimiter {
            limiter: AttemptLimiter::new(3, 100, 10, 15),
        };
        let attacker: IpAddr = "203.0.113.66".parse().unwrap();
        let victim: IpAddr = "192.0.2.10".parse().unwrap();

        for _ in 0..5 {
            limiter.failure(attacker, "a-admin");
        }
        // Atakujący blokuje tylko swoją parę konto+IP.
        assert!(limiter.check(attacker, "a-admin").is_err());
        // Prawdziwy administrator z innej sieci nadal może się zalogować.
        assert!(limiter.check(victim, "a-admin").is_ok());
    }

    #[test]
    fn attempt_limiter_prunes_and_caps_the_map() {
        let limiter = AttemptLimiter::new(3, 5, 10, 15);

        for index in 0..(MAX_TRACKED_KEYS + 50) {
            limiter.failure(&format!("konto-{index}"), "wspolny-ip");
        }

        let map = limiter
            .inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        assert!(
            map.len() <= MAX_TRACKED_KEYS,
            "mapa przekroczyła górny limit: {}",
            map.len()
        );
    }

    #[test]
    fn login_limiter_has_high_but_finite_ip_limit() {
        let limiter = LoginLimiter {
            limiter: AttemptLimiter::new(10, 3, 10, 15),
        };
        let ip: IpAddr = "198.51.100.20".parse().unwrap();

        for index in 0..3 {
            limiter.failure(ip, &format!("s-{index}"));
        }
        assert!(limiter.check(ip, "s-nowy").is_err());
    }

    #[test]
    fn pickup_limiter_is_independent_per_staff_account() {
        let limiter = PickupLimiter {
            limiter: AttemptLimiter::new(2, 100, 10, 15),
        };
        let ip: IpAddr = "198.51.100.30".parse().unwrap();

        limiter.failure(7, ip);
        limiter.failure(7, ip);
        assert!(limiter.check(7, ip).is_err());
        assert!(limiter.check(8, ip).is_ok());
    }
}
