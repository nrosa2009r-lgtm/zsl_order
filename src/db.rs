use crate::config::Config;
use crate::security;
use crate::util;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::{Row, SqlitePool};
use std::error::Error;
use std::fs;
use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

const SCHEMA: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS users (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        login TEXT NOT NULL UNIQUE,
        password_hash TEXT NOT NULL,
        role TEXT NOT NULL CHECK (role IN ('student','kitchen','admin')),
        display_code TEXT NOT NULL UNIQUE,
        active INTEGER NOT NULL DEFAULT 1,
        must_change_password INTEGER NOT NULL DEFAULT 0,
        created_at INTEGER NOT NULL,
        last_login_at INTEGER
    )",
    "CREATE INDEX IF NOT EXISTS users_role_idx ON users(role)",
    "CREATE TABLE IF NOT EXISTS sessions (
        token_hash TEXT PRIMARY KEY,
        user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
        csrf_token TEXT NOT NULL,
        created_at INTEGER NOT NULL,
        expires_at INTEGER NOT NULL,
        last_seen_at INTEGER NOT NULL
    )",
    "CREATE INDEX IF NOT EXISTS sessions_user_idx ON sessions(user_id)",
    "CREATE INDEX IF NOT EXISTS sessions_expiry_idx ON sessions(expires_at)",
    "CREATE TABLE IF NOT EXISTS menu_items (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        name TEXT NOT NULL,
        description TEXT NOT NULL DEFAULT '',
        price_cents INTEGER NOT NULL CHECK (price_cents >= 0),
        category TEXT NOT NULL DEFAULT 'Dania',
        allergens TEXT NOT NULL DEFAULT '',
        available INTEGER NOT NULL DEFAULT 1,
        sort_order INTEGER NOT NULL DEFAULT 0,
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL
    )",
    "CREATE INDEX IF NOT EXISTS menu_available_idx ON menu_items(available, sort_order)",
    "CREATE TABLE IF NOT EXISTS orders (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
        code TEXT NOT NULL UNIQUE,
        qr_token TEXT NOT NULL UNIQUE,
        status TEXT NOT NULL DEFAULT 'new'
            CHECK (status IN ('new','preparing','ready','completed','cancelled')),
        payment_method TEXT NOT NULL CHECK (payment_method IN ('cash','card')),
        payment_status TEXT NOT NULL DEFAULT 'unpaid'
            CHECK (payment_status IN ('unpaid','paid')),
        total_cents INTEGER NOT NULL CHECK (total_cents >= 0),
        pickup_time TEXT NOT NULL DEFAULT '',
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL
    )",
    "CREATE INDEX IF NOT EXISTS orders_user_idx ON orders(user_id, created_at)",
    "CREATE INDEX IF NOT EXISTS orders_status_idx ON orders(status, created_at)",
    "CREATE TABLE IF NOT EXISTS order_items (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        order_id INTEGER NOT NULL REFERENCES orders(id) ON DELETE CASCADE,
        menu_item_id INTEGER REFERENCES menu_items(id) ON DELETE SET NULL,
        name TEXT NOT NULL,
        price_cents INTEGER NOT NULL CHECK (price_cents >= 0),
        quantity INTEGER NOT NULL CHECK (quantity > 0)
    )",
    "CREATE INDEX IF NOT EXISTS order_items_order_idx ON order_items(order_id)",
    "CREATE TABLE IF NOT EXISTS settings (
        key TEXT PRIMARY KEY,
        value TEXT NOT NULL
    )",
    "CREATE TABLE IF NOT EXISTS audit_log (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        actor_id INTEGER,
        action TEXT NOT NULL,
        details TEXT NOT NULL DEFAULT '',
        created_at INTEGER NOT NULL
    )",
    "CREATE INDEX IF NOT EXISTS audit_created_idx ON audit_log(created_at)",
];

pub async fn init(cfg: &Config) -> Result<SqlitePool, Box<dyn Error>> {
    let db_url = cfg.sqlite_url();
    let options = SqliteConnectOptions::from_str(&db_url)?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(Duration::from_secs(5))
        .foreign_keys(true);

    let pool = SqlitePoolOptions::new()
        .max_connections(cfg.max_connections)
        .min_connections(1)
        .acquire_timeout(Duration::from_secs(10))
        .connect_with(options)
        .await?;

    for statement in SCHEMA {
        sqlx::query(statement).execute(&pool).await?;
    }

    seed_settings(&pool).await?;
    seed_menu(&pool).await?;
    cleanup(&pool).await?;
    ensure_initial_admin(&pool).await?;

    Ok(pool)
}

async fn seed_settings(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    let defaults = [
        ("cafeteria_name", "Stołówka ZSŁ"),
        ("ordering_enabled", "1"),
        ("payment_info", "Płatność przy odbiorze: gotówka lub karta."),
        ("max_open_orders", "3"),
        ("max_order_items", "20"),
    ];

    for (key, value) in defaults {
        sqlx::query("INSERT OR IGNORE INTO settings (key, value) VALUES (?, ?)")
            .bind(key)
            .bind(value)
            .execute(pool)
            .await?;
    }

    Ok(())
}

async fn seed_menu(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM menu_items")
        .fetch_one(pool)
        .await?;
    if count > 0 {
        return Ok(());
    }

    let now = util::now();
    let items = [
        (
            "Zupa pomidorowa",
            "Klasyczna zupa z makaronem",
            550i64,
            "Zupy",
            "gluten, seler",
            10i64,
        ),
        (
            "Kotlet schabowy z ziemniakami",
            "Panierowany kotlet, ziemniaki, surówka",
            1400,
            "Dania główne",
            "gluten, jaja, mleko",
            20,
        ),
        (
            "Pierogi ruskie (8 szt.)",
            "Z cebulką i śmietaną",
            1100,
            "Dania główne",
            "gluten, mleko",
            30,
        ),
        (
            "Naleśniki z serem",
            "Z sosem owocowym",
            900,
            "Dania główne",
            "gluten, jaja, mleko",
            40,
        ),
        (
            "Sałatka warzywna",
            "Świeże warzywa z oliwą",
            800,
            "Dodatki",
            "",
            50,
        ),
        ("Kompot owocowy", "Szklanka 250 ml", 300, "Napoje", "", 60),
        ("Woda mineralna", "Butelka 0,5 l", 250, "Napoje", "", 70),
    ];

    for (name, description, price, category, allergens, sort_order) in items {
        sqlx::query(
            "INSERT INTO menu_items
             (name, description, price_cents, category, allergens, available, sort_order, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, 1, ?, ?, ?)",
        )
        .bind(name)
        .bind(description)
        .bind(price)
        .bind(category)
        .bind(allergens)
        .bind(sort_order)
        .bind(now)
        .bind(now)
        .execute(pool)
        .await?;
    }

    Ok(())
}

async fn cleanup(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    let now = util::now();
    sqlx::query("DELETE FROM sessions WHERE expires_at < ?")
        .bind(now)
        .execute(pool)
        .await?;
    // Dziennik zdarzeń nie jest danymi osobowymi, ale nie rośnie bez końca.
    sqlx::query("DELETE FROM audit_log WHERE created_at < ?")
        .bind(now - 90 * 24 * 60 * 60)
        .execute(pool)
        .await?;
    Ok(())
}

/// Tworzy pierwsze konto administratora tylko wtedy, gdy nie ma żadnego.
/// Dane startowe są losowe, wypisywane w konsoli i zapisywane do pliku
/// `admin_start.txt` z uprawnieniami 0600. Plik jest usuwany po zmianie hasła.
async fn ensure_initial_admin(pool: &SqlitePool) -> Result<(), Box<dyn Error>> {
    let admins: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE role = 'admin'")
        .fetch_one(pool)
        .await?;
    if admins > 0 {
        return Ok(());
    }

    let login = security::generate_login("admin");
    let password = security::random_password(18);
    let display_code = security::generate_display_code("ADM");
    let hash = security::hash_password(&password)
        .map_err(|error| format!("Nie udało się zahashować hasła: {error}"))?;
    let now = util::now();

    sqlx::query(
        "INSERT INTO users
         (login, password_hash, role, display_code, active, must_change_password, created_at)
         VALUES (?, ?, 'admin', ?, 1, 1, ?)",
    )
    .bind(&login)
    .bind(&hash)
    .bind(&display_code)
    .bind(now)
    .execute(pool)
    .await?;

    let content = format!(
        "==============================================\n\
         ZSL ORDER - STARTOWE KONTO ADMINISTRATORA\n\
         ==============================================\n\
         Login:  {login}\n\
         Hasło:  {password}\n\n\
         Po pierwszym zalogowaniu system wymusi zmianę hasła.\n\
         Po zmianie hasła ten plik zostanie automatycznie usunięty.\n\
         Nie wysyłaj tego pliku e-mailem i nie przechowuj go dłużej niż to konieczne.\n\
         ==============================================\n"
    );

    if let Ok(()) = fs::write("admin_start.txt", &content) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions("admin_start.txt", fs::Permissions::from_mode(0o600));
        }
    }

    let _ = sqlx::query(
        "INSERT INTO audit_log (actor_id, action, details, created_at)
         VALUES (NULL, 'admin_seeded', 'Utworzono startowe konto administratora', ?)",
    )
    .bind(now)
    .execute(pool)
    .await;

    println!("\n{content}");
    println!("Dane zapisano również w pliku admin_start.txt (tylko właściciel).\n");

    Ok(())
}

pub async fn get_setting(pool: &SqlitePool, key: &str) -> Option<String> {
    sqlx::query("SELECT value FROM settings WHERE key = ?")
        .bind(key)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .and_then(|row| row.try_get::<String, _>("value").ok())
}

pub async fn get_setting_or(pool: &SqlitePool, key: &str, default: &str) -> String {
    get_setting(pool, key)
        .await
        .unwrap_or_else(|| default.to_string())
}

pub async fn set_setting(pool: &SqlitePool, key: &str, value: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO settings (key, value) VALUES (?, ?)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(key)
    .bind(value)
    .execute(pool)
    .await?;
    Ok(())
}

/// Dziennik zdarzeń nie zawiera danych osobowych — tylko identyfikator
/// techniczny, rodzaj operacji i krótki opis.
pub async fn audit(pool: &SqlitePool, actor_id: Option<i64>, action: &str, details: &str) {
    let result = sqlx::query(
        "INSERT INTO audit_log (actor_id, action, details, created_at) VALUES (?, ?, ?, ?)",
    )
    .bind(actor_id)
    .bind(action)
    .bind(details)
    .bind(util::now())
    .execute(pool)
    .await;

    if let Err(error) = result {
        tracing::error!("Nie udało się zapisać zdarzenia audytu: {error}");
    }
}

/// Usuwa startowy plik z danymi administratora po zmianie hasła.
pub fn remove_admin_start_file() {
    if Path::new("admin_start.txt").exists() {
        let _ = fs::remove_file("admin_start.txt");
    }
}
