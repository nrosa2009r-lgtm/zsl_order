use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

// Struktura reprezentująca pozycję w menu
#[derive(Clone, Serialize)]
struct MenuItem {
    id: u32,
    name: String,
    price_pln: f64,
}

// Dane wejściowe od klienta podczas składania zamówienia
#[derive(Deserialize)]
struct CreateOrderDto {
    customer_name: String,
    items_ids: Vec<u32>,
}

// Struktura zamówienia zapisywana w pamięci
#[derive(Clone, Serialize)]
struct Order {
    id: Uuid,
    customer_name: String,
    items: Vec<MenuItem>,
    status: String, // np. "Oczekiwanie na odbiór"
}

// Baza danych w pamięci RAM (Thread-safe dla celów demonstracyjnych)
struct AppState {
    menu: Vec<MenuItem>,
    orders: Mutex<Vec<Order>>,
}

#[tokio::main]
async fn main() {
    // Inicjalizacja przykładowego menu restauracji
    let shared_state = Arc::new(AppState {
        menu: vec![
            MenuItem {
                id: 1,
                name: "Pizza Margherita".to_string(),
                price_pln: 32.0,
            },
            MenuItem {
                id: 2,
                name: "Burger Wołowy".to_string(),
                price_pln: 38.5,
            },
            MenuItem {
                id: 3,
                name: "Sałatka Cezar".to_string(),
                price_pln: 29.0,
            },
        ],
        orders: Mutex::new(Vec::new()),
    });

    // Definicja routingu API
    let app = Router::new()
        .route("/menu", get(get_menu))
        .route("/order", post(create_order))
        .route("/orders", get(get_all_orders))
        .with_state(shared_state);

    // Uruchomienie serwera HTTP
    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000")
        .await
        .unwrap();
    println!(
        "Serwer gastronomiczny działa na http://{}",
        listener.local_addr().unwrap()
    );
    axum::serve(listener, app).await.unwrap();
}

// Endpoint: Pobieranie menu restauracji
async fn get_menu(State(state): State<Arc<AppState>>) -> Json<Vec<MenuItem>> {
    Json(state.menu.clone())
}

// Endpoint: Składanie zamówienia (Odbiór własny - płatność na miejscu)
async fn create_order(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<CreateOrderDto>,
) -> (StatusCode, Json<Order>) {
    // Filtrowanie i dopasowanie wybranych ID do pozycji z menu
    let ordered_items: Vec<MenuItem> = state
        .menu
        .iter()
        .filter(|item| payload.items_ids.contains(&item.id))
        .cloned()
        .collect();

    // Nowe zamówienie z domyślnym statusem
    let new_order = Order {
        id: Uuid::new_v4(),
        customer_name: payload.customer_name,
        items: ordered_items,
        status: "Przyjęte do realizacji (Odbiór własny)".to_string(),
    };

    // Zapis zamówienia do naszej pamięci
    let mut orders = state.orders.lock().unwrap();
    orders.push(new_order.clone());

    (StatusCode::CREATED, Json(new_order))
}

// Endpoint: Podgląd wszystkich złożonych zamówień (np. dla panelu kucharza)
async fn get_all_orders(State(state): State<Arc<AppState>>) -> Json<Vec<Order>> {
    let orders = state.orders.lock().unwrap();
    Json(orders.clone())
}
