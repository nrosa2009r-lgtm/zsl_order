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

/// Migracje wbudowane w binarkę. `sqlx::migrate!` czyta katalog
/// `migrations/` w czasie kompilacji, więc wdrożenie nie wymaga obecności
/// plików SQL na serwerze. Wersje są zapisywane w `_sqlx_migrations`.
static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

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

    MIGRATOR.run(&pool).await?;

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
        ("cancel_window_minutes", "30"),
        ("pickup_limit_minutes", "120"),
        ("max_uncollected", "0"),
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

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn memory_pool() -> SqlitePool {
        SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("baza w pamięci")
    }

    /// Baza utworzona przez stary kod nie ma tabeli `_sqlx_migrations`.
    /// Migracja bazowa nie może usunąć danych, a 0002 musi uzupełnić
    /// `class_group` tylko dla kont zaimportowanych z CSV.
    #[tokio::test]
    async fn migrations_upgrade_legacy_database_without_data_loss() {
        let pool = memory_pool().await;

        sqlx::query(
            "CREATE TABLE users (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                login TEXT NOT NULL UNIQUE,
                password_hash TEXT NOT NULL,
                role TEXT NOT NULL,
                display_code TEXT NOT NULL UNIQUE,
                active INTEGER NOT NULL DEFAULT 1,
                must_change_password INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL,
                last_login_at INTEGER
            )",
        )
        .execute(&pool)
        .await
        .expect("stara tabela users");

        for (login, display_code) in [
            ("s-1a-abcdefghij", "UCZ-1A-AAAAA"),
            ("s-abcdefghij", "UCZ-BBBBB"),
        ] {
            sqlx::query(
                "INSERT INTO users (login, password_hash, role, display_code, created_at)
                 VALUES (?, 'hash', 'student', ?, 1)",
            )
            .bind(login)
            .bind(display_code)
            .execute(&pool)
            .await
            .expect("dane testowe");
        }

        MIGRATOR
            .run(&pool)
            .await
            .expect("migracja bazy starego typu");

        let rows: Vec<(String, Option<String>)> =
            sqlx::query_as("SELECT login, class_group FROM users ORDER BY id")
                .fetch_all(&pool)
                .await
                .expect("odczyt użytkowników");

        assert_eq!(rows.len(), 2, "migracja nie może usuwać danych");
        assert_eq!(rows[0].1.as_deref(), Some("1A"));
        assert_eq!(rows[1].1, None);

        let menus: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM menu_items")
            .fetch_one(&pool)
            .await
            .expect("nowa tabela menu_items");
        assert_eq!(menus, 0);

        // Ponowne uruchomienie nie może powtórzyć migracji.
        MIGRATOR.run(&pool).await.expect("migracje są idempotentne");
    }

    #[tokio::test]
    async fn migrations_create_full_schema_on_empty_database() {
        let pool = memory_pool().await;
        MIGRATOR.run(&pool).await.expect("migracja pustej bazy");

        let tables: Vec<String> =
            sqlx::query_scalar("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
                .fetch_all(&pool)
                .await
                .expect("lista tabel");

        for expected in [
            "users",
            "sessions",
            "menu_items",
            "orders",
            "order_items",
            "settings",
            "audit_log",
        ] {
            assert!(
                tables.iter().any(|name| name == expected),
                "brak tabeli {expected}"
            );
        }

        let applied: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations")
            .fetch_one(&pool)
            .await
            .expect("wpisy migracji");
        assert_eq!(applied, 3);

        let columns: Vec<String> =
            sqlx::query_scalar("SELECT name FROM pragma_table_info('users')")
                .fetch_all(&pool)
                .await
                .expect("kolumny users");
        assert!(columns.iter().any(|name| name == "class_group"));
    }
}
