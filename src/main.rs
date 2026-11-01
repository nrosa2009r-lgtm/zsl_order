use axum::{
    Router,
    routing::{get, post},
};
use tower_http::services::{ServeDir, ServeFile};

use std::fs;

use crate::{
    config::{get_config_data, create_config_file},
    database::{create_order, get_all_orders, get_menu},
    security::{hash_password, veryfy_password},
};

mod config;
mod database;
mod security;

pub const CONF_PATH: &str = "config.json";

#[tokio::main]
async fn main() {
    match fs::exists(CONF_PATH) {
        Ok(true) => println!("Plik konfiguracyjny istneje"),
        Ok(false) => {create_config_file().expect("Nie udało się stworzyć pliku konfiguracyjnego")},
        Err(e) => println!("Błąd {}", e),
    }

    let shared_state = std::sync::Arc::new(database::AppState::new_empty());

    let app = Router::new()
        .route_service("/login", ServeFile::new("frontend/login.html"))
        .route("/menu", get(get_menu))
        .route("/order", post(create_order))
        .route("/orders", get(get_all_orders))
        .with_state(shared_state)
        .fallback_service(ServeDir::new("frontend").append_index_html_on_directories(true));

    let addr = match (
        get_config_data("config.json", "ip"),
        get_config_data("config.json", "port"),
    ) {
        (Ok(ip), Ok(port)) => format!("{}:{}", ip, port),
        _ => "0.0.0.0:3000".to_string(),
    };

    let listener = match tokio::net::TcpListener::bind(&addr).await {
        Ok(l) => l,
        Err(e) => {
            println!("Nie można użyć {addr} ({e}). Uruchamiam na 0.0.0.0:3000...");
            tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap()
        }
    };

    println!(
        "Serwer stołówki działa pod adresem: http://{}",
        listener.local_addr().unwrap()
    );

    axum::serve(listener, app).await.unwrap();
}
