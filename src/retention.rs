use crate::config::Config;
use crate::util;
use chrono::{Duration as ChronoDuration, NaiveDate};
use sqlx::SqlitePool;

/// Rozmiar porcji, po której przerywamy transakcję. Dzięki temu nocne
/// czyszczenie nie blokuje długo jednego zapisu i nie tworzy ogromnego WAL.
pub const BATCH_SIZE: i64 = 1000;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Summary {
    pub orders: u64,
    pub sessions: u64,
    pub idempotency_keys: u64,
    pub audit_log: u64,
}

pub async fn cleanup(pool: &SqlitePool, config: &Config) -> Result<Summary, sqlx::Error> {
    let mut summary = Summary::default();
    let now = util::now();

    if config.retention_orders_days > 0 {
        let cutoff = day_cutoff(util::today(), config.retention_orders_days);
        summary.orders = execute_batches(
            pool,
            "DELETE FROM orders WHERE id IN (
                 SELECT id FROM orders WHERE created_at < ? ORDER BY created_at LIMIT 1000
             )",
            cutoff,
        )
        .await?;
    }

    if config.retention_idempotency_days > 0 {
        let cutoff = day_cutoff(util::today(), config.retention_idempotency_days);
        summary.idempotency_keys = execute_batches(
            pool,
            "UPDATE orders SET idempotency_key = NULL WHERE id IN (
                 SELECT id FROM orders
                 WHERE idempotency_key IS NOT NULL AND created_at < ?
                 ORDER BY created_at LIMIT 1000
             )",
            cutoff,
        )
        .await?;
    }

    // Wygasłe sesje nie są potrzebne do niczego: autoryzacja i tak
    // odrzuca je po `expires_at`. Czyszczenie ogranicza rozmiar bazy.
    summary.sessions = execute_batches(
        pool,
        "DELETE FROM sessions WHERE token_hash IN (
             SELECT token_hash FROM sessions WHERE expires_at < ? LIMIT 1000
         )",
        now,
    )
    .await?;

    if config.retention_audit_days > 0 {
        let cutoff = day_cutoff(util::today(), config.retention_audit_days);
        summary.audit_log = execute_batches(
            pool,
            "DELETE FROM audit_log WHERE id IN (
                 SELECT id FROM audit_log WHERE created_at < ?
                 ORDER BY created_at LIMIT 1000
             )",
            cutoff,
        )
        .await?;
    }

    Ok(summary)
}

/// Początek doby `days` dni przed `base` w strefie szkoły. Całe doby są
/// usuwane razem, więc raporty nie mieszają częściowo wyczyszczonego dnia.
fn day_cutoff(base: NaiveDate, days: i64) -> i64 {
    util::start_of_local_date(base - ChronoDuration::days(days.max(0)))
}

async fn execute_batches(pool: &SqlitePool, sql: &str, cutoff: i64) -> Result<u64, sqlx::Error> {
    let mut total = 0;
    loop {
        let affected = sqlx::query(sql)
            .bind(cutoff)
            .execute(pool)
            .await?
            .rows_affected();
        total += affected;
        if affected < BATCH_SIZE as u64 {
            break;
        }
        tokio::task::yield_now().await;
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::APP_TZ;
    use chrono::TimeZone;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn memory_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("baza w pamięci");
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .expect("klucze obce");
        pool
    }

    async fn migrated_pool() -> SqlitePool {
        let pool = memory_pool().await;
        crate::db::MIGRATOR
            .run(&pool)
            .await
            .expect("migracje testowe");
        pool
    }

    #[tokio::test]
    async fn cleanup_removes_only_expired_rows_and_keeps_history() {
        let pool = migrated_pool().await;
        let now = util::now();
        let old_order = util::start_of_local_date(util::today() - ChronoDuration::days(31)) + 3600;
        let recent_order =
            util::start_of_local_date(util::today() - ChronoDuration::days(10)) + 3600;
        let old_audit = util::start_of_local_date(util::today() - ChronoDuration::days(100)) + 60;

        sqlx::query(
            "INSERT INTO users (login, password_hash, role, display_code, created_at)
             VALUES ('s-test', 'hash', 'student', 'UCZ-TEST', ?)",
        )
        .bind(now)
        .execute(&pool)
        .await
        .unwrap();

        for (id, created_at, key) in [
            ("OLD1", old_order, None),
            ("OLD2", old_order + 60, None),
            ("OLD3", old_order + 120, None),
            ("RECENT", recent_order, Some("idem-old")),
        ] {
            sqlx::query(
                "INSERT INTO orders
                 (user_id, code, qr_token, payment_method, total_cents, created_at, updated_at, idempotency_key)
                 VALUES (1, ?, ?, 'cash', 100, ?, ?, ?)",
            )
            .bind(id)
            .bind(format!("qr-{id}"))
            .bind(created_at)
            .bind(created_at)
            .bind(key)
            .execute(&pool)
            .await
            .unwrap();
        }
        sqlx::query(
            "INSERT INTO order_items (order_id, name, price_cents, quantity)
             VALUES (1, 'Zupa', 100, 1)",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            "INSERT INTO sessions (token_hash, user_id, csrf_token, created_at, expires_at, last_seen_at)
             VALUES ('expired', 1, 'csrf', ?, ?, ?), ('active', 1, 'csrf', ?, ?, ?)",
        )
        .bind(now - 7200)
        .bind(now - 3600)
        .bind(now - 7200)
        .bind(now)
        .bind(now + 3600)
        .bind(now)
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            "INSERT INTO audit_log (actor_id, action, details, created_at)
             VALUES (NULL, 'old', '', ?), (NULL, 'new', '', ?)",
        )
        .bind(old_audit)
        .bind(now)
        .execute(&pool)
        .await
        .unwrap();

        let config = Config {
            retention_orders_days: 30,
            retention_idempotency_days: 7,
            retention_audit_days: 90,
            ..Config::default()
        };

        let summary = cleanup(&pool, &config).await.unwrap();
        assert_eq!(summary.orders, 3);
        assert_eq!(summary.idempotency_keys, 1);
        assert_eq!(summary.sessions, 1);
        assert_eq!(summary.audit_log, 1);

        let orders: Vec<String> = sqlx::query_scalar("SELECT code FROM orders ORDER BY code")
            .fetch_all(&pool)
            .await
            .unwrap();
        assert_eq!(orders, vec!["RECENT".to_string()]);

        let items: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM order_items")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(items, 0, "order_items musi zniknąć przez ON DELETE CASCADE");

        let key: Option<String> =
            sqlx::query_scalar("SELECT idempotency_key FROM orders WHERE code = 'RECENT'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(key, None, "stary klucz idempotencji musi zostać wyzerowany");

        let sessions: Vec<String> = sqlx::query_scalar("SELECT token_hash FROM sessions")
            .fetch_all(&pool)
            .await
            .unwrap();
        assert_eq!(sessions, vec!["active".to_string()]);

        let audit: Vec<String> = sqlx::query_scalar("SELECT action FROM audit_log")
            .fetch_all(&pool)
            .await
            .unwrap();
        assert_eq!(audit, vec!["new".to_string()]);
    }

    #[tokio::test]
    async fn disabled_retention_keeps_orders() {
        let pool = migrated_pool().await;
        let old = util::start_of_local_date(util::today() - ChronoDuration::days(400)) + 60;

        sqlx::query(
            "INSERT INTO users (login, password_hash, role, display_code, created_at)
             VALUES ('s-test', 'hash', 'student', 'UCZ-TEST', ?)",
        )
        .bind(util::now())
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO orders
             (user_id, code, qr_token, payment_method, total_cents, created_at, updated_at)
             VALUES (1, 'OLD', 'qr-old', 'cash', 100, ?, ?)",
        )
        .bind(old)
        .bind(old)
        .execute(&pool)
        .await
        .unwrap();

        let config = Config::default();
        let summary = cleanup(&pool, &config).await.unwrap();
        assert_eq!(summary.orders, 0);
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM orders")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 1, "RETENTION_ORDERS_DAYS=0 nie może usuwać zamówień");
    }

    #[test]
    fn cutoff_uses_whole_local_days() {
        let base = NaiveDate::from_ymd_opt(2026, 11, 11).unwrap();
        let days = day_cutoff(base, 30);
        assert_eq!(
            util::local_date(days),
            Some(NaiveDate::from_ymd_opt(2026, 10, 12).unwrap())
        );
        let expected = APP_TZ
            .with_ymd_and_hms(2026, 10, 12, 0, 0, 0)
            .unwrap()
            .timestamp();
        assert_eq!(days, expected, "cutoff musi wypadać o północy czasu szkoły");
    }

    #[tokio::test]
    async fn deletes_rows_in_batches() {
        let pool = memory_pool().await;
        sqlx::query("CREATE TABLE t (id INTEGER PRIMARY KEY, created_at INTEGER NOT NULL)")
            .execute(&pool)
            .await
            .unwrap();

        let mut transaction = pool.begin().await.unwrap();
        for id in 0..2500 {
            sqlx::query("INSERT INTO t (id, created_at) VALUES (?, ?)")
                .bind(id)
                .bind(id)
                .execute(&mut *transaction)
                .await
                .unwrap();
        }
        transaction.commit().await.unwrap();

        let deleted = execute_batches(
            &pool,
            "DELETE FROM t WHERE id IN (
                 SELECT id FROM t WHERE created_at < ? ORDER BY created_at LIMIT 1000
             )",
            1500,
        )
        .await
        .unwrap();

        assert_eq!(deleted, 1500);
        let remaining: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM t")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(remaining, 1000);
    }
}
