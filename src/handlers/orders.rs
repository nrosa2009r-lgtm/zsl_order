use crate::auth::AuthUser;
use crate::db;
use crate::error::AppError;
use crate::security;
use crate::state::AppState;
use crate::util;
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::{StatusCode, header},
};
use qrcode::{QrCode, render::svg};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, QueryBuilder, Sqlite};
use std::collections::HashMap;
use std::sync::Arc;

const ORDER_SELECT: &str = "SELECT o.id, o.code, o.status, o.payment_method, o.payment_status,
        o.total_cents, o.pickup_time, o.created_at, o.updated_at,
        u.login AS user_login, u.display_code AS user_display_code,
        oi.name AS item_name, oi.price_cents AS item_price, oi.quantity AS item_qty
 FROM orders o
 JOIN users u ON u.id = o.user_id
 LEFT JOIN order_items oi ON oi.order_id = o.id";

#[derive(Clone, Serialize)]
pub struct OrderItemView {
    pub name: String,
    pub price_cents: i64,
    pub quantity: i64,
}

#[derive(Clone, Serialize)]
pub struct OrderView {
    pub id: i64,
    pub code: String,
    pub status: String,
    pub payment_method: String,
    pub payment_status: String,
    pub total_cents: i64,
    pub pickup_time: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub student: String,
    pub student_code: String,
    pub items: Vec<OrderItemView>,
}

#[derive(FromRow)]
struct OrderRow {
    id: i64,
    code: String,
    status: String,
    payment_method: String,
    payment_status: String,
    total_cents: i64,
    pickup_time: String,
    created_at: i64,
    updated_at: i64,
    user_login: String,
    user_display_code: String,
    item_name: Option<String>,
    item_price: Option<i64>,
    item_qty: Option<i64>,
}

#[derive(Clone, FromRow)]
struct MenuItemRow {
    id: i64,
    name: String,
    price_cents: i64,
    available: bool,
}

#[derive(Deserialize)]
pub struct CreateOrderRequest {
    pub items: Vec<CreateOrderItem>,
    pub payment_method: String,
    #[serde(default)]
    pub pickup_time: String,
}

#[derive(Deserialize)]
pub struct CreateOrderItem {
    pub menu_item_id: i64,
    pub quantity: i64,
}

#[derive(Deserialize)]
pub struct OrdersQuery {
    #[serde(default)]
    pub scope: Option<String>,
}

#[derive(Deserialize)]
pub struct PickupRequest {
    pub code: String,
}

#[derive(Deserialize)]
pub struct UpdateOrderRequest {
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub payment_status: Option<String>,
}

pub async fn student_orders(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
) -> Result<Json<Vec<OrderView>>, AppError> {
    let sql = format!(
        "{ORDER_SELECT}
         WHERE o.user_id = ?
         ORDER BY o.created_at DESC, o.id DESC, oi.id ASC
         LIMIT 200"
    );
    let rows = sqlx::query_as::<_, OrderRow>(&sql)
        .bind(user.id)
        .fetch_all(&state.db)
        .await?;

    Ok(Json(group_orders(rows)))
}

pub async fn create_order(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Json(body): Json<CreateOrderRequest>,
) -> Result<(StatusCode, Json<OrderView>), AppError> {
    if db::get_setting_or(&state.db, "ordering_enabled", "1").await != "1" {
        return Err(AppError::conflict(
            "Zamawianie jest obecnie wyłączone. Skontaktuj się z obsługą stołówki.",
        ));
    }

    let max_open_orders = parse_setting(&state, "max_open_orders", 3).await;
    let max_order_items = parse_setting(&state, "max_order_items", 20).await;
    let open_orders: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM orders
         WHERE user_id = ? AND status IN ('new','preparing','ready')",
    )
    .bind(user.id)
    .fetch_one(&state.db)
    .await?;

    if open_orders >= max_open_orders {
        return Err(AppError::conflict(format!(
            "Masz już {open_orders} oczekujących zamówień. Maksymalnie {max_open_orders}."
        )));
    }

    let payment_method = body.payment_method.trim().to_string();
    if !matches!(payment_method.as_str(), "cash" | "card") {
        return Err(AppError::bad(
            "Wybierz sposób płatności: gotówka lub karta.",
        ));
    }

    let pickup_time = body.pickup_time.trim().to_string();
    if !util::valid_pickup_time(&pickup_time) {
        return Err(AppError::bad("Godzina odbioru musi być w formacie HH:MM."));
    }

    if body.items.is_empty() || body.items.len() as i64 > max_order_items {
        return Err(AppError::bad(format!(
            "Zamówienie musi zawierać od 1 do {max_order_items} pozycji."
        )));
    }

    let mut quantities: HashMap<i64, i64> = HashMap::new();
    for item in &body.items {
        if item.menu_item_id <= 0 {
            return Err(AppError::bad("Nieprawidłowa pozycja menu."));
        }
        if !(1..=10).contains(&item.quantity) {
            return Err(AppError::bad(
                "Ilość każdej pozycji musi wynosić od 1 do 10.",
            ));
        }
        if quantities
            .insert(item.menu_item_id, item.quantity)
            .is_some()
        {
            return Err(AppError::bad(
                "Ta sama pozycja menu nie może wystąpić dwa razy.",
            ));
        }
    }

    let ids: Vec<i64> = quantities.keys().copied().collect();
    let mut builder =
        QueryBuilder::<Sqlite>::new("SELECT id, name, price_cents, available FROM menu_items WHERE id IN (");
    {
        let mut separated = builder.separated(", ");
        for id in &ids {
            separated.push_bind(*id);
        }
    }
    builder.push(")");

    let menu_rows = builder
        .build_query_as::<MenuItemRow>()
        .fetch_all(&state.db)
        .await?;

    if menu_rows.len() != ids.len() {
        return Err(AppError::bad(
            "Niektóre wybrane pozycje menu nie istnieją. Odśwież menu.",
        ));
    }

    let by_id: HashMap<i64, MenuItemRow> =
        menu_rows.into_iter().map(|item| (item.id, item)).collect();

    let mut total_cents: i64 = 0;
    for (id, quantity) in &quantities {
        let item = by_id
            .get(id)
            .ok_or_else(|| AppError::bad("Niektóre wybrane pozycje menu nie istnieją."))?;
        if !item.available {
            return Err(AppError::conflict(format!(
                "Pozycja „{}” jest chwilowo niedostępna.",
                item.name
            )));
        }
        total_cents = total_cents
            .checked_add(item.price_cents.saturating_mul(*quantity))
            .ok_or_else(|| AppError::bad("Kwota zamówienia jest zbyt duża."))?;
    }

    if total_cents > 200_000 {
        return Err(AppError::bad("Kwota zamówienia jest zbyt duża."));
    }

    let now = util::now();
    let mut transaction = state.db.begin().await?;

    let mut code = String::new();
    for _ in 0..20 {
        let candidate = security::generate_pickup_code(6);
        let exists: Option<i64> = sqlx::query_scalar("SELECT 1 FROM orders WHERE code = ?")
            .bind(&candidate)
            .fetch_optional(&mut *transaction)
            .await?;
        if exists.is_none() {
            code = candidate;
            break;
        }
    }
    if code.is_empty() {
        return Err(AppError::internal("Nie udało się wygenerować kodu odbioru."));
    }

    let mut qr_token = String::new();
    for _ in 0..20 {
        let candidate = security::random_hex(24);
        let exists: Option<i64> = sqlx::query_scalar("SELECT 1 FROM orders WHERE qr_token = ?")
            .bind(&candidate)
            .fetch_optional(&mut *transaction)
            .await?;
        if exists.is_none() {
            qr_token = candidate;
            break;
        }
    }
    if qr_token.is_empty() {
        return Err(AppError::internal("Nie udało się wygenerować tokenu QR."));
    }

    let result = sqlx::query(
        "INSERT INTO orders
         (user_id, code, qr_token, status, payment_method, payment_status, total_cents,
          pickup_time, created_at, updated_at)
         VALUES (?, ?, ?, 'new', ?, 'unpaid', ?, ?, ?, ?)",
    )
    .bind(user.id)
    .bind(&code)
    .bind(&qr_token)
    .bind(&payment_method)
    .bind(total_cents)
    .bind(&pickup_time)
    .bind(now)
    .bind(now)
    .execute(&mut *transaction)
    .await?;

    let order_id = result.last_insert_rowid();

    for (menu_item_id, quantity) in &quantities {
        let item = by_id
            .get(menu_item_id)
            .ok_or_else(|| AppError::internal("Brak pozycji menu w transakcji."))?;
        sqlx::query(
            "INSERT INTO order_items
             (order_id, menu_item_id, name, price_cents, quantity)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(order_id)
        .bind(menu_item_id)
        .bind(&item.name)
        .bind(item.price_cents)
        .bind(*quantity)
        .execute(&mut *transaction)
        .await?;
    }

    transaction.commit().await?;

    db::audit(
        &state.db,
        Some(user.id),
        "order_created",
        &format!("Zamówienie id={order_id}, kwota={total_cents} gr, płatność={payment_method}"),
    )
    .await;

    let order = load_order(&state, order_id).await?;
    Ok((StatusCode::CREATED, Json(order)))
}

pub async fn cancel_order(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i64>,
) -> Result<Json<OrderView>, AppError> {
    let status: Option<String> =
        sqlx::query_scalar("SELECT status FROM orders WHERE id = ? AND user_id = ?")
            .bind(id)
            .bind(user.id)
            .fetch_optional(&state.db)
            .await?;

    let status = status.ok_or_else(|| AppError::not_found("Nie znaleziono zamówienia."))?;
    if status != "new" {
        return Err(AppError::conflict(
            "Można anulować tylko zamówienie, które nie zostało jeszcze przyjęte przez kuchnię.",
        ));
    }

    sqlx::query("UPDATE orders SET status = 'cancelled', updated_at = ? WHERE id = ?")
        .bind(util::now())
        .bind(id)
        .execute(&state.db)
        .await?;

    db::audit(
        &state.db,
        Some(user.id),
        "order_cancelled_by_student",
        &format!("Zamówienie id={id}"),
    )
    .await;

    Ok(Json(load_order(&state, id).await?))
}

pub async fn order_qr(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i64>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let row: Option<(String, String)> = sqlx::query_as(
        "SELECT qr_token, status FROM orders WHERE id = ? AND user_id = ?",
    )
    .bind(id)
    .bind(user.id)
    .fetch_optional(&state.db)
    .await?;

    let (token, status) = row.ok_or_else(|| AppError::not_found("Nie znaleziono zamówienia."))?;
    if status == "cancelled" {
        return Err(AppError::conflict("Zamówienie zostało anulowane."));
    }

    let qr = QrCode::new(format!("ZSL:{token}").as_bytes())
        .map_err(|error| AppError::internal(format!("QR: {error}")))?;
    let svg = qr
        .render::<svg::Color>()
        .min_dimensions(320, 320)
        .dark_color(svg::Color("#102a43"))
        .light_color(svg::Color("#ffffff"))
        .quiet_zone(true)
        .build();

    Ok((
        [
            (
                header::CONTENT_TYPE,
                "image/svg+xml; charset=utf-8",
            ),
            (header::CACHE_CONTROL, "no-store"),
        ],
        svg,
    ))
}

pub async fn staff_orders(
    State(state): State<Arc<AppState>>,
    Query(query): Query<OrdersQuery>,
) -> Result<Json<Vec<OrderView>>, AppError> {
    let scope = query.scope.as_deref().unwrap_or("open");
    let sql = match scope {
        "all" => format!(
            "{ORDER_SELECT} ORDER BY o.created_at DESC, o.id DESC, oi.id ASC LIMIT 300"
        ),
        "today" => format!(
            "{ORDER_SELECT} WHERE o.created_at >= ? ORDER BY o.created_at DESC, o.id DESC, oi.id ASC LIMIT 300"
        ),
        _ => format!(
            "{ORDER_SELECT} WHERE o.status IN ('new','preparing','ready')
             ORDER BY o.created_at ASC, o.id ASC, oi.id ASC LIMIT 300"
        ),
    };

    let rows = if scope == "today" {
        sqlx::query_as::<_, OrderRow>(&sql)
            .bind(util::start_of_today())
            .fetch_all(&state.db)
            .await?
    } else {
        sqlx::query_as::<_, OrderRow>(&sql)
            .fetch_all(&state.db)
            .await?
    };

    Ok(Json(group_orders(rows)))
}

pub async fn staff_pickup(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Json(body): Json<PickupRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let raw = body.code.trim();
    if raw.is_empty() || raw.len() > 128 {
        return Err(AppError::bad("Wpisz kod odbioru lub zeskanuj kod QR."));
    }

    let token = raw.strip_prefix("ZSL:").unwrap_or(raw).trim();
    let normalized_code = token.to_uppercase();

    let id: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM orders WHERE qr_token = ? OR code = ? LIMIT 1",
    )
    .bind(token)
    .bind(&normalized_code)
    .fetch_optional(&state.db)
    .await?;

    let id = id.ok_or_else(|| AppError::not_found("Nie znaleziono zamówienia o tym kodzie."))?;
    let order = load_order(&state, id).await?;

    db::audit(
        &state.db,
        Some(user.id),
        "pickup_lookup",
        &format!("Odczyt zamówienia id={id}"),
    )
    .await;

    if order.status == "cancelled" {
        return Err(AppError::conflict("To zamówienie zostało anulowane."));
    }

    Ok(Json(serde_json::json!({ "order": order })))
}

pub async fn staff_update_order(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i64>,
    Json(body): Json<UpdateOrderRequest>,
) -> Result<Json<OrderView>, AppError> {
    let status = body.status.map(|value| value.trim().to_string());
    let mut payment_status = body.payment_status.map(|value| value.trim().to_string());

    if status.is_none() && payment_status.is_none() {
        return Err(AppError::bad("Brak zmian do zapisania."));
    }

    if let Some(value) = &status {
        if !matches!(
            value.as_str(),
            "new" | "preparing" | "ready" | "completed" | "cancelled"
        ) {
            return Err(AppError::bad("Nieprawidłowy status zamówienia."));
        }
    }
    if let Some(value) = &payment_status {
        if !matches!(value.as_str(), "unpaid" | "paid") {
            return Err(AppError::bad("Nieprawidłowy status płatności."));
        }
    }

    let current: Option<(String, String)> = sqlx::query_as(
        "SELECT status, payment_status FROM orders WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?;
    let (old_status, old_payment) =
        current.ok_or_else(|| AppError::not_found("Nie znaleziono zamówienia."))?;

    if user.role == "kitchen" {
        if matches!(old_status.as_str(), "completed" | "cancelled") {
            return Err(AppError::conflict(
                "To zamówienie jest już zamknięte i nie można go zmienić.",
            ));
        }
        if let Some(value) = &status {
            if !matches!(value.as_str(), "preparing" | "ready" | "completed") {
                return Err(AppError::forbidden(
                    "Kuchnia może ustawić tylko status: przygotowanie, gotowe lub wydane.",
                ));
            }
        }
        if let Some(value) = &payment_status {
            if value != "paid" {
                return Err(AppError::forbidden(
                    "Kuchnia może tylko oznaczyć płatność jako opłaconą.",
                ));
            }
        }
    }

    if status.as_deref() == Some("completed") && payment_status.is_none() {
        payment_status = Some("paid".to_string());
    }

    sqlx::query(
        "UPDATE orders
         SET status = COALESCE(?, status),
             payment_status = COALESCE(?, payment_status),
             updated_at = ?
         WHERE id = ?",
    )
    .bind(status.as_deref())
    .bind(payment_status.as_deref())
    .bind(util::now())
    .bind(id)
    .execute(&state.db)
    .await?;

    db::audit(
        &state.db,
        Some(user.id),
        "order_updated",
        &format!(
            "Zamówienie id={id}, status {} -> {}, płatność {} -> {}",
            old_status,
            status.as_deref().unwrap_or(&old_status),
            old_payment,
            payment_status.as_deref().unwrap_or(&old_payment)
        ),
    )
    .await;

    Ok(Json(load_order(&state, id).await?))
}

async fn load_order(state: &Arc<AppState>, id: i64) -> Result<OrderView, AppError> {
    let sql = format!(
        "{ORDER_SELECT} WHERE o.id = ? ORDER BY oi.id ASC"
    );
    let rows = sqlx::query_as::<_, OrderRow>(&sql)
        .bind(id)
        .fetch_all(&state.db)
        .await?;

    group_orders(rows)
        .into_iter()
        .next()
        .ok_or_else(|| AppError::not_found("Nie znaleziono zamówienia."))
}

fn group_orders(rows: Vec<OrderRow>) -> Vec<OrderView> {
    let mut orders: Vec<OrderView> = Vec::new();
    let mut index: HashMap<i64, usize> = HashMap::new();

    for row in rows {
        let position = match index.get(&row.id) {
            Some(position) => *position,
            None => {
                let position = orders.len();
                index.insert(row.id, position);
                orders.push(OrderView {
                    id: row.id,
                    code: row.code,
                    status: row.status,
                    payment_method: row.payment_method,
                    payment_status: row.payment_status,
                    total_cents: row.total_cents,
                    pickup_time: row.pickup_time,
                    created_at: row.created_at,
                    updated_at: row.updated_at,
                    student: row.user_login,
                    student_code: row.user_display_code,
                    items: Vec::new(),
                });
                position
            }
        };

        if let (Some(name), Some(price_cents), Some(quantity)) =
            (row.item_name, row.item_price, row.item_qty)
        {
            orders[position].items.push(OrderItemView {
                name,
                price_cents,
                quantity,
            });
        }
    }

    orders
}

async fn parse_setting(state: &Arc<AppState>, key: &str, default: i64) -> i64 {
    db::get_setting_or(&state.db, key, &default.to_string())
        .await
        .parse()
        .unwrap_or(default)
}
