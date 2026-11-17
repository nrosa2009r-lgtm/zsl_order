// SPDX-License-Identifier: MIT
use crate::auth::AuthUser;
use crate::client_ip::ClientIp;
use crate::db;
use crate::error::AppError;
use crate::security;
use crate::state::AppState;
use crate::stock;
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
use sqlx::{Connection, FromRow, QueryBuilder, Sqlite, SqlitePool, Transaction};
use std::collections::HashMap;
use std::sync::Arc;

const ORDER_SELECT: &str = "SELECT o.id, o.code, o.status, o.payment_method, o.payment_status,
        o.total_cents, o.pickup_time, o.pickup_date, o.break_id, o.created_at,
        o.updated_at, o.paid_at, o.issued_at, o.uncollected_at,
        COALESCE(u.login, '') AS user_login,
        COALESCE(u.display_code, 'konto usunięte') AS user_display_code,
        p.login AS paid_by_login,
        b.name AS break_name,
        oi.name AS item_name, oi.price_cents AS item_price, oi.quantity AS item_qty
 FROM orders o
 LEFT JOIN users u ON u.id = o.user_id
 LEFT JOIN users p ON p.id = o.paid_by
 LEFT JOIN breaks b ON b.id = o.break_id
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
    /// Dzień odbioru (`RRRR-MM-DD`); `None` = dzień złożenia.
    pub pickup_date: Option<String>,
    pub break_id: Option<i64>,
    pub break_name: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub paid_at: Option<i64>,
    pub paid_by: Option<String>,
    pub issued_at: Option<i64>,
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
    pickup_date: Option<String>,
    break_id: Option<i64>,
    break_name: Option<String>,
    created_at: i64,
    updated_at: i64,
    paid_at: Option<i64>,
    issued_at: Option<i64>,
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
    stock_remaining: Option<i64>,
}

#[derive(FromRow)]
struct OrderState {
    status: String,
    payment_status: String,
    created_at: i64,
    pickup_date: Option<String>,
    paid_at: Option<i64>,
    paid_by: Option<i64>,
    issued_at: Option<i64>,
    uncollected_at: Option<i64>,
}

#[derive(Deserialize)]
pub struct CreateOrderRequest {
    pub items: Vec<CreateOrderItem>,
    pub payment_method: String,
    #[serde(default)]
    pub pickup_time: String,
    /// Dzień odbioru `RRRR-MM-DD`; brak = dziś. Dozwolony zakres wyznacza
    /// ustawienie `order_days_ahead`.
    #[serde(default)]
    pub pickup_date: Option<String>,
    /// Przerwa (okno wydawania). Wymagana, gdy są aktywne przerwy.
    #[serde(default)]
    pub break_id: Option<i64>,
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
    break_id: Option<i64>,
    break_name: Option<String>,
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
    pub issued_orders: i64,
    pub open_orders: i64,
}

#[derive(Serialize)]
pub struct DailyReport {
    pub date: String,
    #[serde(flatten)]
    pub summary: PaymentTotals,
    pub cash: PaymentTotals,
    pub card: PaymentTotals,
    /// Rozbicie zamówień dnia na przerwy („Bez przerwy” dla zamówień
    /// złożonych, gdy przerwy nie były jeszcze skonfigurowane).
    pub breaks: Vec<BreakReportRow>,
    /// Bieżący stan pozycji menu i liczba sprzedanych sztuk w dniu raportu.
    pub stock: Vec<StockReportRow>,
}

#[derive(Serialize)]
pub struct BreakReportRow {
    pub break_id: Option<i64>,
    pub break_name: String,
    #[serde(flatten)]
    pub totals: PaymentTotals,
}

#[derive(FromRow, Serialize)]
pub struct StockReportRow {
    pub menu_item_id: i64,
    pub name: String,
    pub stock_remaining: Option<i64>,
    pub stock_default: Option<i64>,
    pub sold: i64,
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

    let today = util::today();
    let order_days_ahead = parse_setting(&state, "order_days_ahead", 0)
        .await
        .clamp(0, 7);
    let pickup_date = resolve_pickup_date(body.pickup_date.as_deref(), today, order_days_ahead)?;
    let pickup_date_key = stock::date_key(pickup_date);

    let mut pickup_time = body.pickup_time.trim().to_string();
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

    // Przerwa jest walidowana w transakcji, aby usunięcie/wyłączenie
    // przerwy w trakcie składania zamówienia nie przeszło po cichu.
    let active_breaks: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM breaks WHERE active = 1")
        .fetch_one(&mut *transaction)
        .await?;
    let break_row: Option<(i64, String, String, bool)> = match body.break_id {
        Some(id) => Some(
            sqlx::query_as::<_, (i64, String, String, bool)>(
                "SELECT id, start_time, end_time, active FROM breaks WHERE id = ?",
            )
            .bind(id)
            .fetch_optional(&mut *transaction)
            .await?
            .ok_or_else(|| AppError::bad("Wybrana przerwa nie istnieje. Odśwież formularz."))?,
        ),
        None => None,
    };
    if let Some((_, _, _, false)) = break_row {
        transaction.rollback().await?;
        return Err(AppError::conflict(
            "Wybrana przerwa jest nieaktywna. Odśwież formularz.",
        ));
    }
    if break_row.is_none() && active_breaks > 0 {
        transaction.rollback().await?;
        return Err(AppError::bad(
            "Wybierz przerwę, na której chcesz odebrać zamówienie.",
        ));
    }
    let break_id = break_row.as_ref().map(|row| row.0);
    if let Some((_, start_time, end_time, _)) = &break_row {
        if pickup_time.is_empty() {
            pickup_time = start_time.clone();
        } else if !crate::breaks::time_within(&pickup_time, start_time, end_time) {
            transaction.rollback().await?;
            return Err(AppError::bad(format!(
                "Godzina odbioru musi mieścić się w przerwie {start_time}–{end_time}."
            )));
        }
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
        // Liczą się wyłącznie zamówienia zamknięte automatyczną regułą
        // „koniec przerwy + pickup_limit_minutes”. Ręczne korekty
        // administratora zdejmują `uncollected_at`, więc nie wchodzą do
        // blokady.
        let uncollected: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM orders
             WHERE user_id = ? AND status = 'uncollected'
               AND uncollected_at IS NOT NULL
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
         WHERE user_id = ? AND status = 'ordered'",
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

    // Reset dobowy przed odczytem stanu, aby `stock_remaining` odpowiadał
    // dzisiejszej dacie także przy pierwszym zamówieniu po północy.
    stock::reset_daily_stock(&mut *transaction, util::today()).await?;

    let mut builder = QueryBuilder::<Sqlite>::new(
        "SELECT id, name, price_cents, available, stock_remaining FROM menu_items WHERE id IN (",
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

    // Limity „na przerwę” liczą się dla pary (uczeń, dzień, przerwa).
    // Sprawdzamy je po odczytaniu nazw pozycji, aby komunikat był czytelny.
    if let Some(break_id) = break_id {
        let one_active = setting_i64(&mut transaction, "one_active_order_per_break", 0).await? == 1;
        if one_active {
            let existing: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM orders
                 WHERE user_id = ? AND pickup_date = ? AND break_id = ?
                   AND status = 'ordered'",
            )
            .bind(user.id)
            .bind(&pickup_date_key)
            .bind(break_id)
            .fetch_one(&mut *transaction)
            .await?;
            if existing > 0 {
                transaction.rollback().await?;
                return Err(AppError::conflict(
                    "Masz już aktywne zamówienie na tę przerwę. Odbierz je lub anuluj,                      zanim złożysz kolejne.",
                ));
            }
        }

        let per_break = setting_i64(&mut transaction, "max_item_quantity_per_break", 0).await?;
        if per_break > 0 {
            for (menu_item_id, quantity) in &quantities {
                let already: i64 = sqlx::query_scalar(
                    "SELECT COALESCE(SUM(oi.quantity), 0)
                     FROM orders o
                     JOIN order_items oi ON oi.order_id = o.id
                     WHERE o.user_id = ? AND o.pickup_date = ? AND o.break_id = ?
                       AND o.status IN ('ordered', 'issued')
                       AND oi.menu_item_id = ?",
                )
                .bind(user.id)
                .bind(&pickup_date_key)
                .bind(break_id)
                .bind(menu_item_id)
                .fetch_one(&mut *transaction)
                .await?;
                if already + quantity > per_break {
                    let name = by_id
                        .get(menu_item_id)
                        .map(|item| format!("„{}”", item.name))
                        .unwrap_or_else(|| "wybranej pozycji".to_string());
                    transaction.rollback().await?;
                    return Err(AppError::conflict(format!(
                        "Limit na tę przerwę: maksymalnie {per_break} szt. {name}                          na ucznia. W tym zamówieniu masz już {already} szt."
                    )));
                }
            }
        }
    }

    // Stan magazynowy dotyczy wyłącznie dzisiejszego dnia. Zamówienie na
    // przyszły dzień nie rezerwuje dzisiejszych sztuk — jego stan nie jest
    // kontrolowany (świadome ograniczenie pierwszej wersji).
    let stock_controlled = pickup_date == today;
    let sold_out = if stock_controlled {
        stock::reserve(&mut transaction, &quantities, today).await?
    } else {
        Vec::new()
    };
    if !sold_out.is_empty() {
        transaction.rollback().await?;
        let names = sold_out
            .iter()
            .map(|name| format!("„{name}”"))
            .collect::<Vec<_>>()
            .join(", ");
        return Err(AppError::conflict(format!(
            "Skończyły się: {names}. Odśwież menu i złóż zamówienie ponownie."
        )));
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
          pickup_time, pickup_date, break_id, idempotency_key, created_at, updated_at)
         VALUES (?, ?, ?, 'ordered', ?, 'unpaid', ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(user.id)
    .bind(&code)
    .bind(&qr_token)
    .bind(&payment_method)
    .bind(total_cents)
    .bind(&pickup_time)
    .bind(&pickup_date_key)
    .bind(break_id)
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
             (order_id, menu_item_id, name, price_cents, quantity, stock_reserved)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(order_id)
        .bind(menu_item_id)
        .bind(&item.name)
        .bind(item.price_cents)
        .bind(*quantity)
        .bind(i64::from(
            stock_controlled && item.stock_remaining.is_some(),
        ))
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
        "SELECT status, payment_status, created_at, pickup_date, paid_at, paid_by,
                issued_at, uncollected_at
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
    if current.status != "ordered" {
        transaction.rollback().await?;
        return Err(AppError::conflict(
            "Można anulować tylko zamówienie, które nie zostało jeszcze wydane.",
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
         WHERE id = ? AND user_id = ? AND status = 'ordered'
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

    // Zwrot na stan tylko dla zamówienia z dzisiejszego dnia. Anulowanie
    // zamówienia sprzed północy nie zwiększa dzisiejszego stanu.
    if crate::breaks::is_order_from_today(current.created_at, current.pickup_date.as_deref()) {
        stock::return_order_stock(&mut transaction, id, util::today()).await?;
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
    let row: Option<(String, String, i64, Option<String>)> = sqlx::query_as(
        "SELECT qr_token, status, created_at, pickup_date
         FROM orders WHERE id = ? AND user_id = ?",
    )
    .bind(id)
    .bind(user.id)
    .fetch_optional(&state.db)
    .await?;

    let (token, status, created_at, pickup_date) =
        row.ok_or_else(|| AppError::not_found("Nie znaleziono zamówienia."))?;
    if matches!(status.as_str(), "cancelled" | "issued" | "uncollected") {
        return Err(AppError::conflict(
            "To zamówienie jest już zamknięte — kod QR nie jest aktywny.",
        ));
    }
    if crate::breaks::order_date(created_at, pickup_date.as_deref()) != Some(util::today()) {
        return Err(AppError::conflict(
            "Kod QR jest aktywny tylko w dniu odbioru zamówienia.",
        ));
    }

    // QR zawiera wyłącznie nieprzejrzysty token odbioru: bez loginu, kodu
    // ucznia, ID konta, ceny, URL ani tokenu sesji.
    let qr = QrCode::new(token.as_bytes())
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
            "{ORDER_SELECT}
             WHERE (o.pickup_date = ?)
                OR (o.pickup_date IS NULL AND o.created_at >= ? AND o.created_at < ?)
             ORDER BY o.created_at DESC, o.id DESC, oi.id ASC LIMIT 300"
        ),
        _ => format!(
            "{ORDER_SELECT} WHERE o.status = 'ordered'
             ORDER BY o.created_at ASC, o.id ASC, oi.id ASC LIMIT 300"
        ),
    };

    let rows = if scope == "today" {
        sqlx::query_as::<_, OrderRow>(&sql)
            .bind(stock::date_key(util::today()))
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
            event = "pickup_locked",
            account_id = user.id,
            ip = %client.0,
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

    let token = pickup_token(raw);
    let normalized_code = token.to_uppercase();

    let id: Option<i64> =
        sqlx::query_scalar("SELECT id FROM orders WHERE qr_token = ? OR code = ? LIMIT 1")
            .bind(token)
            .bind(&normalized_code)
            .fetch_optional(&state.db)
            .await?;

    let Some(id) = id else {
        return Err(pickup_failure(
            &state,
            user.id,
            client.0,
            AppError::not_found("Nie znaleziono zamówienia o tym kodzie."),
        ));
    };

    let order = load_order(&state, id).await?;
    if let Some(error) = pickup_rejection(&order) {
        return Err(pickup_failure(&state, user.id, client.0, error));
    }

    state.pickup_limiter.success(user.id, client.0);
    db::audit(
        &state.db,
        Some(user.id),
        "pickup_lookup",
        &format!("Odczyt zamówienia id={id}"),
    )
    .await;

    Ok(Json(serde_json::json!({ "order": order })))
}

/// Akceptuje sam token QR oraz starszy wariant z prefiksem `ZSL:`.
fn pickup_token(raw: &str) -> &str {
    let raw = raw.trim();
    raw.strip_prefix("ZSL:").unwrap_or(raw).trim()
}

/// Odmowa wydania z czytelnym komunikatem. Kod QR jest jednorazowy:
/// po `completed` nie działa ponownie, a kod z innego dnia nie jest ważny.
fn pickup_rejection(order: &OrderView) -> Option<AppError> {
    match order.status.as_str() {
        "issued" => Some(AppError::conflict(
            "To zamówienie zostało już wydane — kod jest jednorazowy.",
        )),
        "cancelled" | "uncollected" => Some(AppError::conflict(
            "To zamówienie zostało anulowane lub nieodebrane.",
        )),
        "ordered" => {
            let day = crate::breaks::order_date(order.created_at, order.pickup_date.as_deref());
            if day != Some(util::today()) {
                Some(AppError::conflict("Kod dotyczy zamówienia z innego dnia."))
            } else {
                None
            }
        }
        _ => Some(AppError::conflict(
            "Zamówienie nie może zostać wydane w tym stanie.",
        )),
    }
}

/// Rejestruje nieudaną próbę i zamienia ją na 429, jeśli limiter jest pełny.
fn pickup_failure(
    state: &AppState,
    user_id: i64,
    ip: std::net::IpAddr,
    error: AppError,
) -> AppError {
    state.pickup_limiter.failure(user_id, ip);
    if let Err(retry_after) = state.pickup_limiter.check(user_id, ip) {
        tracing::warn!(
            event = "pickup_locked",
            account_id = user_id,
            ip = %ip,
            retry_after,
            "Odczyt kodu odbioru zablokowany po nieudanej próbie"
        );
        return AppError::TooManyRequests {
            retry_after,
            message: "Zbyt wiele nieudanych prób odczytu kodu. Odczekaj chwilę.".to_string(),
        };
    }
    error
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
        && !matches!(value.as_str(), "ordered" | "issued" | "cancelled")
    {
        return Err(AppError::bad("Nieprawidłowy status zamówienia."));
    }
    if let Some(value) = &payment_status
        && !matches!(value.as_str(), "unpaid" | "paid")
    {
        return Err(AppError::bad("Nieprawidłowy status płatności."));
    }

    let current: Option<OrderState> = sqlx::query_as(
        "SELECT status, payment_status, created_at, pickup_date, paid_at, paid_by,
                issued_at, uncollected_at
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
        if crate::breaks::order_date(current.created_at, current.pickup_date.as_deref())
            != Some(util::today())
        {
            return Err(AppError::forbidden(
                "Kuchnia może zmieniać tylko zamówienia na dziś.",
            ));
        }
        if matches!(
            current.status.as_str(),
            "issued" | "cancelled" | "uncollected"
        ) {
            return Err(AppError::conflict(
                "To zamówienie jest już zamknięte i nie można go zmienić.",
            ));
        }
        if let Some(value) = &status {
            let allowed =
                current.status == "ordered" && matches!(value.as_str(), "ordered" | "issued");
            if !allowed {
                return Err(AppError::forbidden(
                    "Niedozwolona zmiana statusu. Kuchnia wydaje tylko dzisiejsze zamówienia.",
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
            && matches!(final_status, "cancelled" | "uncollected")
        {
            return Err(AppError::conflict(
                "Anulowanego lub nieodebranego zamówienia nie można opłacić.",
            ));
        }
    }

    if status.as_deref() == Some("issued") && payment_status.is_none() {
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

    // `ready_at` zostaje jako znacznik historyczny (migracja z `ready`);
    // nowe zamówienia korzystają z `issued_at`.
    let new_issued_at = if final_status == "issued" {
        current.issued_at.or(Some(now))
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
             issued_at = ?, uncollected_at = ?, updated_at = ?
         WHERE id = ? AND status = ? AND payment_status = ?
           AND COALESCE(uncollected_at, 0) = ?",
    )
    .bind(&final_status)
    .bind(&final_payment)
    .bind(new_paid_at)
    .bind(new_paid_by)
    .bind(new_issued_at)
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

    // Anulowanie przed wydaniem zwraca stan — tylko admin/uczeń i tylko
    // dla dzisiejszego zamówienia. Wydane i nieodebrane nie wracają.
    let cancelled_before_issue = final_status == "cancelled"
        && current.status == "ordered"
        && current.uncollected_at.is_none();
    if cancelled_before_issue
        && crate::breaks::is_order_from_today(current.created_at, current.pickup_date.as_deref())
    {
        stock::return_order_stock(&mut transaction, id, util::today()).await?;
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

    csv.push_str(
        "\r\nprzerwa;zamowienia;wydane;oplacone;kwota_oplacona_gr;\
         nieoplacone;kwota_nieoplacona_gr;anulowane;nieodebrane\r\n",
    );
    for entry in &report.breaks {
        csv.push_str(&format!(
            "{};{};{};{};{};{};{};{};{}\r\n",
            csv_cell(&entry.break_name),
            entry.totals.orders,
            entry.totals.issued_orders,
            entry.totals.paid_orders,
            entry.totals.paid_cents,
            entry.totals.unpaid_orders,
            entry.totals.unpaid_cents,
            entry.totals.cancelled_orders,
            entry.totals.uncollected_orders,
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

    // Raport dnia grupuje zamówienia po dniu odbioru. Stare wiersze bez
    // `pickup_date` nadal liczą się po dniu złożenia, więc historia i testy
    // granicy doby pozostają bez zmian.
    let rows = sqlx::query_as::<_, ReportOrderRow>(
        "SELECT o.status, o.payment_method, o.payment_status, o.total_cents,
                o.break_id, b.name AS break_name
         FROM orders o
         LEFT JOIN breaks b ON b.id = o.break_id
         WHERE o.pickup_date = ?
            OR (o.pickup_date IS NULL AND o.created_at >= ? AND o.created_at < ?)
         ORDER BY b.sort_order ASC, b.start_time ASC, o.created_at ASC, o.id ASC",
    )
    .bind(stock::date_key(date))
    .bind(start)
    .bind(end)
    .fetch_all(&state.db)
    .await?;

    let summary = aggregate_totals(rows.iter());
    let cash = aggregate_totals(rows.iter().filter(|row| row.payment_method == "cash"));
    let card = aggregate_totals(rows.iter().filter(|row| row.payment_method == "card"));
    let break_rows = aggregate_breaks(&rows);

    stock::reset_daily_stock(&state.db, util::today()).await?;
    let stock_rows = sqlx::query_as::<_, StockReportRow>(
        "SELECT m.id AS menu_item_id, m.name, m.stock_remaining, m.stock_default,
                COALESCE(SUM(CASE
                    WHEN o.id IS NOT NULL
                     AND o.status NOT IN ('cancelled', 'uncollected')
                    THEN oi.quantity
                    ELSE 0
                END), 0) AS sold
         FROM menu_items m
         LEFT JOIN order_items oi ON oi.menu_item_id = m.id
         LEFT JOIN orders o ON o.id = oi.order_id
              AND (o.pickup_date = ?
                   OR (o.pickup_date IS NULL AND o.created_at >= ? AND o.created_at < ?))
         GROUP BY m.id
         ORDER BY m.sort_order ASC, m.name ASC",
    )
    .bind(stock::date_key(date))
    .bind(start)
    .bind(end)
    .fetch_all(&state.db)
    .await?;

    Ok(DailyReport {
        date: date.format("%Y-%m-%d").to_string(),
        summary,
        cash,
        card,
        breaks: break_rows,
        stock: stock_rows,
    })
}

fn aggregate_totals<'a>(rows: impl Iterator<Item = &'a ReportOrderRow>) -> PaymentTotals {
    let mut totals = PaymentTotals::default();
    for row in rows {
        add_order(&mut totals, row);
    }
    totals
}

fn aggregate_breaks(rows: &[ReportOrderRow]) -> Vec<BreakReportRow> {
    let mut result: Vec<BreakReportRow> = Vec::new();
    for row in rows {
        let name = row
            .break_name
            .clone()
            .unwrap_or_else(|| "Bez przerwy".to_string());
        let index = match result
            .iter()
            .position(|entry| entry.break_id == row.break_id && entry.break_name == name)
        {
            Some(index) => index,
            None => {
                result.push(BreakReportRow {
                    break_id: row.break_id,
                    break_name: name,
                    totals: PaymentTotals::default(),
                });
                result.len() - 1
            }
        };
        add_order(&mut result[index].totals, row);
    }
    result
}

fn add_order(totals: &mut PaymentTotals, row: &ReportOrderRow) {
    totals.orders += 1;

    let uncollected = row.status == "uncollected";
    let cancelled = row.status == "cancelled";
    // Kategorie liczby zamówień są rozłączne:
    // paid + unpaid + cancelled + uncollected = orders.
    if uncollected {
        totals.uncollected_orders += 1;
    } else if cancelled {
        totals.cancelled_orders += 1;
    } else if row.status == "issued" {
        totals.issued_orders += 1;
    } else if row.status == "ordered" {
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

/// Oznacza jako nieodebrane zamówienia, których termin odbioru minął.
///
/// * Zamówienie z przerwą na dziś staje się `uncollected` dopiero po
///   **końcu przerwy + `pickup_limit_minutes`** (strefa szkoły). Nie
///   liczymy od `created_at` — zamówienie na późniejszą przerwę złożone
///   rano nie może zostać zamknięte przed swoim oknem.
/// * Zamówienia z poprzednich dni zamykamy zawsze (także te bez przerwy
///   i historyczne wiersze bez `pickup_date`).
/// * Dzisiejsze zamówienie bez przerwy (stare dane) nie jest zamykane
///   automatycznie — brak okna, od którego można liczyć termin.
/// * `pickup_limit_minutes = 0` wyłącza auto-oznaczanie całkowicie.
///
/// Funkcja jest odporna na wyścig z kuchnią dzięki warunkowemu
/// `UPDATE ... WHERE status = 'ordered'`.
pub async fn mark_uncollected(state: &Arc<AppState>) -> Result<u64, sqlx::Error> {
    mark_uncollected_at(&state.db, util::now()).await
}

async fn mark_uncollected_at(db: &SqlitePool, now: i64) -> Result<u64, sqlx::Error> {
    let limit: i64 = db::get_setting_or(db, "pickup_limit_minutes", "120")
        .await
        .parse()
        .unwrap_or(120);
    if limit <= 0 {
        return Ok(0);
    }

    let today = util::local_date(now).unwrap_or_else(util::today);
    let today_key = stock::date_key(today);
    let start_of_today = util::start_of_local_date(today);

    // Kandydaci: poprzednie dni (zawsze zamykane) oraz dzisiejsze
    // zamówienia z przerwą (termin liczymy w Rust po `end_time`). Dzisiejsze
    // zamówienia bez przerwy są pomijane już w SQL.
    let rows: Vec<(i64, i64, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT o.id, o.created_at, o.pickup_date, b.end_time
         FROM orders o
         LEFT JOIN breaks b ON b.id = o.break_id
         WHERE o.status = 'ordered' AND o.uncollected_at IS NULL
           AND (o.pickup_date < ?
                OR (o.pickup_date = ? AND o.break_id IS NOT NULL)
                OR (o.pickup_date IS NULL AND o.created_at < ?))
         ORDER BY o.created_at ASC, o.id ASC
         LIMIT 200",
    )
    .bind(&today_key)
    .bind(&today_key)
    .bind(start_of_today)
    .fetch_all(db)
    .await?;

    let mut ids: Vec<i64> = Vec::new();
    for (id, created_at, pickup_date, break_end) in rows {
        let Some(date) = crate::breaks::order_date(created_at, pickup_date.as_deref()) else {
            continue;
        };
        if date > today {
            continue;
        }
        let due = if date < today {
            true
        } else {
            break_end
                .as_deref()
                .and_then(|end| crate::breaks::uncollected_deadline(today, end, limit))
                .map(|deadline| now >= deadline)
                .unwrap_or(false)
        };
        if due {
            ids.push(id);
        }
    }

    let mut marked = 0_u64;
    let mut marked_ids: Vec<i64> = Vec::new();
    for id in ids {
        let result = sqlx::query(
            "UPDATE orders
             SET status = 'uncollected', uncollected_at = ?, updated_at = ?
             WHERE id = ? AND status = 'ordered' AND uncollected_at IS NULL",
        )
        .bind(now)
        .bind(now)
        .bind(id)
        .execute(db)
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
            db,
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
                orders.push(OrderView {
                    id: row.id,
                    code: row.code,
                    status: row.status,
                    payment_method: row.payment_method,
                    payment_status: row.payment_status,
                    total_cents: row.total_cents,
                    pickup_time: row.pickup_time,
                    pickup_date: row.pickup_date,
                    break_id: row.break_id,
                    break_name: row.break_name,
                    created_at: row.created_at,
                    updated_at: row.updated_at,
                    paid_at: row.paid_at,
                    paid_by: row.paid_by_login,
                    issued_at: row.issued_at,
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

/// Dzień odbioru z żądania. Brak/puste = dziś; dozwolony zakres to
/// `[dziś, dziś + order_days_ahead]`.
fn resolve_pickup_date(
    requested: Option<&str>,
    today: NaiveDate,
    order_days_ahead: i64,
) -> Result<NaiveDate, AppError> {
    let raw = requested.map(str::trim).unwrap_or("");
    if raw.is_empty() {
        return Ok(today);
    }
    let date = NaiveDate::parse_from_str(raw, "%Y-%m-%d")
        .map_err(|_| AppError::bad("Dzień odbioru musi być w formacie RRRR-MM-DD."))?;
    let last = today + Duration::days(order_days_ahead);
    if date < today || date > last {
        return Err(AppError::bad(if order_days_ahead == 0 {
            "Zamawiać można tylko na dziś.".to_string()
        } else {
            format!("Dzień odbioru musi mieścić się między {today} a {last}.")
        }));
    }
    Ok(date)
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

    fn report_row(status: &str, method: &str, payment: &str, total_cents: i64) -> ReportOrderRow {
        ReportOrderRow {
            status: status.to_string(),
            payment_method: method.to_string(),
            payment_status: payment.to_string(),
            total_cents,
            break_id: None,
            break_name: None,
        }
    }

    #[test]
    fn report_totals_match_orders_and_split_payment_methods() {
        let rows = [
            report_row("issued", "cash", "paid", 1500),
            report_row("issued", "card", "paid", 2500),
            report_row("ordered", "cash", "unpaid", 800),
            report_row("cancelled", "cash", "paid", 999),
            report_row("uncollected", "card", "paid", 1200),
        ];

        let summary = aggregate_totals(rows.iter());
        assert_eq!(summary.orders, 5);
        assert_eq!(summary.issued_orders, 2);
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
        let rows = [report_row("uncollected", "cash", "paid", 1500)];

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

    fn order_with(status: &str, created_at: i64) -> OrderView {
        OrderView {
            id: 1,
            code: "K7M2QP".to_string(),
            status: status.to_string(),
            payment_method: "cash".to_string(),
            payment_status: "unpaid".to_string(),
            total_cents: 1200,
            pickup_time: "12:30".to_string(),
            pickup_date: None,
            break_id: None,
            break_name: None,
            created_at,
            updated_at: created_at,
            paid_at: None,
            paid_by: None,
            issued_at: None,
            uncollected_at: None,
            student: "s-test".to_string(),
            student_code: "UCZ-TEST".to_string(),
            items: Vec::new(),
        }
    }

    #[test]
    fn pickup_accepts_only_ordered_order_from_today() {
        assert!(pickup_rejection(&order_with("ordered", util::now())).is_none());

        let yesterday = util::start_of_today() - 60;
        assert!(pickup_rejection(&order_with("ordered", yesterday)).is_some());
        assert!(pickup_rejection(&order_with("issued", util::now())).is_some());
        assert!(pickup_rejection(&order_with("cancelled", util::now())).is_some());
        assert!(pickup_rejection(&order_with("uncollected", util::now())).is_some());
    }

    #[test]
    fn pickup_token_accepts_plain_and_legacy_prefix() {
        assert_eq!(pickup_token("abcdef123456"), "abcdef123456");
        assert_eq!(pickup_token("ZSL:abcdef123456"), "abcdef123456");
        assert_eq!(pickup_token("  ZSL:abcdef123456  "), "abcdef123456");
        assert_eq!(pickup_token("  K7M2QP  "), "K7M2QP");
    }
    use std::str::FromStr;
    use std::sync::atomic::{AtomicU64, Ordering};

    static UNCOLLECTED_COUNTER: AtomicU64 = AtomicU64::new(0);

    async fn uncollected_pool(name: &str) -> (SqlitePool, std::path::PathBuf) {
        use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
        let unique = UNCOLLECTED_COUNTER.fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!(
            "zsl-uncollected-{name}-{}-{unique}.db",
            std::process::id()
        ));
        let options = SqliteConnectOptions::from_str(&format!("sqlite://{}", path.display()))
            .unwrap()
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .busy_timeout(std::time::Duration::from_secs(10));
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(options)
            .await
            .unwrap();
        crate::db::MIGRATOR.run(&pool).await.unwrap();
        (pool, path)
    }

    async fn uncollected_close(pool: SqlitePool, path: std::path::PathBuf) {
        pool.close().await;
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
        }
    }

    async fn set_pickup_limit(pool: &SqlitePool, minutes: i64) {
        sqlx::query(
            "INSERT INTO settings (key, value) VALUES ('pickup_limit_minutes', ?)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        )
        .bind(minutes.to_string())
        .execute(pool)
        .await
        .unwrap();
    }

    async fn insert_break(pool: &SqlitePool, start: &str, end: &str) -> i64 {
        sqlx::query_scalar(
            "INSERT INTO breaks (name, start_time, end_time, active, sort_order,
                                 created_at, updated_at)
             VALUES ('Testowa przerwa', ?, ?, 1, 0, 1, 1)
             RETURNING id",
        )
        .bind(start)
        .bind(end)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    async fn insert_ordered(
        pool: &SqlitePool,
        code: &str,
        created_at: i64,
        pickup_date: Option<&str>,
        break_id: Option<i64>,
    ) -> i64 {
        sqlx::query_scalar(
            "INSERT INTO orders
             (user_id, code, qr_token, status, payment_method, payment_status,
              total_cents, pickup_time, pickup_date, break_id, created_at, updated_at)
             VALUES (NULL, ?, ?, 'ordered', 'cash', 'unpaid', 1000, '', ?, ?, ?, ?)
             RETURNING id",
        )
        .bind(code)
        .bind(format!("{code}-token"))
        .bind(pickup_date)
        .bind(break_id)
        .bind(created_at)
        .bind(created_at)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    async fn order_status(pool: &SqlitePool, id: i64) -> String {
        sqlx::query_scalar("SELECT status FROM orders WHERE id = ?")
            .bind(id)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    fn at(date: NaiveDate, time: &str) -> i64 {
        util::local_datetime_timestamp(date, time).expect("poprawna godzina")
    }

    #[tokio::test]
    async fn later_break_is_not_closed_before_its_end_plus_limit() {
        let (pool, path) = uncollected_pool("later-break").await;
        set_pickup_limit(&pool, 30).await;
        let date = NaiveDate::from_ymd_opt(2026, 11, 17).unwrap();
        let break_id = insert_break(&pool, "11:30", "11:50").await;
        let id = insert_ordered(
            &pool,
            "UNC001",
            at(date, "09:58"),
            Some("2026-11-17"),
            Some(break_id),
        )
        .await;

        // 32 minuty po złożeniu, ale przed przerwą — limit od `created_at`
        // nie może zamknąć zamówienia.
        assert_eq!(
            mark_uncollected_at(&pool, at(date, "10:30")).await.unwrap(),
            0
        );
        assert_eq!(order_status(&pool, id).await, "ordered");

        // Koniec przerwy jeszcze nie minął.
        assert_eq!(
            mark_uncollected_at(&pool, at(date, "11:49")).await.unwrap(),
            0
        );
        // Koniec przerwy + 29 minut — nadal otwarte.
        assert_eq!(
            mark_uncollected_at(&pool, at(date, "12:19")).await.unwrap(),
            0
        );
        assert_eq!(order_status(&pool, id).await, "ordered");

        // Dokładnie koniec przerwy + 30 minut.
        assert_eq!(
            mark_uncollected_at(&pool, at(date, "12:20")).await.unwrap(),
            1
        );
        assert_eq!(order_status(&pool, id).await, "uncollected");
        // Druga iteracja nie podwaja licznika.
        assert_eq!(
            mark_uncollected_at(&pool, at(date, "12:21")).await.unwrap(),
            0
        );

        uncollected_close(pool, path).await;
    }

    #[tokio::test]
    async fn previous_days_are_always_closed_but_today_without_break_is_not() {
        let (pool, path) = uncollected_pool("previous-days").await;
        set_pickup_limit(&pool, 30).await;
        let break_id = insert_break(&pool, "11:30", "11:50").await;
        let today = NaiveDate::from_ymd_opt(2026, 11, 17).unwrap();
        let yesterday = NaiveDate::from_ymd_opt(2026, 11, 16).unwrap();
        let now = at(today, "09:00");

        let with_break = insert_ordered(
            &pool,
            "UNC101",
            at(yesterday, "12:00"),
            Some("2026-11-16"),
            Some(break_id),
        )
        .await;
        let without_break =
            insert_ordered(&pool, "UNC102", at(yesterday, "12:00"), None, None).await;
        let today_no_break = insert_ordered(&pool, "UNC103", at(today, "08:00"), None, None).await;
        let future = insert_ordered(
            &pool,
            "UNC104",
            at(today, "08:00"),
            Some("2026-11-18"),
            Some(break_id),
        )
        .await;

        assert_eq!(mark_uncollected_at(&pool, now).await.unwrap(), 2);
        assert_eq!(order_status(&pool, with_break).await, "uncollected");
        assert_eq!(order_status(&pool, without_break).await, "uncollected");
        assert_eq!(order_status(&pool, today_no_break).await, "ordered");
        assert_eq!(order_status(&pool, future).await, "ordered");

        uncollected_close(pool, path).await;
    }

    #[tokio::test]
    async fn deadline_uses_school_timezone_on_dst_days() {
        let (pool, path) = uncollected_pool("dst").await;
        set_pickup_limit(&pool, 30).await;
        let break_id = insert_break(&pool, "11:30", "11:50").await;

        // Wiosna: 29.03.2026, 12:20 CEST = 10:20 UTC.
        let spring = NaiveDate::from_ymd_opt(2026, 3, 29).unwrap();
        let spring_id = insert_ordered(
            &pool,
            "UNC201",
            at(spring, "09:00"),
            Some("2026-03-29"),
            Some(break_id),
        )
        .await;
        assert_eq!(
            mark_uncollected_at(&pool, at(spring, "12:19"))
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            mark_uncollected_at(&pool, at(spring, "12:20"))
                .await
                .unwrap(),
            1
        );
        assert_eq!(order_status(&pool, spring_id).await, "uncollected");

        // Jesień: 25.10.2026, 12:20 CET = 11:20 UTC.
        let autumn = NaiveDate::from_ymd_opt(2026, 10, 25).unwrap();
        let autumn_id = insert_ordered(
            &pool,
            "UNC202",
            at(autumn, "09:00"),
            Some("2026-10-25"),
            Some(break_id),
        )
        .await;
        assert_eq!(
            mark_uncollected_at(&pool, at(autumn, "12:19"))
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            mark_uncollected_at(&pool, at(autumn, "12:20"))
                .await
                .unwrap(),
            1
        );
        assert_eq!(order_status(&pool, autumn_id).await, "uncollected");

        // Termin w UTC zależy od offsetu dnia: CEST (UTC+2) vs CET (UTC+1).
        use chrono::TimeZone;
        assert_eq!(
            at(spring, "12:20"),
            chrono::Utc
                .with_ymd_and_hms(2026, 3, 29, 10, 20, 0)
                .unwrap()
                .timestamp()
        );
        assert_eq!(
            at(autumn, "12:20"),
            chrono::Utc
                .with_ymd_and_hms(2026, 10, 25, 11, 20, 0)
                .unwrap()
                .timestamp()
        );

        uncollected_close(pool, path).await;
    }

    #[tokio::test]
    async fn pickup_limit_zero_disables_auto_marking_entirely() {
        let (pool, path) = uncollected_pool("disabled").await;
        set_pickup_limit(&pool, 0).await;
        let yesterday = NaiveDate::from_ymd_opt(2026, 11, 16).unwrap();
        let today = NaiveDate::from_ymd_opt(2026, 11, 17).unwrap();
        let id = insert_ordered(
            &pool,
            "UNC301",
            at(yesterday, "12:00"),
            Some("2026-11-16"),
            None,
        )
        .await;
        assert_eq!(
            mark_uncollected_at(&pool, at(today, "09:00"))
                .await
                .unwrap(),
            0
        );
        assert_eq!(order_status(&pool, id).await, "ordered");

        uncollected_close(pool, path).await;
    }

    #[test]
    fn pickup_date_stays_within_configured_window() {
        let today = util::today();
        let today_key = stock::date_key(today);
        let tomorrow_key = stock::date_key(today + Duration::days(1));
        assert_eq!(resolve_pickup_date(None, today, 0).unwrap(), today);
        assert_eq!(resolve_pickup_date(Some(""), today, 0).unwrap(), today);
        assert!(resolve_pickup_date(Some(&today_key), today, 0).is_ok());
        assert!(resolve_pickup_date(Some(&tomorrow_key), today, 0).is_err());
        assert!(resolve_pickup_date(Some(&tomorrow_key), today, 1).is_ok());
        assert!(resolve_pickup_date(Some("2030-02-30"), today, 7).is_err());
    }
}
