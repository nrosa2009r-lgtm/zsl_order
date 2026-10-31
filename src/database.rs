use crate::config::get_config_data;
use rand::distr::{Alphanumeric, SampleString};
use sqlx::SqlitePool;
use sqlx::error;
use sqlx::sqlite::SqlitePoolOptions;
use std::error::Error;

pub fn create_login() -> Result<String, error::Error> {
    let login = Alphanumeric.sample_string(&mut rand::rng(), 10);
    Ok(login)
}

#[derive(Debug, sqlx::FromRow)]
pub struct User {
    id: String,
    login: String,
    password: String,
}

pub async fn create_db() -> Result<SqlitePool, Box<dyn Error>> {
    let db_url = match get_config_data("config.json", "baza.db") {
        Ok(link) => link,
        _ => String::from("baza.db"),
    };

    let max_conn = match get_config_data("config.json", "max_connections") {
        Ok(max) => max.trim().parse().unwrap_or(100),
        _ => 100,
    };

    let pool = SqlitePoolOptions::new()
        .max_connections(max_conn)
        .connect(&db_url)
        .await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS uzytkownicy (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            nazwa TEXT NOT NULL,
            email TEXT NOT NULL UNIQUE
        )",
    )
    .execute(&pool)
    .await?;

    Ok(pool)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_login() {
        match create_login() {
            Ok(login) => println!("\n👉 Wygenerowany login: {}\n", login),
            Err(e) => panic!("Błąd: {:?}", e),
        }
    }
}
