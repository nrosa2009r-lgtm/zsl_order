// SPDX-License-Identifier: MIT
use crate::auth::{LoginLimiter, PickupLimiter};
use crate::config::Config;
use sqlx::SqlitePool;
use std::sync::Mutex;
use std::time::Instant;

pub struct AppState {
    pub db: SqlitePool,
    pub cfg: Config,
    /// Hash używany do wyrównania czasu odpowiedzi, gdy login nie istnieje.
    pub dummy_hash: String,
    pub login_limiter: LoginLimiter,
    pub pickup_limiter: PickupLimiter,
    /// Sekret HMAC do podpisywania i weryfikacji tokenów JWT.
    pub jwt_secret: Vec<u8>,
    /// Czas ostatniego ostrzeżenia o braku `CF-Connecting-IP` w trybie tunnel.
    pub client_ip_warning: Mutex<Option<Instant>>,
}
