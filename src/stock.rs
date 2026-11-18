// SPDX-License-Identifier: MIT
//! Dzienny stan magazynowy pozycji menu.
//!
//! Stan jest kontrolowany wyłącznie w transakcji `BEGIN IMMEDIATE`
//! (`create_order`), więc dwie osoby nie mogą zamówić tej samej ostatniej
//! sztuki. Reset dobowy jest leniwy: `stock_default` wraca przy pierwszym
//! odczycie nowego dnia w `Europe/Warsaw`, bez zadania w tle.

use crate::util;
use chrono::NaiveDate;
use sqlx::{Sqlite, Transaction};
use std::collections::HashMap;

/// Data w formacie używanym w `menu_items.stock_date`.
pub fn date_key(date: NaiveDate) -> String {
    date.format("%Y-%m-%d").to_string()
}

/// Leniwy reset stanu na nowy dzień. Pozycje bez `stock_default` zachowują
/// stan ustawiony ręcznie; pozycje z domyślnym stanem wracają do niego.
/// Zwraca liczbę zaktualizowanych wierszy (0 = nic do zrobienia).
pub async fn reset_daily_stock<'e, E>(executor: E, today: NaiveDate) -> Result<u64, sqlx::Error>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
    let today_key = date_key(today);
    let result = sqlx::query(
        "UPDATE menu_items
         SET stock_remaining = stock_default, stock_date = ?, updated_at = ?
         WHERE stock_default IS NOT NULL
           AND stock_date IS NOT NULL
           AND stock_date <> ?",
    )
    .bind(&today_key)
    .bind(util::now())
    .bind(&today_key)
    .execute(executor)
    .await?;

    Ok(result.rows_affected())
}

/// Rezerwuje stan w transakcji zamówienia. Warunkowy `UPDATE` gwarantuje,
/// że stan nie spadnie poniżej zera; `rows_affected = 0` oznacza brak
/// wystarczającej liczby sztuk. Zwraca nazwy pozycji, których zabrakło.
///
/// Funkcja najpierw wykonuje leniwy reset dobowy, więc stan dotyczy
/// właściwego dnia także tuż po północy.
pub async fn reserve(
    transaction: &mut Transaction<'_, Sqlite>,
    quantities: &HashMap<i64, i64>,
    today: NaiveDate,
) -> Result<Vec<String>, sqlx::Error> {
    reset_daily_stock(&mut **transaction, today).await?;
    let now = util::now();
    let mut sold_out = Vec::new();

    for (item_id, quantity) in quantities {
        let result = sqlx::query(
            "UPDATE menu_items
             SET stock_remaining = CASE
                     WHEN stock_remaining IS NULL THEN NULL
                     ELSE stock_remaining - ?
                 END,
                 updated_at = ?
             WHERE id = ?
               AND (stock_remaining IS NULL OR stock_remaining >= ?)",
        )
        .bind(quantity)
        .bind(now)
        .bind(item_id)
        .bind(quantity)
        .execute(&mut **transaction)
        .await?;

        if result.rows_affected() == 0 {
            let name: Option<String> =
                sqlx::query_scalar("SELECT name FROM menu_items WHERE id = ?")
                    .bind(item_id)
                    .fetch_optional(&mut **transaction)
                    .await?;
            sold_out.push(name.unwrap_or_else(|| format!("pozycja id={item_id}")));
        }
    }

    Ok(sold_out)
}

/// Zwraca sztuki na stan po anulowaniu zamówienia. Działa tylko dla pozycji
/// limitowanych i tylko wtedy, gdy `stock_date` to ten sam dzień co data
/// zamówienia — anulowanie wczorajszego zamówienia nie zwiększa
/// dzisiejszego stanu.
pub async fn return_order_stock(
    transaction: &mut Transaction<'_, Sqlite>,
    order_id: i64,
    today: NaiveDate,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        "UPDATE menu_items
         SET stock_remaining = stock_remaining + COALESCE((
                 SELECT SUM(oi.quantity) FROM order_items oi
                 WHERE oi.order_id = ? AND oi.menu_item_id = menu_items.id
                   AND oi.stock_reserved = 1
             ), 0),
             updated_at = ?
         WHERE stock_remaining IS NOT NULL
           AND stock_date = ?
           AND EXISTS (
                 SELECT 1 FROM order_items oi
                 WHERE oi.order_id = ? AND oi.menu_item_id = menu_items.id
                   AND oi.stock_reserved = 1
             )",
    )
    .bind(order_id)
    .bind(util::now())
    .bind(date_key(today))
    .bind(order_id)
    .execute(&mut **transaction)
    .await?;

    Ok(result.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
    use sqlx::{Connection, SqlitePool};
    use std::path::PathBuf;
    use std::str::FromStr;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    async fn test_pool(name: &str) -> (SqlitePool, PathBuf) {
        let unique = COUNTER.fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!(
            "zsl-stock-{name}-{}-{unique}.db",
            std::process::id()
        ));
        let options = SqliteConnectOptions::from_str(&format!("sqlite://{}", path.display()))
            .unwrap()
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .busy_timeout(std::time::Duration::from_secs(10));
        let pool = SqlitePoolOptions::new()
            .max_connections(8)
            .connect_with(options)
            .await
            .unwrap();
        crate::db::MIGRATOR.run(&pool).await.unwrap();
        (pool, path)
    }

    async fn close(pool: SqlitePool, path: PathBuf) {
        pool.close().await;
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
        }
    }

    async fn insert_item(
        pool: &SqlitePool,
        name: &str,
        remaining: Option<i64>,
        default: Option<i64>,
        date: Option<&str>,
    ) -> i64 {
        sqlx::query_scalar(
            "INSERT INTO menu_items
             (name, description, price_cents, category, available, sort_order,
              created_at, updated_at, stock_remaining, stock_default, stock_date)
             VALUES (?, '', 500, 'Dania', 1, 0, 1, 1, ?, ?, ?)
             RETURNING id",
        )
        .bind(name)
        .bind(remaining)
        .bind(default)
        .bind(date)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    async fn insert_order_with_item(pool: &SqlitePool, item_id: i64, quantity: i64) -> i64 {
        let mut connection = pool.acquire().await.unwrap();
        let mut tx = connection.begin().await.unwrap();
        sqlx::query(
            "INSERT OR IGNORE INTO users (login, password_hash, role, display_code, active, must_change_password, created_at)
             VALUES ('s-test', 'x', 'student', 'UCZ-TEST', 1, 0, 1)",
        )
        .execute(&mut *tx)
        .await
        .unwrap();
        let user_id: i64 = sqlx::query_scalar("SELECT id FROM users WHERE login = 's-test'")
            .fetch_one(&mut *tx)
            .await
            .unwrap();
        let code = crate::security::random_hex(3).to_uppercase();
        let qr_token = crate::security::random_hex(8);
        let order_id: i64 = sqlx::query_scalar(
            "INSERT INTO orders (user_id, code, qr_token, status, payment_method, payment_status,
                                 total_cents, pickup_time, created_at, updated_at)
             VALUES (?, ?, ?, 'ordered', 'cash', 'unpaid', 500, '', 1, 1)
             RETURNING id",
        )
        .bind(user_id)
        .bind(&code)
        .bind(&qr_token)
        .fetch_one(&mut *tx)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO order_items (order_id, menu_item_id, name, price_cents, quantity, stock_reserved)
             VALUES (?, ?, 'Test', 500, ?, 1)",
        )
        .bind(order_id)
        .bind(item_id)
        .bind(quantity)
        .execute(&mut *tx)
        .await
        .unwrap();
        tx.commit().await.unwrap();
        order_id
    }

    async fn remaining(pool: &SqlitePool, id: i64) -> Option<i64> {
        sqlx::query_scalar("SELECT stock_remaining FROM menu_items WHERE id = ?")
            .bind(id)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn last_item_can_be_reserved_once() {
        let (pool, path) = test_pool("reserve").await;
        let id = insert_item(&pool, "Ostatnia", Some(1), None, None).await;
        let mut quantities = HashMap::new();
        quantities.insert(id, 1);

        {
            let mut connection = pool.acquire().await.unwrap();
            let mut tx = connection.begin().await.unwrap();
            let sold = reserve(&mut tx, &quantities, util::today()).await.unwrap();
            assert!(sold.is_empty());
            tx.commit().await.unwrap();
        }
        assert_eq!(remaining(&pool, id).await, Some(0));

        {
            let mut connection = pool.acquire().await.unwrap();
            let mut tx = connection.begin().await.unwrap();
            let sold = reserve(&mut tx, &quantities, util::today()).await.unwrap();
            assert_eq!(sold, vec!["Ostatnia".to_string()]);
            tx.rollback().await.unwrap();
        }
        assert_eq!(remaining(&pool, id).await, Some(0));

        close(pool, path).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn twenty_parallel_reservations_of_one_item_yield_one_winner() {
        let (pool, path) = test_pool("parallel20").await;
        let id = insert_item(&pool, "Ostatnia", Some(1), None, None).await;

        let mut handles = Vec::new();
        for _ in 0..20 {
            let pool = pool.clone();
            handles.push(tokio::spawn(async move {
                let mut quantities = HashMap::new();
                quantities.insert(id, 1);
                let mut connection = match pool.acquire().await {
                    Ok(value) => value,
                    Err(_) => return false,
                };
                let mut tx = match connection.begin_with("BEGIN IMMEDIATE").await {
                    Ok(value) => value,
                    Err(_) => return false,
                };
                match reserve(&mut tx, &quantities, util::today()).await {
                    Ok(sold) if sold.is_empty() => tx.commit().await.is_ok(),
                    _ => {
                        let _ = tx.rollback().await;
                        false
                    }
                }
            }));
        }

        let mut winners = 0;
        for handle in handles {
            if handle.await.unwrap() {
                winners += 1;
            }
        }
        assert_eq!(
            winners, 1,
            "dokładnie jedno zamówienie może wziąć ostatnią sztukę"
        );
        assert_eq!(remaining(&pool, id).await, Some(0));

        close(pool, path).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn two_parallel_reservations_of_one_item_yield_one_winner() {
        let (pool, path) = test_pool("parallel2").await;
        let id = insert_item(&pool, "Ostatnia", Some(1), None, None).await;

        let mut handles = Vec::new();
        for _ in 0..2 {
            let pool = pool.clone();
            handles.push(tokio::spawn(async move {
                let mut quantities = HashMap::new();
                quantities.insert(id, 1);
                let mut connection = pool.acquire().await.unwrap();
                let mut tx = connection.begin_with("BEGIN IMMEDIATE").await.unwrap();
                match reserve(&mut tx, &quantities, util::today()).await {
                    Ok(sold) if sold.is_empty() => tx.commit().await.is_ok(),
                    _ => {
                        let _ = tx.rollback().await;
                        false
                    }
                }
            }));
        }

        let mut winners = 0;
        for handle in handles {
            if handle.await.unwrap() {
                winners += 1;
            }
        }
        assert_eq!(
            winners, 1,
            "dokładnie jedno z dwóch zamówień bierze ostatnią sztukę"
        );
        assert_eq!(remaining(&pool, id).await, Some(0));
        close(pool, path).await;
    }

    #[tokio::test]
    async fn reset_uses_default_and_dst_dates_are_calendar_days() {
        let (pool, path) = test_pool("reset").await;
        let auto = insert_item(&pool, "Auto", Some(1), Some(10), Some("2026-03-28")).await;
        let manual = insert_item(&pool, "Ręczna", Some(7), None, Some("2026-03-28")).await;

        let changed = reset_daily_stock(&pool, NaiveDate::from_ymd_opt(2026, 3, 29).unwrap())
            .await
            .unwrap();
        assert_eq!(changed, 1);
        assert_eq!(remaining(&pool, auto).await, Some(10));
        assert_eq!(remaining(&pool, manual).await, Some(7));

        // Kolejny dzień zmiany czasu (25.10) działa tak samo: data to data.
        sqlx::query(
            "UPDATE menu_items SET stock_remaining = 3, stock_date = '2026-10-24' WHERE id = ?",
        )
        .bind(auto)
        .execute(&pool)
        .await
        .unwrap();
        let changed = reset_daily_stock(&pool, NaiveDate::from_ymd_opt(2026, 10, 25).unwrap())
            .await
            .unwrap();
        assert_eq!(changed, 1);
        assert_eq!(remaining(&pool, auto).await, Some(10));

        // Ten sam dzień nie resetuje ponownie.
        let changed = reset_daily_stock(&pool, NaiveDate::from_ymd_opt(2026, 10, 25).unwrap())
            .await
            .unwrap();
        assert_eq!(changed, 0);

        close(pool, path).await;
    }

    #[tokio::test]
    async fn cancel_returns_stock_only_for_the_same_day() {
        let (pool, path) = test_pool("return").await;
        let today = util::today();
        let today_key = date_key(today);
        let item = insert_item(&pool, "Zupa", Some(4), None, Some(&today_key)).await;
        let order_id = insert_order_with_item(&pool, item, 2).await;

        {
            let mut connection = pool.acquire().await.unwrap();
            let mut tx = connection.begin().await.unwrap();
            let changed = return_order_stock(&mut tx, order_id, today).await.unwrap();
            tx.commit().await.unwrap();
            assert_eq!(changed, 1);
        }
        assert_eq!(remaining(&pool, item).await, Some(6));

        // Stan z innego dnia nie przyjmuje zwrotu.
        sqlx::query(
            "UPDATE menu_items SET stock_remaining = 4, stock_date = '2000-01-01' WHERE id = ?",
        )
        .bind(item)
        .execute(&pool)
        .await
        .unwrap();
        {
            let mut connection = pool.acquire().await.unwrap();
            let mut tx = connection.begin().await.unwrap();
            let changed = return_order_stock(&mut tx, order_id, today).await.unwrap();
            tx.commit().await.unwrap();
            assert_eq!(changed, 0);
        }
        assert_eq!(remaining(&pool, item).await, Some(4));

        // Pozycja bez limitu nie dostaje stanu.
        let unlimited = insert_item(&pool, "Bez limitu", None, None, None).await;
        let order2 = insert_order_with_item(&pool, unlimited, 1).await;
        {
            let mut connection = pool.acquire().await.unwrap();
            let mut tx = connection.begin().await.unwrap();
            let changed = return_order_stock(&mut tx, order2, today).await.unwrap();
            tx.commit().await.unwrap();
            assert_eq!(changed, 0);
        }
        assert_eq!(remaining(&pool, unlimited).await, None);

        close(pool, path).await;
    }
}
