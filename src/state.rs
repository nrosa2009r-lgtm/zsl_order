use crate::auth::LoginLimiter;
use crate::config::Config;
use sqlx::SqlitePool;

pub struct AppState {
    pub db: SqlitePool,
    pub cfg: Config,
    /// Hash używany do wyrównania czasu odpowiedzi, gdy login nie istnieje.
    pub dummy_hash: String,
    pub login_limiter: LoginLimiter,
    /// Sekret HMAC do podpisywania i weryfikacji tokenów JWT.
    pub jwt_secret: Vec<u8>,
}
