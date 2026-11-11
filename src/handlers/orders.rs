use crate::auth::AuthUser;
use crate::client_ip::ClientIp;
use crate::db;
use crate::error::AppError;
use crate::security;
use crate::state::AppState;
use crate::util;
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use chrono::{Duration, NaiveDate};
use qrcode::{QrCode, render::svg};
use serde::{Deserialize, Serialize};
use sqlx::{Connection, FromRow, QueryBuilder, Sqlite, Transaction};
use std::collections::HashMap;
use std::sync::Arc;

const ORDER_SELECT: &str = "SELECT o.id, o.code, o.status, o.payment_method, o.payment_status,
        o.total_cents, o.pickup_time, o.created_at, o.updated_at,
        o.paid_at, o.uncollected_at,
        u.login AS user_login, u.display_code AS user_display_code,
        p.login AS paid_by_login,
        oi.name AS item_name, oi.price_cents AS item_price, oi.quantity AS item_qty
 FROM orders o
 JOIN users u ON u.id = o.user_id
 LEFT JOIN users p ON p.id = o.paid_by
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
    pub paid_at: Option<i64>,
    pub paid_by: Option<String>,
    pub uncollected_at: Option<i64>,
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
    paid_at: Option<i64>,
    uncollected_at: Option<i64>,
    user_login: String,
    user_display_code: String,
    paid_by_login: Option<String>,
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

#[derive(FromRow)]
struct OrderState {
    status: String,
    payment_status: String,
    created_at: i64,
    paid_at: Option<i64>,
    paid_by: Option<i64>,
    ready_at: Option<i64>,
    uncollected_at: Option<i64>,
}

#[derive(Deserialize)]
pub struct CreateOrderRequest {
    pub items: Vec<CreateOrderItem>,
    pub payment_method: String,
    #[serde(default)]
    pub pickup_time: String,
    #[serde(default)]
    pub idempotency_key: Option<String>,
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

#[derive(Deserialize)]
pub struct ReportQuery {
    #[serde(default)]
    pub date: Option<String>,
}

#[derive(FromRow)]
struct ReportOrderRow {
    status: String,
    payment_method: String,
    payment_status: String,
    total_cents: i64,
    uncollected_at: Option<i64>,
}

#[derive(Clone, Default, Serialize)]
pub struct PaymentTotals {
    pub orders: i64,
    pub paid_orders: i64,
    pub paid_cents: i64,
    pub unpaid_orders: i64,
    pub unpaid_cents: i64,
    pub cancelled_orders: i64,
    pub uncollected_orders: i64,
    pub completed_orders: i64,
    pub open_orders: i64,
}

#[derive(Serialize)]
pub struct DailyReport {
    pub date: String,
    #[serde(flatten)]
    pub summary: PaymentTotals,
    pub cash: PaymentTotals,
    pub card: PaymentTotals,
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
    headers: HeaderMap,
    Json(body): Json<CreateOrderRequest>,
) -> Result<(StatusCode, Json<OrderView>), AppError> {
    if db::get_setting_or(&state.db, "ordering_enabled", "1").await != "1" {
        return Err(AppError::conflict(
            "Zamawianie jest obecnie wyłączone. Skontaktuj się z obsługą stołówki.",
        ));
    }

    let idempotency_key = resolve_idempotency_key(&headers, body.idempotency_key.as_deref())?;

    let max_order_items = parse_setting(&state, "max_order_items", 20).await;

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
    let now = util::now();

    // BEGIN IMMEDIATE serializuje zapisy: równoległe kliknięcia nie mogą
    // przekroczyć limitu otwartych zamówień ani utworzyć dwóch zamówień
    // z tym samym kluczem idempotencji.
    let mut connection = state.db.acquire().await?;
    let mut transaction = connection.begin_with("BEGIN IMMEDIATE").await?;

    let existing_id: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM orders WHERE user_id = ? AND idempotency_key = ? LIMIT 1",
    )
    .bind(user.id)
    .bind(&idempotency_key)
    .fetch_optional(&mut *transaction)
    .await?;

    if let Some(existing_id) = existing_id {
        transaction.rollback().await?;
        tracing::info!(
            user_id = user.id,
            order_id = existing_id,
            "Powtórzone żądanie utworzenia zamówienia — zwracam istniejące"
        );
        return Ok((StatusCode::OK, Json(load_order(&state, existing_id).await?)));
    }

    let max_uncollected = setting_i64(&mut transaction, "max_uncollected", 0).await?;
    if max_uncollected > 0 {
        let cutoff = now - 30 * 24 * 60 * 60;
        let block_reset_at: Option<i64> =
            sqlx::query_scalar("SELECT uncollected_block_reset_at FROM users WHERE id = ?")
                .bind(user.id)
                .fetch_one(&mut *transaction)
                .await?;
        let block_reset_at = block_reset_at.unwrap_or(0);
        let uncollected: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM orders
             WHERE user_id = ? AND uncollected_at IS NOT NULL
               AND uncollected_at >= ? AND uncollected_at > ?",
        )
        .bind(user.id)
        .bind(cutoff)
        .bind(block_reset_at)
        .fetch_one(&mut *transaction)
        .await?;
        if uncollected >= max_uncollected {
            transaction.rollback().await?;
            return Err(AppError::conflict(format!(
                "Masz {uncollected} nieodebranych zamówień w ostatnich 30 dniach \
                 (limit: {max_uncollected}). Nowe zamówienia są zablokowane — \
                 poproś obsługę stołówki o zdjęcie blokady."
            )));
        }
    }

    let max_open_orders = setting_i64(&mut transaction, "max_open_orders", 3).await?;
    let open_orders: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM orders
         WHERE user_id = ? AND status IN ('new','preparing','ready')
           AND uncollected_at IS NULL",
    )
    .bind(user.id)
    .fetch_one(&mut *transaction)
    .await?;

    if open_orders >= max_open_orders {
        transaction.rollback().await?;
        return Err(AppError::conflict(format!(
            "Masz już {open_orders} oczekujących zamówień. Maksymalnie {max_open_orders}."
        )));
    }

    let mut builder = QueryBuilder::<Sqlite>::new(
        "SELECT id, name, price_cents, available FROM menu_items WHERE id IN (",
    );
    {
        let mut separated = builder.separated(", ");
        for id in &ids {
            separated.push_bind(*id);
        }
    }
    builder.push(")");

    let menu_rows = builder
        .build_query_as::<MenuItemRow>()
        .fetch_all(&mut *transaction)
        .await?;

    if menu_rows.len() != ids.len() {
        transaction.rollback().await?;
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
            transaction.rollback().await?;
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
        transaction.rollback().await?;
        return Err(AppError::bad("Kwota zamówienia jest zbyt duża."));
    }

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
        return Err(AppError::internal(
            "Nie udało się wygenerować kodu odbioru.",
        ));
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
          pickup_time, idempotency_key, created_at, updated_at)
         VALUES (?, ?, ?, 'new', ?, 'unpaid', ?, ?, ?, ?, ?)",
    )
    .bind(user.id)
    .bind(&code)
    .bind(&qr_token)
    .bind(&payment_method)
    .bind(total_cents)
    .bind(&pickup_time)
    .bind(&idempotency_key)
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
    let cancel_window_minutes = parse_setting(&state, "cancel_window_minutes", 30).await;
    let mut connection = state.db.acquire().await?;
    let mut transaction = connection.begin_with("BEGIN IMMEDIATE").await?;

    let current: Option<OrderState> = sqlx::query_as(
        "SELECT status, payment_status, created_at, paid_at, paid_by, ready_at, uncollected_at
         FROM orders WHERE id = ? AND user_id = ?",
    )
    .bind(id)
    .bind(user.id)
    .fetch_optional(&mut *transaction)
    .await?;

    let Some(current) = current else {
        transaction.rollback().await?;
        return Err(AppError::not_found("Nie znaleziono zamówienia."));
    };

    if current.uncollected_at.is_some() {
        transaction.rollback().await?;
        return Err(AppError::conflict(
            "To zamówienie zostało oznaczone jako nieodebrane.",
        ));
    }
    if current.status != "new" {
        transaction.rollback().await?;
        return Err(AppError::conflict(
            "Można anulować tylko zamówienie, które nie zostało jeszcze przyjęte przez kuchnię.",
        ));
    }
    if current.payment_status == "paid" {
        transaction.rollback().await?;
        return Err(AppError::conflict(
            "Opłaconego zamówienia nie można anulować. Skontaktuj się z obsługą stołówki.",
        ));
    }
    if cancel_window_minutes > 0
        && util::now().saturating_sub(current.created_at) > cancel_window_minutes * 60
    {
        transaction.rollback().await?;
        return Err(AppError::conflict(format!(
            "Termin anulowania zamówienia ({cancel_window_minutes} min od złożenia) już minął."
        )));
    }

    let result = sqlx::query(
        "UPDATE orders
         SET status = 'cancelled', updated_at = ?
         WHERE id = ? AND user_id = ? AND status = 'new'
           AND payment_status = 'unpaid' AND uncollected_at IS NULL",
    )
    .bind(util::now())
    .bind(id)
    .bind(user.id)
    .execute(&mut *transaction)
    .await?;

    if result.rows_affected() == 0 {
        transaction.rollback().await?;
        return Err(AppError::conflict(
            "Zamówienie zostało właśnie zmienione. Odśwież listę i spróbuj ponownie.",
        ));
    }

    transaction.commit().await?;

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
    let row: Option<(String, String)> =
        sqlx::query_as("SELECT qr_token, status FROM orders WHERE id = ? AND user_id = ?")
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
            (header::CONTENT_TYPE, "image/svg+xml; charset=utf-8"),
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
        "all" => {
            format!("{ORDER_SELECT} ORDER BY o.created_at DESC, o.id DESC, oi.id ASC LIMIT 300")
        }
        "today" => format!(
            "{ORDER_SELECT} WHERE o.created_at >= ? AND o.created_at < ? ORDER BY o.created_at DESC, o.id DESC, oi.id ASC LIMIT 300"
        ),
        _ => format!(
            "{ORDER_SELECT} WHERE o.status IN ('new','preparing','ready')
             AND o.uncollected_at IS NULL
             ORDER BY o.created_at ASC, o.id ASC, oi.id ASC LIMIT 300"
        ),
    };

    let rows = if scope == "today" {
        sqlx::query_as::<_, OrderRow>(&sql)
            .bind(util::start_of_today())
            .bind(util::start_of_local_date(util::today() + Duration::days(1)))
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
    Extension(client): Extension<ClientIp>,
    Json(body): Json<PickupRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    if let Err(retry_after) = state.pickup_limiter.check(user.id, client.0) {
        tracing::warn!(
            user_id = user.id,
            retry_after,
            "Odczyt kodu odbioru zablokowany przez limiter"
        );
        return Err(AppError::TooManyRequests {
            retry_after,
            message: "Zbyt wiele nieudanych prób odczytu kodu. Odczekaj chwilę.".to_string(),
        });
    }

    let raw = body.code.trim();
    if raw.is_empty() || raw.len() > 128 {
        return Err(AppError::bad("Wpisz kod odbioru lub zeskanuj kod QR."));
    }

    let token = raw.strip_prefix("ZSL:").unwrap_or(raw).trim();
    let normalized_code = token.to_uppercase();

    let id: Option<i64> =
        sqlx::query_scalar("SELECT id FROM orders WHERE qr_token = ? OR code = ? LIMIT 1")
            .bind(token)
            .bind(&normalized_code)
            .fetch_optional(&state.db)
            .await?;

    let Some(id) = id else {
        state.pickup_limiter.failure(user.id, client.0);
        if let Err(retry_after) = state.pickup_limiter.check(user.id, client.0) {
            return Err(AppError::TooManyRequests {
                retry_after,
                message: "Zbyt wiele nieudanych prób odczytu kodu. Odczekaj chwilę.".to_string(),
            });
        }
        return Err(AppError::not_found(
            "Nie znaleziono zamówienia o tym kodzie.",
        ));
    };

    state.pickup_limiter.success(user.id, client.0);
    let order = load_order(&state, id).await?;

    db::audit(
        &state.db,
        Some(user.id),
        "pickup_lookup",
        &format!("Odczyt zamówienia id={id}"),
    )
    .await;

    if order.status == "cancelled" || order.status == "uncollected" {
        return Err(AppError::conflict(
            "To zamówienie zostało anulowane lub nieodebrane.",
        ));
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

    if let Some(value) = &status
        && !matches!(
            value.as_str(),
            "new" | "preparing" | "ready" | "completed" | "cancelled"
        )
    {
        return Err(AppError::bad("Nieprawidłowy status zamówienia."));
    }
    if let Some(value) = &payment_status
        && !matches!(value.as_str(), "unpaid" | "paid")
    {
        return Err(AppError::bad("Nieprawidłowy status płatności."));
    }

    let current: Option<OrderState> = sqlx::query_as(
        "SELECT status, payment_status, created_at, paid_at, paid_by, ready_at, uncollected_at
         FROM orders WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?;

    let current = current.ok_or_else(|| AppError::not_found("Nie znaleziono zamówienia."))?;

    if current.payment_status == "paid"
        && payment_status.as_deref() == Some("unpaid")
        && user.role != "admin"
    {
        return Err(AppError::forbidden(
            "Cofnięcie płatności może wykonać tylko administrator.",
        ));
    }

    if current.uncollected_at.is_some() && user.role != "admin" {
        return Err(AppError::conflict(
            "To zamówienie zostało oznaczone jako nieodebrane i nie można go już zmienić.",
        ));
    }

    if user.role == "kitchen" {
        if current.created_at < util::start_of_today() {
            return Err(AppError::forbidden(
                "Kuchnia może zmieniać tylko dzisiejsze zamówienia.",
            ));
        }
        if matches!(current.status.as_str(), "completed" | "cancelled") {
            return Err(AppError::conflict(
                "To zamówienie jest już zamknięte i nie można go zmienić.",
            ));
        }
        if let Some(value) = &status {
            let allowed = match current.status.as_str() {
                "new" => matches!(value.as_str(), "new" | "preparing"),
                "preparing" => matches!(value.as_str(), "preparing" | "ready"),
                "ready" => matches!(value.as_str(), "ready" | "completed"),
                _ => false,
            };
            if !allowed {
                return Err(AppError::forbidden(
                    "Niedozwolona zmiana statusu. Kuchnia prowadzi zamówienie krok po kroku.",
                ));
            }
        }
        if let Some(value) = &payment_status
            && value != "paid"
        {
            return Err(AppError::forbidden(
                "Kuchnia może tylko oznaczyć płatność jako opłaconą.",
            ));
        }
        let final_status = status.as_deref().unwrap_or(&current.status);
        if payment_status.as_deref() == Some("paid")
            && !matches!(final_status, "ready" | "completed")
        {
            return Err(AppError::conflict(
                "Płatność można oznaczyć dopiero przy wydaniu zamówienia.",
            ));
        }
    }

    if status.as_deref() == Some("completed") && payment_status.is_none() {
        payment_status = Some("paid".to_string());
    }

    let final_status = status.as_deref().unwrap_or(&current.status).to_string();
    let final_payment = payment_status
        .as_deref()
        .unwrap_or(&current.payment_status)
        .to_string();

    let now = util::now();
    let (new_paid_at, new_paid_by) = if final_payment == "paid" {
        if current.payment_status == "paid" {
            (current.paid_at, current.paid_by)
        } else {
            (Some(now), Some(user.id))
        }
    } else {
        (None, None)
    };

    let new_ready_at = if final_status == "ready" {
        if current.status == "ready" {
            current.ready_at
        } else {
            Some(now)
        }
    } else {
        None
    };

    // Administrator może skorygować pomyłkę: zmiana statusu zdejmuje
    // znacznik nieodebranego i przywraca zamówienie do normalnego obiegu.
    let new_uncollected_at = if user.role == "admin" && status.is_some() {
        None
    } else {
        current.uncollected_at
    };

    let mut connection = state.db.acquire().await?;
    let mut transaction = connection.begin_with("BEGIN IMMEDIATE").await?;

    let result = sqlx::query(
        "UPDATE orders
         SET status = ?, payment_status = ?, paid_at = ?, paid_by = ?,
             ready_at = ?, uncollected_at = ?, updated_at = ?
         WHERE id = ? AND status = ? AND payment_status = ?
           AND COALESCE(uncollected_at, 0) = ?",
    )
    .bind(&final_status)
    .bind(&final_payment)
    .bind(new_paid_at)
    .bind(new_paid_by)
    .bind(new_ready_at)
    .bind(new_uncollected_at)
    .bind(now)
    .bind(id)
    .bind(&current.status)
    .bind(&current.payment_status)
    .bind(current.uncollected_at.unwrap_or(0))
    .execute(&mut *transaction)
    .await?;

    if result.rows_affected() == 0 {
        transaction.rollback().await?;
        return Err(AppError::conflict(
            "Zamówienie zostało właśnie zmienione przez inną osobę. Odśwież listę.",
        ));
    }

    transaction.commit().await?;

    let payment_reverted = current.payment_status == "paid" && final_payment == "unpaid";
    let audit_action = if payment_reverted {
        "order_payment_reverted"
    } else {
        "order_updated"
    };
    db::audit(
        &state.db,
        Some(user.id),
        audit_action,
        &format!(
            "Zamówienie id={id}, status {} -> {}, płatność {} -> {}, opłacone przez={}, paid_at={}",
            current.status,
            final_status,
            current.payment_status,
            final_payment,
            new_paid_by
                .map(|value| value.to_string())
                .unwrap_or_else(|| "—".to_string()),
            new_paid_at
                .map(|value| value.to_string())
                .unwrap_or_else(|| "—".to_string()),
        ),
    )
    .await;

    Ok(Json(load_order(&state, id).await?))
}

pub async fn staff_report(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ReportQuery>,
) -> Result<Json<DailyReport>, AppError> {
    Ok(Json(
        build_daily_report(&state, query.date.as_deref()).await?,
    ))
}

pub async fn staff_report_csv(
    State(state): State<Arc<AppState>>,
    Extension(actor): Extension<AuthUser>,
    Query(query): Query<ReportQuery>,
) -> Result<Response, AppError> {
    let report = build_daily_report(&state, query.date.as_deref()).await?;

    let mut csv = String::from("\u{feff}");
    csv.push_str(
        "data;metoda;zamowienia;oplacone;kwota_oplacona_gr;nieoplacone;\
         kwota_nieoplacona_gr;anulowane;nieodebrane\r\n",
    );
    for (method, totals) in [
        ("gotówka", &report.cash),
        ("karta", &report.card),
        ("razem", &report.summary),
    ] {
        csv.push_str(&format!(
            "{};{};{};{};{};{};{};{};{}\r\n",
            csv_cell(&report.date),
            csv_cell(method),
            totals.orders,
            totals.paid_orders,
            totals.paid_cents,
            totals.unpaid_orders,
            totals.unpaid_cents,
            totals.cancelled_orders,
            totals.uncollected_orders,
        ));
    }

    let filename = format!("raport-zsl-order-{}.csv", report.date);
    db::audit(
        &state.db,
        Some(actor.id),
        "report_exported",
        &format!(
            "Eksport raportu dziennego {}, zamówień={}",
            report.date, report.summary.orders
        ),
    )
    .await;

    let mut response = (StatusCode::OK, csv).into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/csv; charset=utf-8"),
    );
    if let Ok(value) = HeaderValue::from_str(&format!("attachment; filename=\"{filename}\"")) {
        headers.insert(header::CONTENT_DISPOSITION, value);
    }
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}

async fn build_daily_report(
    state: &Arc<AppState>,
    requested_date: Option<&str>,
) -> Result<DailyReport, AppError> {
    let date = match requested_date
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(value) => NaiveDate::parse_from_str(value, "%Y-%m-%d")
            .map_err(|_| AppError::bad("Data raportu musi być w formacie RRRR-MM-DD."))?,
        None => util::today(),
    };
    let start = util::start_of_local_date(date);
    let end = util::start_of_local_date(date + Duration::days(1));

    let rows = sqlx::query_as::<_, ReportOrderRow>(
        "SELECT status, payment_method, payment_status, total_cents, uncollected_at
         FROM orders WHERE created_at >= ? AND created_at < ?",
    )
    .bind(start)
    .bind(end)
    .fetch_all(&state.db)
    .await?;

    let summary = aggregate_totals(rows.iter());
    let cash = aggregate_totals(rows.iter().filter(|row| row.payment_method == "cash"));
    let card = aggregate_totals(rows.iter().filter(|row| row.payment_method == "card"));

    Ok(DailyReport {
        date: date.format("%Y-%m-%d").to_string(),
        summary,
        cash,
        card,
    })
}

fn aggregate_totals<'a>(rows: impl Iterator<Item = &'a ReportOrderRow>) -> PaymentTotals {
    let mut totals = PaymentTotals::default();
    for row in rows {
        totals.orders += 1;

        let uncollected = row.uncollected_at.is_some();
        let cancelled = row.status == "cancelled" && !uncollected;
        // Kategorie liczby zamówień są rozłączne:
        // paid + unpaid + cancelled + uncollected = orders.
        if uncollected {
            totals.uncollected_orders += 1;
        } else if cancelled {
            totals.cancelled_orders += 1;
        } else if row.status == "completed" {
            totals.completed_orders += 1;
        } else if matches!(row.status.as_str(), "new" | "preparing" | "ready") {
            totals.open_orders += 1;
        }

        // Kwoty pieniężne obejmują również nieodebrane zamówienia: jeśli
        // zostały opłacone, pieniądze wpłynęły do kasy. Jawne anulowanie
        // nie jest przychodem. Liczniki `paid_orders`/`unpaid_orders` są
        // rozłączne z `uncollected_orders`, aby nie podwajać zamówień.
        if !cancelled {
            if row.payment_status == "paid" {
                if !uncollected {
                    totals.paid_orders += 1;
                }
                totals.paid_cents += row.total_cents;
            } else {
                if !uncollected {
                    totals.unpaid_orders += 1;
                }
                totals.unpaid_cents += row.total_cents;
            }
        }
    }
    totals
}

fn csv_cell(value: &str) -> String {
    let mut text = value.to_string();
    if text
        .chars()
        .next()
        .is_some_and(|first| matches!(first, '=' | '+' | '-' | '@'))
    {
        text.insert(0, '\'');
    }
    format!("\"{}\"", text.replace('"', "\"\""))
}

/// Oznacza jako nieodebrane zamówienia, które przez `pickup_limit_minutes`
/// od ustawienia statusu `ready` nie zostały wydane. W bazie zostają
/// `cancelled` z ustawionym `uncollected_at`; API pokazuje je jako
/// `uncollected`. Funkcja jest odporna na wyścig z kuchnią dzięki
/// warunkowemu `UPDATE ... WHERE status = 'ready'`.
pub async fn mark_uncollected(state: &Arc<AppState>) -> Result<u64, sqlx::Error> {
    let limit = parse_setting(state, "pickup_limit_minutes", 120).await;
    if limit <= 0 {
        return Ok(0);
    }

    let now = util::now();
    let cutoff = now - limit * 60;
    let ids: Vec<i64> = sqlx::query_scalar(
        "SELECT id FROM orders
         WHERE status = 'ready' AND uncollected_at IS NULL
           AND ready_at IS NOT NULL AND ready_at <= ?
         ORDER BY ready_at ASC LIMIT 200",
    )
    .bind(cutoff)
    .fetch_all(&state.db)
    .await?;

    let mut marked = 0_u64;
    let mut marked_ids: Vec<i64> = Vec::new();
    for id in ids {
        let result = sqlx::query(
            "UPDATE orders
             SET status = 'cancelled', uncollected_at = ?, updated_at = ?
             WHERE id = ? AND status = 'ready' AND uncollected_at IS NULL",
        )
        .bind(now)
        .bind(now)
        .bind(id)
        .execute(&state.db)
        .await?;
        if result.rows_affected() > 0 {
            marked += 1;
            if marked_ids.len() < 50 {
                marked_ids.push(id);
            }
        }
    }

    if marked > 0 {
        db::audit(
            &state.db,
            None,
            "orders_uncollected",
            &format!(
                "Automatycznie oznaczono nieodebrane: {marked}, id=[{}]",
                marked_ids
                    .iter()
                    .map(|id| id.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            ),
        )
        .await;
        tracing::info!(marked, "Oznaczono nieodebrane zamówienia");
    }

    Ok(marked)
}

async fn load_order(state: &Arc<AppState>, id: i64) -> Result<OrderView, AppError> {
    let sql = format!("{ORDER_SELECT} WHERE o.id = ? ORDER BY oi.id ASC");
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
                let status = if row.uncollected_at.is_some() {
                    "uncollected".to_string()
                } else {
                    row.status
                };
                orders.push(OrderView {
                    id: row.id,
                    code: row.code,
                    status,
                    payment_method: row.payment_method,
                    payment_status: row.payment_status,
                    total_cents: row.total_cents,
                    pickup_time: row.pickup_time,
                    created_at: row.created_at,
                    updated_at: row.updated_at,
                    paid_at: row.paid_at,
                    paid_by: row.paid_by_login,
                    uncollected_at: row.uncollected_at,
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

async fn setting_i64(
    transaction: &mut Transaction<'_, Sqlite>,
    key: &str,
    default: i64,
) -> Result<i64, sqlx::Error> {
    let value: Option<String> = sqlx::query_scalar("SELECT value FROM settings WHERE key = ?")
        .bind(key)
        .fetch_optional(&mut **transaction)
        .await?;
    Ok(value
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(default))
}

/// Klient generuje losowy klucz na jedno kliknięcie „Złóż zamówienie”.
/// Powtórzenie tego samego żądania (np. ponowienie po timeoucie) zwraca
/// istniejące zamówienie zamiast tworzyć drugie.
fn resolve_idempotency_key(
    headers: &HeaderMap,
    body_key: Option<&str>,
) -> Result<String, AppError> {
    let header_key = headers
        .get("idempotency-key")
        .or_else(|| headers.get("x-idempotency-key"))
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let body_key = body_key.map(str::trim).filter(|value| !value.is_empty());

    if let (Some(body), Some(header)) = (body_key, header_key)
        && body != header
    {
        return Err(AppError::bad(
            "Klucz idempotencji w nagłówku i treści musi być zgodny.",
        ));
    }

    let key = body_key.or(header_key).ok_or_else(|| {
        AppError::bad("Brak klucza idempotencji. Odśwież stronę i spróbuj ponownie.")
    })?;

    if key.len() < 8
        || key.len() > 128
        || !key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        return Err(AppError::bad("Nieprawidłowy klucz idempotencji."));
    }

    Ok(key.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idempotency_key_requires_safe_value() {
        let headers = HeaderMap::new();
        assert!(resolve_idempotency_key(&headers, Some("abcdefgh")).is_ok());
        assert!(resolve_idempotency_key(&headers, Some("krótki")).is_err());
        assert!(resolve_idempotency_key(&headers, Some("abcdefgh")).is_ok());
        assert!(resolve_idempotency_key(&headers, Some("abc defgh")).is_err());
        assert!(resolve_idempotency_key(&headers, Some("abc;defgh")).is_err());
        assert!(resolve_idempotency_key(&headers, None).is_err());
    }

    #[test]
    fn idempotency_key_header_and_body_must_match() {
        let mut headers = HeaderMap::new();
        headers.insert("idempotency-key", "abcdefgh".parse().unwrap());
        assert!(resolve_idempotency_key(&headers, Some("abcdefgh")).is_ok());
        assert!(resolve_idempotency_key(&headers, Some("ijklmnop")).is_err());
        assert!(resolve_idempotency_key(&headers, None).is_ok());
    }

    fn report_row(
        status: &str,
        method: &str,
        payment: &str,
        total_cents: i64,
        uncollected: bool,
    ) -> ReportOrderRow {
        ReportOrderRow {
            status: status.to_string(),
            payment_method: method.to_string(),
            payment_status: payment.to_string(),
            total_cents,
            uncollected_at: uncollected.then_some(1_700_000_000),
        }
    }

    #[test]
    fn report_totals_match_orders_and_split_payment_methods() {
        let rows = [
            report_row("completed", "cash", "paid", 1500, false),
            report_row("completed", "card", "paid", 2500, false),
            report_row("ready", "cash", "unpaid", 800, false),
            report_row("cancelled", "cash", "paid", 999, false),
            report_row("cancelled", "card", "paid", 1200, true),
        ];

        let summary = aggregate_totals(rows.iter());
        assert_eq!(summary.orders, 5);
        assert_eq!(summary.completed_orders, 2);
        assert_eq!(summary.open_orders, 1);
        assert_eq!(summary.cancelled_orders, 1);
        assert_eq!(summary.uncollected_orders, 1);
        // Kategorie są rozłączne: nieodebrane nie jest liczone jako opłacone.
        assert_eq!(
            summary.paid_orders
                + summary.unpaid_orders
                + summary.cancelled_orders
                + summary.uncollected_orders,
            summary.orders
        );
        // Anulowane (jawne) nie wchodzi do przychodu, nieodebrane opłacone wchodzi.
        assert_eq!(summary.paid_orders, 2);
        assert_eq!(summary.paid_cents, 5200);
        assert_eq!(summary.unpaid_orders, 1);
        assert_eq!(summary.unpaid_cents, 800);

        let cash = aggregate_totals(rows.iter().filter(|row| row.payment_method == "cash"));
        let card = aggregate_totals(rows.iter().filter(|row| row.payment_method == "card"));
        assert_eq!(cash.paid_cents, 1500);
        assert_eq!(cash.unpaid_cents, 800);
        assert_eq!(cash.cancelled_orders, 1);
        assert_eq!(card.paid_cents, 3700);
        assert_eq!(card.uncollected_orders, 1);
        assert_eq!(cash.paid_cents + card.paid_cents, summary.paid_cents);
        assert_eq!(cash.unpaid_cents + card.unpaid_cents, summary.unpaid_cents);
    }

    #[test]
    fn report_paid_then_uncollected_counts_once() {
        let rows = [report_row("cancelled", "cash", "paid", 1500, true)];

        let summary = aggregate_totals(rows.iter());
        assert_eq!(summary.orders, 1);
        assert_eq!(summary.uncollected_orders, 1);
        assert_eq!(summary.cancelled_orders, 0);
        assert_eq!(summary.paid_orders, 0);
        assert_eq!(summary.unpaid_orders, 0);
        // Pieniądze z opłaconego, ale nieodebranego zamówienia zostają w kasie.
        assert_eq!(summary.paid_cents, 1500);
        assert_eq!(
            summary.paid_orders
                + summary.unpaid_orders
                + summary.cancelled_orders
                + summary.uncollected_orders,
            summary.orders
        );
    }

    #[test]
    fn report_csv_quotes_cells_and_blocks_formulas() {
        assert_eq!(csv_cell("gotówka"), "\"gotówka\"");
        assert_eq!(csv_cell("a;b"), "\"a;b\"");
        assert_eq!(csv_cell("=1+1"), "\"'=1+1\"");
        assert_eq!(csv_cell("+51"), "\"'+51\"");
        assert_eq!(csv_cell("-1"), "\"'-1\"");
        assert_eq!(csv_cell("@x"), "\"'@x\"");
    }
}
