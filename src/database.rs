use crate::CONF_PATH;
use crate::config::get_config_data;
use axum::{Json, extract::State, http::StatusCode};
use rand::distr::{Alphanumeric, SampleString};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use sqlx::error;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use std::error::Error;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

#[derive(Clone, Serialize)]
pub struct MenuItem {
    id: u32,
    name: String,
    price_pln: f64,
}

#[derive(Clone, Serialize)]
pub struct Order {
    id: Uuid,
    customer_name: String,
    items: Vec<MenuItem>,
    status: String,
}

pub struct AppState {
    menu: Vec<MenuItem>,
    orders: Mutex<Vec<Order>>,
}

// Dodaj to w pliku src/database.rs w bloku impl dla AppState
impl AppState {
    pub fn new_empty() -> Self {
        Self {
            menu: vec![], // Tutaj docelowo wrzucisz wczytane z bazy menu
            orders: std::sync::Mutex::new(vec![]),
        }
    }
}

pub async fn create_db() -> Result<SqlitePool, Box<dyn Error>> {
    let db_url = match get_config_data(CONF_PATH, "baza.db") {
        Ok(link) => link,
        _ => String::from("sqlite://baza.db"),
    };

    let max_conn = match get_config_data("config.json", "max_connections") {
        Ok(max) => max.trim().parse().unwrap_or(100),
        _ => 100,
    };

    let connect_options = SqliteConnectOptions::from_str(&db_url)?.create_if_missing(true);

    let pool = SqlitePoolOptions::new()
        .max_connections(max_conn)
        .connect_with(connect_options)
        .await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS users (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            password TEXT NOT NULL,
            login TEXT NOT NULL UNIQUE
        )",
    )
    .execute(&pool)
    .await?;

    Ok(pool)
}
