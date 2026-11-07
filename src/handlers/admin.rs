use crate::auth::AuthUser;
use crate::db;
use crate::error::AppError;
use crate::handlers::auth::hash_password_async;
use crate::security;
use crate::state::AppState;
use crate::util;
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use chrono::Local;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, QueryBuilder, Row, Sqlite};
use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

#[derive(FromRow, Serialize)]
pub struct AdminUserView {
    pub id: i64,
    pub login: String,
    pub role: String,
    pub display_code: String,
    pub active: bool,
    pub must_change_password: bool,
    pub created_at: i64,
    pub last_login_at: Option<i64>,
}

#[derive(Deserialize)]
pub struct UsersQuery {
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub q: Option<String>,
    #[serde(default)]
    pub active: Option<bool>,
}

#[derive(Deserialize)]
pub struct GenerateRequest {
    pub role: String,
    pub count: u32,
}

#[derive(Clone, Serialize)]
pub struct GeneratedAccount {
    pub login: String,
    pub password: String,
    pub role: String,
    pub display_code: String,
}

#[derive(Deserialize)]
pub struct ResetPasswordRequest {
    // Puste ciało jest dozwolone; pole istnieje dla przyszłej kompatybilności.
    #[serde(default)]
    pub _confirm: Option<bool>,
}

#[derive(Deserialize)]
pub struct UpdateUserRequest {
    #[serde(default)]
    pub active: Option<bool>,
    #[serde(default)]
    pub role: Option<String>,
}

#[derive(Deserialize)]
pub struct SettingsRequest {
    #[serde(default)]
    pub cafeteria_name: Option<String>,
    #[serde(default)]
    pub ordering_enabled: Option<bool>,
    #[serde(default)]
    pub payment_info: Option<String>,
    #[serde(default)]
    pub max_open_orders: Option<i64>,
    #[serde(default)]
    pub max_order_items: Option<i64>,
}

#[derive(Deserialize)]
pub struct AuditQuery {
    #[serde(default)]
    pub limit: Option<i64>,
}

#[derive(FromRow, Serialize)]
pub struct AuditEntry {
    pub id: i64,
    pub action: String,
    pub details: String,
    pub created_at: i64,
    pub actor_login: Option<String>,
}

#[derive(Deserialize)]
pub struct ActivityQuery {
    #[serde(default)]
    pub days: Option<i64>,
}

#[derive(Serialize)]
pub struct DailyActivity {
    pub date: String,
    pub orders: i64,
    pub revenue_cents: i64,
    pub completed: i64,
    pub cancelled: i64,
}

#[derive(Serialize)]
pub struct HourlyActivity {
    pub hour: u32,
    pub orders: i64,
}

#[derive(Serialize)]
pub struct TopItem {
    pub name: String,
    pub quantity: i64,
    pub revenue_cents: i64,
}

#[derive(FromRow)]
struct ExportUserRow {
    login: String,
    role: String,
    display_code: String,
    active: bool,
    must_change_password: bool,
    created_at: i64,
    last_login_at: Option<i64>,
}

fn append_user_filters(builder: &mut QueryBuilder<'_, Sqlite>, query: &UsersQuery) {
    if let Some(role) = query.role.as_deref() {
        if matches!(role, "student" | "kitchen" | "admin") {
            builder.push(" AND role = ");
            builder.push_bind(role.to_string());
        }
    }

    if let Some(active) = query.active {
        builder.push(" AND active = ");
        builder.push_bind(active);
    }

    if let Some(search) = query.q.as_deref() {
        let trimmed = search.trim();
        if !trimmed.is_empty() {
            let pattern = format!("%{}%", trimmed.chars().take(40).collect::<String>());
            builder.push(" AND (login LIKE ");
            builder.push_bind(pattern.clone());
            builder.push(" OR display_code LIKE ");
            builder.push_bind(pattern);
            builder.push(")");
        }
    }
}

pub async fn stats(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, AppError> {
    let total_users: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(&state.db)
        .await?;
    let students: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE role = 'student' AND active = 1")
            .fetch_one(&state.db)
            .await?;
    let kitchen: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE role = 'kitchen' AND active = 1")
            .fetch_one(&state.db)
            .await?;
    let admins: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE role = 'admin' AND active = 1")
            .fetch_one(&state.db)
            .await?;
    let orders_today: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM orders WHERE created_at >= ?",
    )
    .bind(util::start_of_today())
    .fetch_one(&state.db)
    .await?;
    let revenue_today: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(total_cents), 0) FROM orders
         WHERE created_at >= ? AND payment_status = 'paid' AND status <> 'cancelled'",
    )
    .bind(util::start_of_today())
    .fetch_one(&state.db)
    .await?;
    let open_orders: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM orders WHERE status IN ('new','preparing','ready')",
    )
    .fetch_one(&state.db)
    .await?;
    let awaiting_payment: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM orders
         WHERE payment_status = 'unpaid' AND status <> 'cancelled'",
    )
    .fetch_one(&state.db)
    .await?;
    let menu_available: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM menu_items WHERE available = 1")
            .fetch_one(&state.db)
            .await?;
    let active_sessions: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM sessions WHERE expires_at > ?")
            .bind(util::now())
            .fetch_one(&state.db)
            .await?;

    Ok(Json(serde_json::json!({
        "total_users": total_users,
        "students": students,
        "kitchen": kitchen,
        "admins": admins,
        "orders_today": orders_today,
        "revenue_today_cents": revenue_today,
        "open_orders": open_orders,
        "awaiting_payment": awaiting_payment,
        "menu_available": menu_available,
        "active_sessions": active_sessions,
    })))
}

pub async fn activity(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ActivityQuery>,
) -> Result<Json<serde_json::Value>, AppError> {
    let days = query.days.unwrap_or(14).clamp(1, 90);
    let today = Local::now().date_naive();
    let first_date = today - chrono::Duration::days(days - 1);
    let range_start = util::start_of_local_date(first_date);

    let rows = sqlx::query(
        "SELECT created_at, status, payment_status, total_cents
         FROM orders WHERE created_at >= ? ORDER BY created_at ASC",
    )
    .bind(range_start)
    .fetch_all(&state.db)
    .await?;

    let mut daily: Vec<DailyActivity> = (0..days)
        .map(|offset| {
            let date = first_date + chrono::Duration::days(offset);
            DailyActivity {
                date: date.format("%Y-%m-%d").to_string(),
                orders: 0,
                revenue_cents: 0,
                completed: 0,
                cancelled: 0,
            }
        })
        .collect();
    let daily_index: std::collections::HashMap<String, usize> = daily
        .iter()
        .enumerate()
        .map(|(index, point)| (point.date.clone(), index))
        .collect();

    let mut hourly: Vec<HourlyActivity> = (0..24)
        .map(|hour| HourlyActivity { hour, orders: 0 })
        .collect();
    let mut status_counts: BTreeMap<String, i64> = ["new", "preparing", "ready", "completed", "cancelled"]
        .into_iter()
        .map(|status| (status.to_string(), 0))
        .collect();
    let mut payment_counts: BTreeMap<String, i64> = ["unpaid", "paid"]
        .into_iter()
        .map(|status| (status.to_string(), 0))
        .collect();

    let mut non_cancelled_total: i64 = 0;
    let mut non_cancelled_orders: i64 = 0;

    for row in rows {
        let created_at: i64 = row.try_get("created_at").map_err(AppError::from)?;
        let status: String = row.try_get("status").map_err(AppError::from)?;
        let payment_status: String = row.try_get("payment_status").map_err(AppError::from)?;
        let total_cents: i64 = row.try_get("total_cents").map_err(AppError::from)?;

        *status_counts.entry(status.clone()).or_insert(0) += 1;
        *payment_counts
            .entry(payment_status.clone())
            .or_insert(0) += 1;

        if status != "cancelled" {
            non_cancelled_orders += 1;
            non_cancelled_total += total_cents;
        }

        let Some(date) = util::local_date(created_at) else {
            continue;
        };

        let key = date.format("%Y-%m-%d").to_string();
        if let Some(&index) = daily_index.get(&key) {
            let point = &mut daily[index];
            point.orders += 1;
            if status == "completed" {
                point.completed += 1;
            }
            if status == "cancelled" {
                point.cancelled += 1;
            }
            if payment_status == "paid" && status != "cancelled" {
                point.revenue_cents += total_cents;
            }
        }

        if date == today {
            if let Some(hour) = util::local_hour(created_at) {
                if let Some(point) = hourly.get_mut(hour as usize) {
                    point.orders += 1;
                }
            }
        }
    }

    let top_rows = sqlx::query(
        "SELECT oi.name AS name, SUM(oi.quantity) AS quantity,
                SUM(oi.quantity * oi.price_cents) AS revenue_cents
         FROM order_items oi
         JOIN orders o ON o.id = oi.order_id
         WHERE o.created_at >= ? AND o.status <> 'cancelled'
         GROUP BY oi.name
         ORDER BY quantity DESC, revenue_cents DESC
         LIMIT 8",
    )
    .bind(range_start)
    .fetch_all(&state.db)
    .await?;

    let mut top_items = Vec::with_capacity(top_rows.len());
    for row in top_rows {
        top_items.push(TopItem {
            name: row.try_get("name").map_err(AppError::from)?,
            quantity: row.try_get("quantity").map_err(AppError::from)?,
            revenue_cents: row.try_get("revenue_cents").map_err(AppError::from)?,
        });
    }

    let total_orders: i64 = daily.iter().map(|point| point.orders).sum();
    let total_completed: i64 = daily.iter().map(|point| point.completed).sum();
    let total_cancelled: i64 = daily.iter().map(|point| point.cancelled).sum();
    let total_revenue: i64 = daily.iter().map(|point| point.revenue_cents).sum();
    let average_order_cents = if non_cancelled_orders > 0 {
        non_cancelled_total / non_cancelled_orders
    } else {
        0
    };

    Ok(Json(serde_json::json!({
        "days": days,
        "from": first_date.format("%Y-%m-%d").to_string(),
        "to": today.format("%Y-%m-%d").to_string(),
        "daily": daily,
        "hourly_today": hourly,
        "status_counts": status_counts,
        "payment_counts": payment_counts,
        "top_items": top_items,
        "totals": {
            "orders": total_orders,
            "completed": total_completed,
            "cancelled": total_cancelled,
            "revenue_cents": total_revenue,
            "non_cancelled_orders": non_cancelled_orders,
            "average_order_cents": average_order_cents,
        }
    })))
}

pub async fn users(
    State(state): State<Arc<AppState>>,
    Query(query): Query<UsersQuery>,
) -> Result<Json<Vec<AdminUserView>>, AppError> {
    let mut builder = QueryBuilder::<Sqlite>::new(
        "SELECT id, login, role, display_code, active, must_change_password, created_at, last_login_at
         FROM users WHERE 1 = 1",
    );
    append_user_filters(&mut builder, &query);

    builder.push(" ORDER BY created_at DESC LIMIT 500");
    let rows = builder
        .build_query_as::<AdminUserView>()
        .fetch_all(&state.db)
        .await?;

    Ok(Json(rows))
}

pub async fn generate_users(
    State(state): State<Arc<AppState>>,
    Extension(actor): Extension<AuthUser>,
    Json(body): Json<GenerateRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    if !matches!(body.role.as_str(), "student" | "kitchen" | "admin") {
        return Err(AppError::bad("Nieprawidłowa rola użytkownika."));
    }
    if !(1..=100).contains(&body.count) {
        return Err(AppError::bad(
            "Jednorazowo można wygenerować od 1 do 100 kont.",
        ));
    }

    let existing: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(&state.db)
        .await?;
    if existing + i64::from(body.count) > 20_000 {
        return Err(AppError::conflict(
            "Osiągnięto limit liczby kont w systemie.",
        ));
    }

    let (login_prefix, display_prefix) = match body.role.as_str() {
        "student" => ("s", "UCZ"),
        "kitchen" => ("k", "KUC"),
        _ => ("a", "ADM"),
    };

    let mut accounts = Vec::with_capacity(body.count as usize);
    let mut used_logins = HashSet::new();
    let mut used_codes = HashSet::new();

    for _ in 0..body.count {
        let login = loop {
            let candidate = security::generate_login(login_prefix);
            if used_logins.insert(candidate.clone()) {
                break candidate;
            }
        };
        let display_code = loop {
            let candidate = security::generate_display_code(display_prefix);
            if used_codes.insert(candidate.clone()) {
                break candidate;
            }
        };
        accounts.push(GeneratedAccount {
            login,
            password: security::random_password(14),
            role: body.role.clone(),
            display_code,
        });
    }

    let accounts_for_hash = accounts.clone();
    let hashes = tokio::task::spawn_blocking(move || {
        accounts_for_hash
            .iter()
            .map(|account| security::hash_password(&account.password))
            .collect::<Result<Vec<_>, _>>()
    })
    .await
    .map_err(|error| AppError::internal(format!("spawn_blocking: {error}")))?
    .map_err(AppError::from)?;

    let now = util::now();
    let mut transaction = state.db.begin().await?;
    for (account, hash) in accounts.iter().zip(hashes.iter()) {
        sqlx::query(
            "INSERT INTO users
             (login, password_hash, role, display_code, active, must_change_password, created_at)
             VALUES (?, ?, ?, ?, 1, 1, ?)",
        )
        .bind(&account.login)
        .bind(hash)
        .bind(&account.role)
        .bind(&account.display_code)
        .bind(now)
        .execute(&mut *transaction)
        .await?;
    }
    transaction.commit().await?;

    db::audit(
        &state.db,
        Some(actor.id),
        "users_generated",
        &format!("Rola={}, liczba={}", body.role, body.count),
    )
    .await;
    tracing::info!(
        actor_id = actor.id,
        role = %body.role,
        count = body.count,
        "Wygenerowano konta"
    );

    Ok(Json(serde_json::json!({
        "accounts": accounts,
        "note": "Hasła są widoczne tylko teraz. Zapisz je i przekaż uczniom w bezpieczny sposób."
    })))
}

pub async fn update_user(
    State(state): State<Arc<AppState>>,
    Extension(actor): Extension<AuthUser>,
    Path(id): Path<i64>,
    Json(body): Json<UpdateUserRequest>,
) -> Result<Json<AdminUserView>, AppError> {
    if body.active.is_none() && body.role.is_none() {
        return Err(AppError::bad("Brak zmian do zapisania."));
    }
    if let Some(role) = body.role.as_deref() {
        if !matches!(role, "student" | "kitchen" | "admin") {
            return Err(AppError::bad("Nieprawidłowa rola użytkownika."));
        }
    }

    let current: Option<(String, bool)> =
        sqlx::query_as("SELECT role, active FROM users WHERE id = ?")
            .bind(id)
            .fetch_optional(&state.db)
            .await?;
    let (current_role, current_active) =
        current.ok_or_else(|| AppError::not_found("Nie znaleziono użytkownika."))?;

    if id == actor.id {
        let self_deactivation = body.active == Some(false);
        let self_role_change = body
            .role
            .as_deref()
            .is_some_and(|role| role != current_role);
        if self_deactivation || self_role_change {
            return Err(AppError::bad(
                "Nie możesz odebrać uprawnień własnemu kontu.",
            ));
        }
    }

    let sessions_to_revoke = body.active == Some(false)
        || body
            .role
            .as_deref()
            .is_some_and(|role| role != current_role);

    sqlx::query("UPDATE users SET active = COALESCE(?, active), role = COALESCE(?, role) WHERE id = ?")
        .bind(body.active)
        .bind(body.role.as_deref())
        .bind(id)
        .execute(&state.db)
        .await?;

    if sessions_to_revoke {
        sqlx::query("DELETE FROM sessions WHERE user_id = ?")
            .bind(id)
            .execute(&state.db)
            .await?;
    }

    db::audit(
        &state.db,
        Some(actor.id),
        "user_updated",
        &format!(
            "Konto id={id}, aktywne {} -> {}, rola {current_role} -> {}",
            current_active,
            body.active.unwrap_or(current_active),
            body.role.as_deref().unwrap_or(current_role.as_str())
        ),
    )
    .await;
    tracing::info!(
        actor_id = actor.id,
        target_id = id,
        "Zaktualizowano konto"
    );

    sqlx::query_as::<_, AdminUserView>(
        "SELECT id, login, role, display_code, active, must_change_password, created_at, last_login_at
         FROM users WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .map(Json)
    .ok_or_else(|| AppError::not_found("Nie znaleziono użytkownika."))
}

pub async fn delete_user(
    State(state): State<Arc<AppState>>,
    Extension(actor): Extension<AuthUser>,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, AppError> {
    if id == actor.id {
        return Err(AppError::bad(
            "Nie możesz usunąć własnego konta. Użyj innego konta administratora.",
        ));
    }

    let target: Option<(String, String)> =
        sqlx::query_as("SELECT login, role FROM users WHERE id = ?")
            .bind(id)
            .fetch_optional(&state.db)
            .await?;
    let (login, role) = target.ok_or_else(|| AppError::not_found("Nie znaleziono użytkownika."))?;

    if role == "admin" {
        let other_active_admins: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM users WHERE role = 'admin' AND active = 1 AND id <> ?",
        )
        .bind(id)
        .fetch_one(&state.db)
        .await?;
        if other_active_admins == 0 {
            return Err(AppError::conflict(
                "Nie można usunąć ostatniego aktywnego administratora.",
            ));
        }
    }

    let mut transaction = state.db.begin().await?;

    sqlx::query(
        "DELETE FROM order_items WHERE order_id IN
         (SELECT id FROM orders WHERE user_id = ?)",
    )
    .bind(id)
    .execute(&mut *transaction)
    .await?;

    let deleted_orders = sqlx::query("DELETE FROM orders WHERE user_id = ?")
        .bind(id)
        .execute(&mut *transaction)
        .await?
        .rows_affected();

    sqlx::query("DELETE FROM sessions WHERE user_id = ?")
        .bind(id)
        .execute(&mut *transaction)
        .await?;

    let deleted_user = sqlx::query("DELETE FROM users WHERE id = ?")
        .bind(id)
        .execute(&mut *transaction)
        .await?;

    if deleted_user.rows_affected() == 0 {
        transaction.rollback().await?;
        return Err(AppError::not_found("Nie znaleziono użytkownika."));
    }

    transaction.commit().await?;

    db::audit(
        &state.db,
        Some(actor.id),
        "user_deleted",
        &format!("Usunięto konto id={id}, rola={role}, zamówienia={deleted_orders}"),
    )
    .await;

    tracing::warn!(
        actor_id = actor.id,
        target_id = id,
        role = %role,
        deleted_orders,
        "Usunięto konto wraz z historią zamówień"
    );

    Ok(Json(serde_json::json!({
        "ok": true,
        "login": login,
        "deleted_orders": deleted_orders
    })))
}

/// Eksport metadanych kont do CSV. Hasła nie są i nie mogą być częścią
/// eksportu — w bazie znajdują się wyłącznie skróty Argon2id.
pub async fn export_users_csv(
    State(state): State<Arc<AppState>>,
    Extension(actor): Extension<AuthUser>,
    Query(query): Query<UsersQuery>,
) -> Result<Response, AppError> {
    let mut builder = QueryBuilder::<Sqlite>::new(
        "SELECT login, role, display_code, active, must_change_password, created_at, last_login_at
         FROM users WHERE 1 = 1",
    );
    append_user_filters(&mut builder, &query);
    builder.push(" ORDER BY created_at DESC LIMIT 20000");

    let rows = builder
        .build_query_as::<ExportUserRow>()
        .fetch_all(&state.db)
        .await?;

    let mut csv = String::from("\u{feff}");
    csv.push_str("login;rola;kod;aktywne;haslo_startowe;utworzono;ostatnie_logowanie\r\n");

    for row in &rows {
        let created = format_local_datetime(row.created_at);
        let last_login = row
            .last_login_at
            .map(format_local_datetime)
            .unwrap_or_default();

        csv.push_str(&format!(
            "{};{};{};{};{};{};{}\r\n",
            csv_cell(&row.login),
            csv_cell(&row.role),
            csv_cell(&row.display_code),
            if row.active { "tak" } else { "nie" },
            if row.must_change_password { "tak" } else { "nie" },
            csv_cell(&created),
            csv_cell(&last_login),
        ));
    }

    let filename = format!(
        "konta-zsl-order-{}.csv",
        Local::now().format("%Y-%m-%d")
    );

    db::audit(
        &state.db,
        Some(actor.id),
        "users_exported",
        &format!("Eksport metadanych kont CSV, liczba={}", rows.len()),
    )
    .await;
    tracing::info!(
        actor_id = actor.id,
        rows = rows.len(),
        "Wyeksportowano metadane kont do CSV"
    );

    let mut response = (StatusCode::OK, csv).into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("text/csv; charset=utf-8"),
    );
    if let Ok(value) = axum::http::HeaderValue::from_str(&format!(
        "attachment; filename=\"{filename}\""
    )) {
        headers.insert(header::CONTENT_DISPOSITION, value);
    }
    headers.insert(
        header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );

    Ok(response)
}

fn format_local_datetime(timestamp: i64) -> String {
    use chrono::TimeZone;
    Local
        .timestamp_opt(timestamp, 0)
        .single()
        .map(|value| value.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_default()
}

fn csv_cell(value: &str) -> String {
    let mut text = value.to_string();
    // Ochrona przed formułami w arkuszach kalkulacyjnych.
    if text
        .chars()
        .next()
        .is_some_and(|first| matches!(first, '=' | '+' | '-' | '@'))
    {
        text.insert(0, '\'');
    }
    format!("\"{}\"", text.replace('"', "\"\""))
}

pub async fn reset_password(
    State(state): State<Arc<AppState>>,
    Extension(actor): Extension<AuthUser>,
    Path(id): Path<i64>,
    Json(_body): Json<ResetPasswordRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    if id == actor.id {
        return Err(AppError::bad(
            "Własne hasło zmień w sekcji „Moje konto”.",
        ));
    }

    let login: Option<String> = sqlx::query_scalar("SELECT login FROM users WHERE id = ?")
        .bind(id)
        .fetch_optional(&state.db)
        .await?;
    let login = login.ok_or_else(|| AppError::not_found("Nie znaleziono użytkownika."))?;

    let password = security::random_password(14);
    let hash = hash_password_async(password.clone()).await?;

    sqlx::query(
        "UPDATE users SET password_hash = ?, must_change_password = 1 WHERE id = ?",
    )
    .bind(hash)
    .bind(id)
    .execute(&state.db)
    .await?;

    sqlx::query("DELETE FROM sessions WHERE user_id = ?")
        .bind(id)
        .execute(&state.db)
        .await?;

    db::audit(
        &state.db,
        Some(actor.id),
        "password_reset",
        &format!("Reset hasła konta id={id}"),
    )
    .await;
    tracing::info!(
        actor_id = actor.id,
        target_id = id,
        "Zresetowano hasło konta"
    );

    Ok(Json(serde_json::json!({
        "login": login,
        "password": password,
        "note": "Nowe hasło jest widoczne tylko teraz."
    })))
}

pub async fn settings(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, AppError> {
    let rows = sqlx::query("SELECT key, value FROM settings")
        .fetch_all(&state.db)
        .await?;

    let mut map = serde_json::Map::new();
    for row in rows {
        let key: String = row.try_get("key").map_err(AppError::from)?;
        let value: String = row.try_get("value").map_err(AppError::from)?;
        map.insert(key, serde_json::Value::String(value));
    }

    Ok(Json(serde_json::Value::Object(map)))
}

pub async fn update_settings(
    State(state): State<Arc<AppState>>,
    Extension(actor): Extension<AuthUser>,
    Json(body): Json<SettingsRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let mut updates: Vec<(String, String)> = Vec::new();

    if let Some(name) = body.cafeteria_name {
        let name = name.trim().to_string();
        if name.is_empty() || name.chars().count() > 80 {
            return Err(AppError::bad(
                "Nazwa stołówki musi mieć od 1 do 80 znaków.",
            ));
        }
        updates.push(("cafeteria_name".to_string(), name));
    }
    if let Some(enabled) = body.ordering_enabled {
        updates.push((
            "ordering_enabled".to_string(),
            if enabled { "1" } else { "0" }.to_string(),
        ));
    }
    if let Some(info) = body.payment_info {
        let info = info.trim().to_string();
        if info.chars().count() > 300 {
            return Err(AppError::bad(
                "Informacja o płatności może mieć maksymalnie 300 znaków.",
            ));
        }
        updates.push(("payment_info".to_string(), info));
    }
    if let Some(value) = body.max_open_orders {
        if !(1..=10).contains(&value) {
            return Err(AppError::bad(
                "Limit oczekujących zamówień musi wynosić od 1 do 10.",
            ));
        }
        updates.push(("max_open_orders".to_string(), value.to_string()));
    }
    if let Some(value) = body.max_order_items {
        if !(1..=50).contains(&value) {
            return Err(AppError::bad(
                "Limit pozycji w zamówieniu musi wynosić od 1 do 50.",
            ));
        }
        updates.push(("max_order_items".to_string(), value.to_string()));
    }

    if updates.is_empty() {
        return Err(AppError::bad("Brak zmian do zapisania."));
    }

    for (key, value) in &updates {
        db::set_setting(&state.db, key, value).await?;
    }

    db::audit(
        &state.db,
        Some(actor.id),
        "settings_updated",
        &format!(
            "Zmieniono ustawienia: {}",
            updates
                .iter()
                .map(|(key, _)| key.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    )
    .await;

    settings(State(state)).await
}

pub async fn audit_log(
    State(state): State<Arc<AppState>>,
    Query(query): Query<AuditQuery>,
) -> Result<Json<Vec<AuditEntry>>, AppError> {
    let limit = query.limit.unwrap_or(200).clamp(1, 500);
    let rows = sqlx::query_as::<_, AuditEntry>(
        "SELECT a.id, a.action, a.details, a.created_at, u.login AS actor_login
         FROM audit_log a
         LEFT JOIN users u ON u.id = a.actor_id
         ORDER BY a.id DESC
         LIMIT ?",
    )
    .bind(limit)
    .fetch_all(&state.db)
    .await?;

    Ok(Json(rows))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_cell_quotes_and_escapes() {
        assert_eq!(csv_cell("abc"), "\"abc\"");
        assert_eq!(csv_cell("a;b"), "\"a;b\"");
        assert_eq!(csv_cell("a\"b"), "\"a\"\"b\"");
    }

    #[test]
    fn csv_cell_blocks_formula_injection() {
        assert_eq!(csv_cell("=1+1"), "\"'=1+1\"");
        assert_eq!(csv_cell("+51"), "\"'+51\"");
        assert_eq!(csv_cell("-1"), "\"'-1\"");
        assert_eq!(csv_cell("@x"), "\"'@x\"");
    }
}
