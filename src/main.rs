use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs, io,
    sync::{Arc, Mutex},
};
use tower_http::services::{ServeDir, ServeFile};
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
    if let Err(e) = write_config_parm("config.json", "status", "working") {
        eprintln!("Nie udało się zapisać config.json: {e}");
    }

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

    // API ma pierwszeństwo, wszystko inne (html, css, js) leci z ./frontend
    let app = Router::new()
        .route_service("/login", ServeFile::new("frontend/login.html"))
        .route("/menu", get(get_menu))
        .route("/order", post(create_order))
        .route("/orders", get(get_all_orders))
        .fallback_service(ServeDir::new("frontend").append_index_html_on_directories(true))
        .with_state(shared_state);

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

    state.orders.lock().unwrap().push(new_order.clone());

    (StatusCode::CREATED, Json(new_order))
}

async fn get_all_orders(State(state): State<Arc<AppState>>) -> Json<Vec<Order>> {
    Json(state.orders.lock().unwrap().clone())
}

fn write_config_parm(
    path: &str,
    parm: &str,
    value: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut json_value: Value = match fs::read_to_string(path) {
        Ok(data) => serde_json::from_str(&data)?,
        Err(e) if e.kind() == io::ErrorKind::NotFound => Value::Object(Default::default()),
        Err(e) => return Err(e.into()),
    };

    if let Some(obj) = json_value.as_object_mut() {
        obj.insert(parm.to_string(), Value::from(value));
    }

    fs::write(path, serde_json::to_string_pretty(&json_value)?)?;
    Ok(())
}

fn get_config_data(path: &str, key: &str) -> Result<String, io::Error> {
    let file = fs::File::open(path)?;
    let v: Value =
        serde_json::from_reader(file).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    match v.get(key) {
        Some(value) => Ok(value
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| value.to_string())),
        None => Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("Klucz '{key}' nie został znaleziony w pliku konfiguracyjnym"),
        )),
    }
}
