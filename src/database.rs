use crate::config::get_config_data;
use axum::{Json, extract::State, http::StatusCode};
use rand::distr::{Alphanumeric, SampleString};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use sqlx::error;
use sqlx::sqlite::SqlitePoolOptions;
use std::error::Error;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

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

#[derive(Clone, Serialize)]
pub struct MenuItem {
    id: u32,
    name: String,
    price_pln: f64,
}

#[derive(Deserialize)]
pub struct CreateOrderDto {
    customer_name: String,
    items_ids: Vec<u32>,
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

pub async fn get_menu(State(state): State<Arc<AppState>>) -> Json<Vec<MenuItem>> {
    Json(state.menu.clone())
}

pub async fn create_order(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<CreateOrderDto>,
) -> (StatusCode, Json<Order>) {
    let ordered_items: Vec<MenuItem> = state
        .menu
        .iter()
        .filter(|item| payload.items_ids.contains(&item.id))
        .cloned()
        .collect();

    let new_order = Order {
        id: Uuid::new_v4(),
        customer_name: payload.customer_name,
        items: ordered_items,
        status: "Przyjęte do realizacji (Płatność na miejscu)".to_string(),
    };

    state.orders.lock().unwrap().push(new_order.clone());

    (StatusCode::CREATED, Json(new_order))
}

pub async fn get_all_orders(State(state): State<Arc<AppState>>) -> Json<Vec<Order>> {
    Json(state.orders.lock().unwrap().clone())
}
