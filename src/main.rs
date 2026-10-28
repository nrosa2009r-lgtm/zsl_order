use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::Html,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs::File;
use std::sync::{Arc, Mutex};
use std::{
    fs::{self},
    io,
};
use uuid::Uuid;

#[derive(Clone, Serialize)]
struct MenuItem {
    id: u32,
    name: String,
    price_pln: f64,
}

#[derive(Deserialize)]
struct CreateOrderDto {
    customer_name: String,
    items_ids: Vec<u32>,
}

#[derive(Clone, Serialize)]
struct Order {
    id: Uuid,
    customer_name: String,
    items: Vec<MenuItem>,
    status: String,
}

struct AppState {
    menu: Vec<MenuItem>,
    orders: Mutex<Vec<Order>>,
}

#[tokio::main]
async fn main() {
    write_config_parm("config.json", "status", "working").unwrap();

    let shared_state = Arc::new(AppState {
        menu: vec![
            MenuItem {
                id: 1,
                name: "Zupa Pomidorowa z Makaronem".to_string(),
                price_pln: 8.5,
            },
            MenuItem {
                id: 2,
                name: "Kotlet Schabowy z Ziemniakami".to_string(),
                price_pln: 18.0,
            },
            MenuItem {
                id: 3,
                name: "Naleśniki z Serem i Dżemem".to_string(),
                price_pln: 14.0,
            },
            MenuItem {
                id: 4,
                name: "Kompot Owocowy".to_string(),
                price_pln: 3.0,
            },
        ],
        orders: Mutex::new(Vec::new()),
    });

    // Definicja routingu z obsługą strony głównej (/)
    let app = Router::new()
        .route("/", get(serve_index)) // Wysyła plik HTML
        .route("/menu", get(get_menu))
        .route("/order", post(create_order))
        .route("/orders", get(get_all_orders))
        .with_state(shared_state);

    // Nasłuchiwanie na porcie 80 (standardowy port HTTP) lub 3000
    let addr = "0.0.0.0:80";
    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(_) => {
            println!("Brak uprawnień do portu 80. Uruchamiam na port 3000...");
            tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap()
        }
    };

    println!(
        "Serwer stołówki działa pod adresem: http://{}",
        listener.local_addr().unwrap()
    );
    axum::serve(listener, app).await.unwrap();
}

// Endpoint serwujący interfejs HTML
async fn serve_index() -> Html<&'static str> {
    Html(include_str!("../frontend/index.html"))
}

async fn get_menu(State(state): State<Arc<AppState>>) -> Json<Vec<MenuItem>> {
    Json(state.menu.clone())
}

async fn create_order(
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

    let mut orders = state.orders.lock().unwrap();
    orders.push(new_order.clone());

    (StatusCode::CREATED, Json(new_order))
}

async fn get_all_orders(State(state): State<Arc<AppState>>) -> Json<Vec<Order>> {
    let orders = state.orders.lock().unwrap();
    Json(orders.clone())
}

fn write_config_parm(
    path: &str,
    parm: &str,
    value: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let data = fs::read_to_string(path)?;

    let mut json_value: Value = serde_json::from_str(&data)?;

    if let Some(obj) = json_value.as_object_mut() {
        obj.insert(parm.to_string(), Value::from(value.to_string()));
    }

    let updated_json = serde_json::to_string_pretty(&json_value)?;

    fs::write(path, updated_json)?;

    Ok(())
}

fn get_config_data(path: &str, key: &str) -> Result<String, io::Error> {
    let file = File::open(path)?;

    let v: Value =
        serde_json::from_reader(file).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    if let Some(value) = v.get(key) {
        // Jeśli wartość to string w formacie JSON (np. "wartość"), as_str() wyciągnie czysty tekst bez cudzysłowów.
        // W przeciwnym wypadku to_string() zamieni dowolny typ (np. tablicę, obiekt) na reprezentację tekstową.
        let result = value
            .as_str()
            .map(|s| s.to_string())
            .unwrap_or_else(|| value.to_string());

        Ok(result)
    } else {
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "Klucz '{}' nie został znaleziony w pliku konfiguracyjnym",
                key
            ),
        ))
    }
}
