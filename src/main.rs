use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::Html,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
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
    Html(include_str!("index.html"))
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
