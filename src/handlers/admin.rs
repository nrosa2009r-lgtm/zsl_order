// SPDX-License-Identifier: MIT
use crate::auth::AuthUser;
use crate::client_ip::ClientIp;
use crate::db;
use crate::error::AppError;
use crate::handlers::auth::{hash_password_async, verify_password_async};
use crate::security;
use crate::state::AppState;
use crate::util;
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, QueryBuilder, Row, Sqlite, SqliteConnection, SqlitePool};
use std::collections::{BTreeMap, HashSet};
use std::net::IpAddr;
use std::sync::Arc;

/// Nagłówki dla odpowiedzi zawierających jednorazowe hasła. `no-store`
/// blokuje zapis w pamięci podręcznej przeglądarki i pośredników; hasła
/// nie mogą wyciec przez cache ani historię podręczną.
fn secret_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    headers
}

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
    #[serde(default)]
    pub cancel_window_minutes: Option<i64>,
    #[serde(default)]
    pub pickup_limit_minutes: Option<i64>,
    #[serde(default)]
    pub max_uncollected: Option<i64>,
    #[serde(default)]
    pub order_days_ahead: Option<i64>,
    #[serde(default)]
    pub max_item_quantity_per_break: Option<i64>,
    #[serde(default)]
    pub one_active_order_per_break: Option<bool>,
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
    pub issued: i64,
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
    if let Some(role) = query.role.as_deref()
        && matches!(role, "student" | "kitchen" | "admin")
    {
        builder.push(" AND role = ");
        builder.push_bind(role.to_string());
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
    let orders_today: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM orders WHERE created_at >= ?")
        .bind(util::start_of_today())
        .fetch_one(&state.db)
        .await?;
    // Przychód dnia to pieniądze faktycznie przyjęte dziś. Zamówienie
    // złożone wcześniej, ale opłacone przy odbiorze, liczy się w dniu
    // `paid_at`, a nie w dniu złożenia.
    let revenue_today: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(total_cents), 0) FROM orders
         WHERE paid_at >= ? AND paid_at < ? AND payment_status = 'paid'
           AND status <> 'cancelled'",
    )
    .bind(util::start_of_today())
    .bind(util::start_of_local_date(
        util::today() + chrono::Duration::days(1),
    ))
    .fetch_one(&state.db)
    .await?;
    let open_orders: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM orders WHERE status = 'ordered'")
            .fetch_one(&state.db)
            .await?;
    let awaiting_payment: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM orders
         WHERE payment_status = 'unpaid'
           AND status NOT IN ('cancelled', 'uncollected')",
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
    let today = util::today();
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
                issued: 0,
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
    let mut status_counts: BTreeMap<String, i64> =
        ["ordered", "issued", "cancelled", "uncollected"]
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
        *payment_counts.entry(payment_status.clone()).or_insert(0) += 1;

        if !matches!(status.as_str(), "cancelled" | "uncollected") {
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
            if status == "issued" {
                point.issued += 1;
            }
            if status == "cancelled" {
                point.cancelled += 1;
            }
            if payment_status == "paid" && status != "cancelled" {
                point.revenue_cents += total_cents;
            }
        }

        if date == today
            && let Some(hour) = util::local_hour(created_at)
            && let Some(point) = hourly.get_mut(hour as usize)
        {
            point.orders += 1;
        }
    }

    let top_rows = sqlx::query(
        "SELECT oi.name AS name, SUM(oi.quantity) AS quantity,
                SUM(oi.quantity * oi.price_cents) AS revenue_cents
         FROM order_items oi
         JOIN orders o ON o.id = oi.order_id
         WHERE o.created_at >= ?
           AND o.status NOT IN ('cancelled', 'uncollected')
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
    let total_issued: i64 = daily.iter().map(|point| point.issued).sum();
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
            "issued": total_issued,
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
) -> Result<(HeaderMap, Json<serde_json::Value>), AppError> {
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

    Ok((
        secret_headers(),
        Json(serde_json::json!({
            "accounts": accounts,
            "generated_at": now,
            "note": "Hasła są widoczne tylko teraz. Pobierz CSV i przekaż je uczniom w bezpieczny sposób."
        })),
    ))
}

/// Jedna grupa z importu CSV: nazwa klasy/grupy i liczba kont.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ImportGroup {
    group: String,
    count: u32,
}

#[derive(Clone, Serialize)]
pub struct ImportedAccount {
    pub login: String,
    pub password: String,
    pub role: String,
    pub display_code: String,
    pub group: String,
}

fn parse_import_csv(text: &str) -> Result<Vec<ImportGroup>, AppError> {
    let mut groups: Vec<ImportGroup> = Vec::new();
    let mut seen = HashSet::new();
    let mut total: u32 = 0;

    for (index, raw_line) in text.lines().enumerate() {
        let line = raw_line.trim_start_matches('\u{feff}').trim().to_string();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let fields: Vec<&str> = if line.contains(';') {
            line.split(';').collect()
        } else {
            line.split(',').collect()
        };
        if fields.len() != 2 {
            return Err(AppError::bad(format!(
                "Wiersz {}: oczekiwano formatu „klasa;liczba”, np. „1A;30”.",
                index + 1
            )));
        }

        let group = fields[0].trim();
        let count_raw = fields[1].trim();
        let Ok(count) = count_raw.parse::<u32>() else {
            // Nagłówek typu "klasa;liczba" pomijamy tylko na początku pliku.
            let is_header = groups.is_empty()
                && matches!(
                    group.to_ascii_lowercase().as_str(),
                    "klasa" | "grupa" | "class" | "group"
                );
            if is_header {
                continue;
            }
            return Err(AppError::bad(format!(
                "Wiersz {}: liczba kont musi być liczbą całkowitą.",
                index + 1
            )));
        };

        if group.is_empty()
            || group.len() > 16
            || !group
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
        {
            return Err(AppError::bad(format!(
                "Wiersz {}: nazwa klasy/grupy może mieć 1–16 znaków (litery ASCII, cyfry, „-”, „_”).",
                index + 1
            )));
        }
        if !(1..=100).contains(&count) {
            return Err(AppError::bad(format!(
                "Wiersz {}: liczba kont musi wynosić od 1 do 100.",
                index + 1
            )));
        }
        if !seen.insert(group.to_ascii_lowercase()) {
            return Err(AppError::bad(format!(
                "Wiersz {}: grupa „{group}” występuje więcej niż raz.",
                index + 1
            )));
        }

        total = total.saturating_add(count);
        if total > 200 {
            return Err(AppError::bad(
                "Jednorazowo można zaimportować maksymalnie 200 kont.",
            ));
        }

        groups.push(ImportGroup {
            group: group.to_string(),
            count,
        });
    }

    if groups.is_empty() {
        return Err(AppError::bad(
            "Plik CSV nie zawiera żadnych grup. Użyj formatu „1A;30”.",
        ));
    }

    Ok(groups)
}

/// Import kont uczniów z CSV w formacie „klasa;liczba” (np. `1A;30`).
/// Klasa trafia do loginu i kodu konta, więc można ją odfiltrować w panelu.
/// Hasła są zwracane wyłącznie w tej odpowiedzi i nie są zapisywane jawnie.
pub async fn import_users_csv(
    State(state): State<Arc<AppState>>,
    Extension(actor): Extension<AuthUser>,
    body: String,
) -> Result<(HeaderMap, Json<serde_json::Value>), AppError> {
    let groups = parse_import_csv(&body)?;
    let count: u32 = groups.iter().map(|group| group.count).sum();

    let existing: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(&state.db)
        .await?;
    if existing + i64::from(count) > 20_000 {
        return Err(AppError::conflict(
            "Osiągnięto limit liczby kont w systemie.",
        ));
    }

    let mut accounts: Vec<ImportedAccount> = Vec::with_capacity(count as usize);
    let mut used_logins = HashSet::new();
    let mut used_codes = HashSet::new();

    for group in &groups {
        let slug = group.group.to_ascii_lowercase();
        let login_prefix = format!("s-{slug}");
        let display_prefix = format!("UCZ-{}", group.group.to_ascii_uppercase());

        for _ in 0..group.count {
            let login = loop {
                let candidate = security::generate_login(&login_prefix);
                if used_logins.contains(&candidate) {
                    continue;
                }
                let exists: Option<i64> = sqlx::query_scalar("SELECT 1 FROM users WHERE login = ?")
                    .bind(&candidate)
                    .fetch_optional(&state.db)
                    .await?;
                if exists.is_none() {
                    used_logins.insert(candidate.clone());
                    break candidate;
                }
            };

            let display_code = loop {
                let candidate = security::generate_display_code(&display_prefix);
                if !used_codes.contains(&candidate) {
                    used_codes.insert(candidate.clone());
                    break candidate;
                }
            };

            accounts.push(ImportedAccount {
                login,
                password: security::random_password(14),
                role: "student".to_string(),
                display_code,
                group: group.group.clone(),
            });
        }
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
             (login, password_hash, role, display_code, class_group, active, must_change_password, created_at)
             VALUES (?, ?, 'student', ?, ?, 1, 1, ?)",
        )
        .bind(&account.login)
        .bind(hash)
        .bind(&account.display_code)
        .bind(&account.group)
        .bind(now)
        .execute(&mut *transaction)
        .await?;
    }
    transaction.commit().await?;

    let group_summary: Vec<String> = groups
        .iter()
        .map(|group| format!("{}={}", group.group, group.count))
        .collect();
    db::audit(
        &state.db,
        Some(actor.id),
        "users_imported",
        &format!(
            "Import CSV: kont={count}, grupy={}",
            group_summary.join(", ")
        ),
    )
    .await;
    tracing::info!(
        actor_id = actor.id,
        count,
        groups = groups.len(),
        "Zaimportowano konta uczniów z CSV"
    );

    Ok((
        secret_headers(),
        Json(serde_json::json!({
            "accounts": accounts,
            "generated_at": now,
            "groups": groups
                .iter()
                .map(|group| serde_json::json!({
                    "group": group.group,
                    "count": group.count,
                }))
                .collect::<Vec<_>>(),
            "note": "Hasła są widoczne tylko teraz. Pobierz CSV, wydrukuj karty i przekaż je uczniom w bezpieczny sposób."
        })),
    ))
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
    if let Some(role) = body.role.as_deref()
        && !matches!(role, "student" | "kitchen" | "admin")
    {
        return Err(AppError::bad("Nieprawidłowa rola użytkownika."));
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

    sqlx::query(
        "UPDATE users SET active = COALESCE(?, active), role = COALESCE(?, role) WHERE id = ?",
    )
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
    tracing::info!(actor_id = actor.id, target_id = id, "Zaktualizowano konto");

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

    // Historia zamówień zostaje: `orders.user_id` ma `ON DELETE SET NULL`,
    // więc raporty i nieodebrane nadal są kompletne, a w interfejsie
    // zamówienie pokazuje „konto usunięte”. Usuwamy też sesje (kaskada
    // zrobi to samo, ale jawne usunięcie jest czytelniejsze).
    let kept_orders: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM orders WHERE user_id = ?")
        .bind(id)
        .fetch_one(&mut *transaction)
        .await?;

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
        &format!(
            "Usunięto konto id={id}, rola={role}, zamówienia zachowane={kept_orders} (user_id NULL)"
        ),
    )
    .await;

    tracing::info!(
        actor_id = actor.id,
        target_id = id,
        role = %role,
        kept_orders,
        "Usunięto konto; historia zamówień zachowana"
    );

    Ok(Json(serde_json::json!({
        "ok": true,
        "login": login,
        "kept_orders": kept_orders
    })))
}

// ==================== Akcje masowe na kontach ====================

/// Maksymalna liczba kont w jednej operacji masowej.
pub const BULK_MAX_ACCOUNTS: usize = 500;
/// Ile kont pokazujemy szczegółowo w podglądzie (reszta tylko jako liczba).
const BULK_PREVIEW_ACCOUNTS: usize = 50;
/// Ile identyfikatorów trafia do wpisu audytu.
const BULK_AUDIT_IDS: usize = 50;

#[derive(Deserialize)]
pub struct BulkSelectionRequest {
    /// Jawna lista identyfikatorów (z podglądu). Pierwszeństwo przed
    /// filtrem; pusta lista jest błędem.
    #[serde(default)]
    pub ids: Option<Vec<i64>>,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub q: Option<String>,
    #[serde(default)]
    pub active: Option<bool>,
}

/// Krok 2 (potwierdzenie). Musi wskazywać **ten sam wybór** co krok 1:
/// tę samą listę ID albo ten sam filtr, a `expected_count` musi zgadzać
/// się z liczbą kont z podglądu. Zmiana liczby kont między krokami daje
/// `409` i nie zmienia żadnych danych.
#[derive(Deserialize)]
pub struct BulkConfirmRequest {
    #[serde(default)]
    pub ids: Option<Vec<i64>>,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub q: Option<String>,
    #[serde(default)]
    pub active: Option<bool>,
    pub expected_count: i64,
    /// Hasło administratora wykonującego akcję (re-autoryzacja).
    /// Brak pola lub pusta wartość = czytelny błąd 400, nie 422.
    #[serde(default)]
    pub password: String,
}

#[derive(Serialize)]
pub struct BulkAccountView {
    pub id: i64,
    pub login: String,
    pub display_code: String,
    pub role: String,
    pub active: bool,
    pub orders: i64,
    pub open_orders: i64,
}

#[derive(Serialize)]
pub struct ProtectedAccount {
    pub id: i64,
    pub login: String,
    pub reason: String,
}

#[derive(Serialize)]
pub struct BulkPreview {
    pub mode: String,
    /// Rozstrzygnięta lista ID — krok 2 może wysłać dokładnie tę listę.
    pub ids: Vec<i64>,
    pub count: i64,
    pub accounts: Vec<BulkAccountView>,
    pub accounts_truncated: bool,
    pub protected: Vec<ProtectedAccount>,
    pub total_orders: i64,
    pub open_orders: i64,
}

struct NormalizedSelection {
    ids: Option<Vec<i64>>,
    role: Option<String>,
    q: Option<String>,
    active: Option<bool>,
}

#[derive(Clone, Debug, Serialize)]
pub struct BulkApplyResult {
    pub action: String,
    pub count: u64,
    pub orders_orphaned: i64,
    /// Rozstrzygnięta lista kont objętych akcją.
    pub ids: Vec<i64>,
}

enum BulkAction {
    Delete,
    Deactivate,
}

fn normalize_selection(req: &BulkSelectionRequest) -> Result<NormalizedSelection, AppError> {
    if let Some(ids) = &req.ids {
        let mut cleaned: Vec<i64> = ids.iter().copied().filter(|id| *id > 0).collect();
        cleaned.sort_unstable();
        cleaned.dedup();
        if cleaned.is_empty() {
            return Err(AppError::bad("Lista ID jest pusta."));
        }
        if cleaned.len() > BULK_MAX_ACCOUNTS {
            return Err(AppError::bad(format!(
                "Maksymalnie {BULK_MAX_ACCOUNTS} kont w jednej operacji."
            )));
        }
        return Ok(NormalizedSelection {
            ids: Some(cleaned),
            role: None,
            q: None,
            active: None,
        });
    }

    if let Some(role) = req.role.as_deref()
        && !matches!(role, "student" | "kitchen" | "admin")
    {
        return Err(AppError::bad("Nieprawidłowa rola użytkownika."));
    }
    let search = req.q.as_deref().map(str::trim).unwrap_or("");
    let has_filter = req.role.is_some() || req.active.is_some() || !search.is_empty();
    if !has_filter {
        return Err(AppError::bad(
            "Podaj listę ID albo filtr (rolę, aktywność lub wyszukiwanie).",
        ));
    }
    Ok(NormalizedSelection {
        ids: None,
        role: req.role.clone(),
        q: Some(search.to_string()),
        active: req.active,
    })
}

/// Chronione konta pasujące do tego samego wyboru: konto wykonujące akcję
/// oraz wszyscy aktywni administratorzy (w tym ostatni aktywny). Akcje
/// masowe ich nigdy nie dotykają.
async fn bulk_protected<'e, E>(
    db: E,
    actor_id: i64,
    selection: &NormalizedSelection,
) -> Result<Vec<ProtectedAccount>, AppError>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
    let mut builder = QueryBuilder::<Sqlite>::new(
        "SELECT id, login, role, active FROM users
         WHERE (id = ",
    );
    builder.push_bind(actor_id);
    builder.push(" OR (role = 'admin' AND active = 1))");
    match &selection.ids {
        Some(ids) => {
            builder.push(" AND id IN (");
            let mut separated = builder.separated(", ");
            for id in ids {
                separated.push_bind(id);
            }
            separated.push_unseparated(")");
        }
        None => {
            let query = UsersQuery {
                role: selection.role.clone(),
                q: selection.q.clone(),
                active: selection.active,
            };
            append_user_filters(&mut builder, &query);
        }
    }
    builder.push(" ORDER BY id ASC LIMIT 10");
    let rows: Vec<(i64, String, String, bool)> = builder.build_query_as().fetch_all(db).await?;
    Ok(rows
        .into_iter()
        .map(|(id, login, role, _active)| ProtectedAccount {
            id,
            login,
            reason: if id == actor_id {
                "własne konto".to_string()
            } else {
                format!("aktywny administrator ({role})")
            },
        })
        .collect())
}

/// Rozstrzyga wybór do konkretnej listy kont przeznaczonych do akcji.
async fn bulk_resolve<'e, E>(
    db: E,
    actor_id: i64,
    selection: &NormalizedSelection,
) -> Result<(Vec<i64>, Vec<BulkAccountView>, i64, i64), AppError>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
    let mut builder = QueryBuilder::<Sqlite>::new(
        "SELECT u.id, u.login, u.role, u.display_code, u.active,
                (SELECT COUNT(*) FROM orders o WHERE o.user_id = u.id) AS orders,
                (SELECT COUNT(*) FROM orders o
                 WHERE o.user_id = u.id AND o.status = 'ordered') AS open_orders
         FROM users u
         WHERE 1 = 1",
    );
    match &selection.ids {
        Some(ids) => {
            builder.push(" AND u.id IN (");
            let mut separated = builder.separated(", ");
            for id in ids {
                separated.push_bind(id);
            }
            separated.push_unseparated(")");
        }
        None => {
            let query = UsersQuery {
                role: selection.role.clone(),
                q: selection.q.clone(),
                active: selection.active,
            };
            append_user_filters(&mut builder, &query);
        }
    }
    builder
        .push(" AND u.id <> ")
        .push_bind(actor_id)
        .push(" AND NOT (u.role = 'admin' AND u.active = 1)")
        .push(" ORDER BY u.id ASC LIMIT ")
        .push_bind(BULK_MAX_ACCOUNTS as i64 + 1);

    let rows: Vec<(i64, String, String, String, bool, i64, i64)> =
        builder.build_query_as().fetch_all(db).await?;
    if rows.len() > BULK_MAX_ACCOUNTS {
        return Err(AppError::bad(format!(
            "Wybór obejmuje więcej niż {BULK_MAX_ACCOUNTS} kont. Zawęź filtr."
        )));
    }

    let accounts = rows
        .iter()
        .map(|row| BulkAccountView {
            id: row.0,
            login: row.1.clone(),
            display_code: row.3.clone(),
            role: row.2.clone(),
            active: row.4,
            orders: row.5,
            open_orders: row.6,
        })
        .collect();
    let ids: Vec<i64> = rows.iter().map(|row| row.0).collect();
    let total_orders: i64 = rows.iter().map(|row| row.5).sum();
    let open_orders: i64 = rows.iter().map(|row| row.6).sum();
    Ok((ids, accounts, total_orders, open_orders))
}
/// Krok 1: podgląd wyboru (filtr albo lista ID). Zwraca rozstrzygniętą
/// listę ID, którą krok 2 może odesłać bez zmian, listę kont do pokazania
/// w interfejsie oraz konta chronione pasujące do wyboru (nigdy nie
/// objęte akcją).
pub async fn bulk_users_preview(
    State(state): State<Arc<AppState>>,
    Extension(actor): Extension<AuthUser>,
    Json(req): Json<BulkSelectionRequest>,
) -> Result<Json<BulkPreview>, AppError> {
    let selection = normalize_selection(&req)?;
    let (ids, accounts, total_orders, open_orders) =
        bulk_resolve(&state.db, actor.id, &selection).await?;
    let protected = bulk_protected(&state.db, actor.id, &selection).await?;
    let accounts_truncated = accounts.len() > BULK_PREVIEW_ACCOUNTS;
    let accounts = accounts.into_iter().take(BULK_PREVIEW_ACCOUNTS).collect();
    let mode = if selection.ids.is_some() {
        "ids"
    } else {
        "filter"
    };
    Ok(Json(BulkPreview {
        mode: mode.to_string(),
        count: ids.len() as i64,
        ids,
        accounts,
        accounts_truncated,
        protected,
        total_orders,
        open_orders,
    }))
}

pub async fn bulk_users_delete(
    State(state): State<Arc<AppState>>,
    Extension(actor): Extension<AuthUser>,
    Extension(ip): Extension<ClientIp>,
    Json(req): Json<BulkConfirmRequest>,
) -> Result<Json<BulkApplyResult>, AppError> {
    bulk_users_confirm(&state, &actor, ip.0, req, BulkAction::Delete).await
}

pub async fn bulk_users_deactivate(
    State(state): State<Arc<AppState>>,
    Extension(actor): Extension<AuthUser>,
    Extension(ip): Extension<ClientIp>,
    Json(req): Json<BulkConfirmRequest>,
) -> Result<Json<BulkApplyResult>, AppError> {
    bulk_users_confirm(&state, &actor, ip.0, req, BulkAction::Deactivate).await
}

async fn bulk_users_confirm(
    state: &Arc<AppState>,
    actor: &AuthUser,
    ip: IpAddr,
    req: BulkConfirmRequest,
    action: BulkAction,
) -> Result<Json<BulkApplyResult>, AppError> {
    let selection = normalize_selection(&BulkSelectionRequest {
        ids: req.ids,
        role: req.role,
        q: req.q,
        active: req.active,
    })?;
    if req.password.trim().is_empty() {
        return Err(AppError::bad("Podaj hasło administratora."));
    }

    // Rate limit: liczone tylko błędne hasła, ale blokada sprawdzana
    // zanim zaczniemy kosztowną weryfikację.
    if let Err(retry_after) = state.bulk_limiter.check(actor.id, ip) {
        tracing::warn!(
            event = "bulk_reauth_limited",
            actor_id = actor.id,
            ip = %ip,
            retry_after,
            "Akcje masowe zablokowane po nieudanych próbach re-autoryzacji"
        );
        return Err(AppError::TooManyRequests {
            retry_after,
            message: "Zbyt wiele nieudanych prób. Spróbuj ponownie za 15 minut.".to_string(),
        });
    }

    let hash: String = sqlx::query_scalar("SELECT password_hash FROM users WHERE id = ?")
        .bind(actor.id)
        .fetch_one(&state.db)
        .await?;
    let valid = verify_password_async(req.password.clone(), hash).await;
    if !valid {
        state.bulk_limiter.failure(actor.id, ip);
        tracing::warn!(
            event = "bulk_reauth_failed",
            actor_id = actor.id,
            ip = %ip,
            "Błędne hasło przy potwierdzeniu akcji masowej"
        );
        return Err(AppError::unauthorized(
            "Błędne hasło administratora. Akcja nie została wykonana.",
        ));
    }
    state.bulk_limiter.success(actor.id, ip);

    let (action_code, audit_action) = match action {
        BulkAction::Delete => ("delete", "bulk_users_deleted"),
        BulkAction::Deactivate => ("deactivate", "bulk_users_deactivated"),
    };

    let result = bulk_confirm(&state.db, actor.id, &selection, req.expected_count, &action).await?;

    let ids_in_audit: Vec<String> = result
        .ids
        .iter()
        .take(BULK_AUDIT_IDS)
        .map(|id| id.to_string())
        .collect();
    let ids_desc = if result.ids.len() > BULK_AUDIT_IDS {
        format!("{}… ({} kont)", ids_in_audit.join(","), result.ids.len())
    } else {
        ids_in_audit.join(",")
    };
    db::audit(
        &state.db,
        Some(actor.id),
        audit_action,
        &format!(
            "Liczba={}, zamówienia user_id=NULL={}, ID={}",
            result.count, result.orders_orphaned, ids_desc
        ),
    )
    .await;

    tracing::warn!(
        event = "bulk_users_applied",
        actor_id = actor.id,
        action = action_code,
        count = result.count,
        orders_orphaned = result.orders_orphaned,
        "Akcja masowa na kontach wykonana"
    );

    Ok(Json(result))
}

/// Wspólna logika kroku 2 w jednej transakcji `BEGIN IMMEDIATE`:
/// rozstrzygnięcie wyboru, kontrola zgodności liczby (i zbioru dla trybu
/// ID) z podglądem, egzekucja. Wszelki rozbieżność daje `409` i nie
/// zmienia danych.
async fn bulk_confirm(
    db: &SqlitePool,
    actor_id: i64,
    selection: &NormalizedSelection,
    expected_count: i64,
    action: &BulkAction,
) -> Result<BulkApplyResult, AppError> {
    if expected_count < 1 {
        return Err(AppError::bad(
            "Podaj liczbę kont z podglądu (expected_count, co najmniej 1).",
        ));
    }

    let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
    let (ids, _accounts, _total_orders, _open_orders) =
        bulk_resolve(&mut *tx, actor_id, selection).await?;

    if let Some(requested) = &selection.ids {
        let resolved: HashSet<i64> = ids.iter().copied().collect();
        let requested_set: HashSet<i64> = requested.iter().copied().collect();
        if resolved != requested_set {
            let protected = bulk_protected(&mut *tx, actor_id, selection).await?;
            let overlap: Vec<i64> = requested
                .iter()
                .filter(|id| protected.iter().any(|p| p.id == **id))
                .copied()
                .collect();
            tx.rollback().await?;
            return Err(if overlap.is_empty() {
                AppError::conflict(
                    "Lista kont zmieniła się między podglądem a potwierdzeniem. Uruchom podgląd ponownie.",
                )
            } else {
                let shown: Vec<String> = overlap.iter().take(5).map(|id| id.to_string()).collect();
                AppError::conflict(format!(
                    "Wybór obejmuje konta chronione (ID {}). Akcje masowe nie dotykają własnego konta ani aktywnych administratorów.",
                    shown.join(", ")
                ))
            });
        }
    }

    if ids.len() as i64 != expected_count {
        tx.rollback().await?;
        return Err(AppError::conflict(format!(
            "Liczba kont zmieniła się: podgląd wskazywał {expected_count}, teraz {}. Uruchom podgląd ponownie.",
            ids.len()
        )));
    }
    if ids.is_empty() {
        tx.rollback().await?;
        return Err(AppError::conflict(
            "Wybór nie zawiera żadnych kont objętych akcją.",
        ));
    }

    let (count, orders_orphaned) = bulk_execute(&mut tx, action, &ids).await?;
    tx.commit().await?;

    Ok(BulkApplyResult {
        action: match action {
            BulkAction::Delete => "delete".to_string(),
            BulkAction::Deactivate => "deactivate".to_string(),
        },
        count,
        orders_orphaned,
        ids,
    })
}

/// Egzekucja w ramach otwartej transakcji. Uczniowie zachowują historię:
/// `orders.user_id` ma `ON DELETE SET NULL`. Sesje usuwamy jawnie —
/// przy usuwaniu kaskada zrobiłaby to sama, przy dezaktywacji jest to
/// zgodne z pojedynczą dezaktywacją.
async fn bulk_execute(
    db: &mut SqliteConnection,
    action: &BulkAction,
    ids: &[i64],
) -> Result<(u64, i64), sqlx::Error> {
    let mut count_builder =
        QueryBuilder::<Sqlite>::new("SELECT COUNT(*) FROM orders WHERE user_id IN (");
    let mut separated = count_builder.separated(", ");
    for id in ids {
        separated.push_bind(*id);
    }
    separated.push_unseparated(")");
    let orders_orphaned: i64 = count_builder
        .build_query_scalar()
        .fetch_one(&mut *db)
        .await?;

    let mut statement = match action {
        BulkAction::Delete => QueryBuilder::<Sqlite>::new("DELETE FROM users WHERE id IN ("),
        BulkAction::Deactivate => {
            QueryBuilder::<Sqlite>::new("UPDATE users SET active = 0 WHERE id IN (")
        }
    };
    let mut separated = statement.separated(", ");
    for id in ids {
        separated.push_bind(*id);
    }
    separated.push_unseparated(")");
    let count = statement.build().execute(&mut *db).await?.rows_affected();

    let mut sessions_builder =
        QueryBuilder::<Sqlite>::new("DELETE FROM sessions WHERE user_id IN (");
    let mut separated = sessions_builder.separated(", ");
    for id in ids {
        separated.push_bind(*id);
    }
    separated.push_unseparated(")");
    sessions_builder.build().execute(&mut *db).await?;

    Ok((count, orders_orphaned))
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
            if row.must_change_password {
                "tak"
            } else {
                "nie"
            },
            csv_cell(&created),
            csv_cell(&last_login),
        ));
    }

    let filename = format!("konta-zsl-order-{}.csv", util::today().format("%Y-%m-%d"));

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
    if let Ok(value) =
        axum::http::HeaderValue::from_str(&format!("attachment; filename=\"{filename}\""))
    {
        headers.insert(header::CONTENT_DISPOSITION, value);
    }
    headers.insert(
        header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );

    Ok(response)
}

fn format_local_datetime(timestamp: i64) -> String {
    util::format_local_datetime(timestamp)
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

/// Unieważnia wszystkie sesje wybranego konta (ciasteczka i tokeny JWT).
/// Przydatne po podejrzeniu kradzieży urządzenia lub podejrzanym logowaniu.
pub async fn revoke_user_sessions(
    State(state): State<Arc<AppState>>,
    Extension(actor): Extension<AuthUser>,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, AppError> {
    if id == actor.id {
        return Err(AppError::bad(
            "Aby zakończyć własną sesję, użyj przycisku wylogowania.",
        ));
    }

    let exists: Option<i64> = sqlx::query_scalar("SELECT 1 FROM users WHERE id = ?")
        .bind(id)
        .fetch_optional(&state.db)
        .await?;
    if exists.is_none() {
        return Err(AppError::not_found("Nie znaleziono użytkownika."));
    }

    let revoked = sqlx::query("DELETE FROM sessions WHERE user_id = ?")
        .bind(id)
        .execute(&state.db)
        .await?
        .rows_affected();

    db::audit(
        &state.db,
        Some(actor.id),
        "sessions_revoked",
        &format!("Unieważniono sesje konta id={id}, liczba={revoked}"),
    )
    .await;

    Ok(Json(serde_json::json!({
        "ok": true,
        "revoked": revoked,
    })))
}

/// Zdejmuje blokadę nowych zamówień wynikającą z limitu nieodebranych,
/// nie usuwając historii ani znaczników `uncollected_at`. Licznik blokady
/// liczy tylko zamówienia nieodebrane po dacie resetu.
pub async fn clear_uncollected_block(
    State(state): State<Arc<AppState>>,
    Extension(actor): Extension<AuthUser>,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, AppError> {
    let role: Option<String> = sqlx::query_scalar("SELECT role FROM users WHERE id = ?")
        .bind(id)
        .fetch_optional(&state.db)
        .await?;
    let role = role.ok_or_else(|| AppError::not_found("Nie znaleziono użytkownika."))?;

    if role != "student" {
        return Err(AppError::bad(
            "Blokada nieodebranych dotyczy wyłącznie kont uczniów.",
        ));
    }

    let cleared_at = util::now();
    sqlx::query("UPDATE users SET uncollected_block_reset_at = ? WHERE id = ?")
        .bind(cleared_at)
        .bind(id)
        .execute(&state.db)
        .await?;

    db::audit(
        &state.db,
        Some(actor.id),
        "uncollected_block_cleared",
        &format!("Zdjęto blokadę nieodebranych dla konta id={id}"),
    )
    .await;
    tracing::info!(
        actor_id = actor.id,
        target_id = id,
        "Zdjęto blokadę nieodebranych"
    );

    Ok(Json(serde_json::json!({
        "ok": true,
        "cleared_at": cleared_at,
    })))
}

pub async fn reset_password(
    State(state): State<Arc<AppState>>,
    Extension(actor): Extension<AuthUser>,
    Path(id): Path<i64>,
    Json(_body): Json<ResetPasswordRequest>,
) -> Result<(HeaderMap, Json<serde_json::Value>), AppError> {
    if id == actor.id {
        return Err(AppError::bad("Własne hasło zmień w sekcji „Moje konto”."));
    }

    let login: Option<String> = sqlx::query_scalar("SELECT login FROM users WHERE id = ?")
        .bind(id)
        .fetch_optional(&state.db)
        .await?;
    let login = login.ok_or_else(|| AppError::not_found("Nie znaleziono użytkownika."))?;

    let password = security::random_password(14);
    let hash = hash_password_async(password.clone()).await?;

    sqlx::query("UPDATE users SET password_hash = ?, must_change_password = 1 WHERE id = ?")
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

    Ok((
        secret_headers(),
        Json(serde_json::json!({
            "login": login,
            "password": password,
            "note": "Nowe hasło jest widoczne tylko teraz."
        })),
    ))
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
            return Err(AppError::bad("Nazwa stołówki musi mieć od 1 do 80 znaków."));
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
    if let Some(value) = body.cancel_window_minutes {
        if !(0..=1440).contains(&value) {
            return Err(AppError::bad(
                "Termin anulowania musi wynosić od 0 do 1440 minut (0 = bez limitu).",
            ));
        }
        updates.push(("cancel_window_minutes".to_string(), value.to_string()));
    }
    if let Some(value) = body.pickup_limit_minutes {
        if !(0..=720).contains(&value) {
            return Err(AppError::bad(
                "Limit czasu odbioru musi wynosić od 0 do 720 minut (0 = wyłączony).",
            ));
        }
        updates.push(("pickup_limit_minutes".to_string(), value.to_string()));
    }
    if let Some(value) = body.max_uncollected {
        if !(0..=20).contains(&value) {
            return Err(AppError::bad(
                "Limit nieodebranych zamówień musi wynosić od 0 do 20 (0 = brak blokady).",
            ));
        }
        updates.push(("max_uncollected".to_string(), value.to_string()));
    }
    if let Some(value) = body.order_days_ahead {
        if !(0..=7).contains(&value) {
            return Err(AppError::bad(
                "Zamawianie z wyprzedzeniem musi wynosić od 0 do 7 dni (0 = tylko dziś).",
            ));
        }
        updates.push(("order_days_ahead".to_string(), value.to_string()));
    }
    if let Some(value) = body.max_item_quantity_per_break {
        if !(0..=10).contains(&value) {
            return Err(AppError::bad(
                "Limit sztuk jednej pozycji na przerwę musi wynosić od 0 do 10 (0 = bez limitu).",
            ));
        }
        updates.push(("max_item_quantity_per_break".to_string(), value.to_string()));
    }
    if let Some(value) = body.one_active_order_per_break {
        updates.push((
            "one_active_order_per_break".to_string(),
            if value { "1" } else { "0" }.to_string(),
        ));
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

    #[test]
    fn generated_credentials_use_safe_alphabet() {
        const SPECIAL: &[char] = &['=', '+', '-', '@'];
        const AMBIGUOUS: &[char] = &['0', 'O', '1', 'l', 'I'];

        for _ in 0..200 {
            let password = crate::security::random_password(14);
            let login = crate::security::generate_login("s-1a");

            let first_password = password.chars().next().expect("hasło nie jest puste");
            let first_login = login.chars().next().expect("login nie jest pusty");
            assert!(first_password.is_ascii_alphanumeric());
            assert!(first_login.is_ascii_alphanumeric());
            assert!(!SPECIAL.contains(&first_password));
            assert!(!SPECIAL.contains(&first_login));

            for character in password.chars() {
                assert!(!SPECIAL.contains(&character), "znak specjalny w haśle");
                assert!(!AMBIGUOUS.contains(&character), "znak mylący w haśle");
            }
            assert!(
                password
                    .chars()
                    .any(|character| character.is_ascii_alphabetic())
            );
            assert!(password.chars().any(|character| character.is_ascii_digit()));
        }
    }

    #[test]
    fn import_parses_groups_and_header() {
        let groups = parse_import_csv("\u{feff}klasa;liczba\n1A;30\n2B;25\n").unwrap();
        assert_eq!(
            groups,
            vec![
                ImportGroup {
                    group: "1A".to_string(),
                    count: 30
                },
                ImportGroup {
                    group: "2B".to_string(),
                    count: 25
                }
            ]
        );
    }

    #[test]
    fn import_accepts_comments_and_commas() {
        let groups = parse_import_csv("# komentarz\n3C,10\n").unwrap();
        assert_eq!(groups[0].group, "3C");
        assert_eq!(groups[0].count, 10);
    }

    #[test]
    fn import_rejects_bad_input() {
        assert!(parse_import_csv("").is_err());
        assert!(parse_import_csv("1A\n").is_err());
        assert!(parse_import_csv("1A;0\n").is_err());
        assert!(parse_import_csv("1A;101\n").is_err());
        assert!(parse_import_csv("1A;10\n1a;5\n").is_err());
        assert!(parse_import_csv("zła grupa;10\n").is_err());
        assert!(parse_import_csv("1A;10;20\n").is_err());
    }

    // ---------------- masowe usuwanie/dezaktywacja (DB) ----------------

    use std::path::PathBuf;
    use std::str::FromStr;
    use std::sync::atomic::{AtomicU64, Ordering};

    static BULK_POOL_COUNTER: AtomicU64 = AtomicU64::new(0);

    async fn bulk_pool(name: &str) -> (SqlitePool, PathBuf) {
        use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
        let unique = BULK_POOL_COUNTER.fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!(
            "zsl-bulk-{name}-{}-{unique}.db",
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

    async fn bulk_close(pool: SqlitePool, path: PathBuf) {
        pool.close().await;
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
        }
    }

    async fn insert_user(pool: &SqlitePool, login: &str, role: &str, active: bool) -> i64 {
        sqlx::query_scalar(
            "INSERT INTO users (login, password_hash, role, display_code, active,
                                must_change_password, created_at)
             VALUES (?, 'test-hash', ?, ?, ?, 0, 1)
             RETURNING id",
        )
        .bind(login)
        .bind(role)
        .bind(login.to_uppercase())
        .bind(active)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    async fn insert_order(pool: &SqlitePool, user_id: Option<i64>, code: &str) -> i64 {
        sqlx::query_scalar(
            "INSERT INTO orders (user_id, code, qr_token, status, payment_method,
                                 payment_status, total_cents, pickup_time, created_at, updated_at)
             VALUES (?, ?, ?, 'ordered', 'cash', 'unpaid', 500, '', 1, 1)
             RETURNING id",
        )
        .bind(user_id)
        .bind(code)
        .bind(format!("{code}-tok"))
        .fetch_one(pool)
        .await
        .unwrap()
    }

    async fn insert_session(pool: &SqlitePool, user_id: i64, token: &str) {
        sqlx::query(
            "INSERT INTO sessions (token_hash, user_id, csrf_token, created_at, expires_at, last_seen_at)
             VALUES (?, ?, 'csrf', 1, 9999999999, 1)",
        )
        .bind(token)
        .bind(user_id)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn user_exists(pool: &SqlitePool, id: i64) -> bool {
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM users WHERE id = ?")
            .bind(id)
            .fetch_one(pool)
            .await
            .unwrap()
            == 1
    }

    async fn user_active(pool: &SqlitePool, id: i64) -> bool {
        sqlx::query_scalar::<_, i64>("SELECT active FROM users WHERE id = ?")
            .bind(id)
            .fetch_one(pool)
            .await
            .unwrap()
            == 1
    }

    async fn session_count(pool: &SqlitePool, user_id: i64) -> i64 {
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sessions WHERE user_id = ?")
            .bind(user_id)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    async fn order_owner(pool: &SqlitePool, order_id: i64) -> Option<i64> {
        sqlx::query_scalar::<_, Option<i64>>("SELECT user_id FROM orders WHERE id = ?")
            .bind(order_id)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    fn ids_selection(ids: Vec<i64>) -> NormalizedSelection {
        NormalizedSelection {
            ids: Some(ids),
            role: None,
            q: None,
            active: None,
        }
    }

    fn filter_selection(role: &str) -> NormalizedSelection {
        NormalizedSelection {
            ids: None,
            role: Some(role.to_string()),
            q: Some(String::new()),
            active: None,
        }
    }

    #[tokio::test]
    async fn bulk_delete_keeps_order_history_and_removes_sessions() {
        let (pool, path) = bulk_pool("delete-happy").await;
        let admin = insert_user(&pool, "admin-1", "admin", true).await;
        let s1 = insert_user(&pool, "uczen-1", "student", true).await;
        let s2 = insert_user(&pool, "uczen-2", "student", true).await;
        let o1 = insert_order(&pool, Some(s1), "BUL001").await;
        let o2 = insert_order(&pool, Some(s2), "BUL002").await;
        insert_session(&pool, s1, "tok-bulk-1").await;
        insert_session(&pool, s2, "tok-bulk-2").await;

        let selection = ids_selection(vec![s1, s2]);
        let result = bulk_confirm(&pool, admin, &selection, 2, &BulkAction::Delete)
            .await
            .expect("usunięcie udane");

        assert_eq!(result.action, "delete");
        assert_eq!(result.count, 2);
        assert_eq!(result.orders_orphaned, 2);
        assert!(!user_exists(&pool, s1).await);
        assert!(!user_exists(&pool, s2).await);
        // Historia zamówień zostaje, ale bez powiązania z kontem.
        assert_eq!(order_owner(&pool, o1).await, None);
        assert_eq!(order_owner(&pool, o2).await, None);
        // Sesje usunięte (kaskada + jawne usunięcie).
        assert_eq!(session_count(&pool, s1).await, 0);
        assert_eq!(session_count(&pool, s2).await, 0);
        // Konto administratora nietknięte.
        assert!(user_exists(&pool, admin).await);

        bulk_close(pool, path).await;
    }

    #[tokio::test]
    async fn bulk_delete_wrong_expected_count_conflicts_without_changes() {
        let (pool, path) = bulk_pool("count-mismatch").await;
        let admin = insert_user(&pool, "admin-2", "admin", true).await;
        let s1 = insert_user(&pool, "uczen-3", "student", true).await;
        let s2 = insert_user(&pool, "uczen-4", "student", true).await;
        let order = insert_order(&pool, Some(s1), "BUL011").await;

        // Liczba kont wzrosła względem podglądu -> 409 i zero zmian.
        let selection = ids_selection(vec![s1, s2]);
        let error = bulk_confirm(&pool, admin, &selection, 1, &BulkAction::Delete)
            .await
            .expect_err("niezgodna liczba kont musi dać 409");
        assert!(matches!(error, AppError::Conflict(_)));
        assert!(user_exists(&pool, s1).await);
        assert!(user_exists(&pool, s2).await);
        assert_eq!(order_owner(&pool, order).await, Some(s1));

        // Liczba kont spadła względem podglądu -> też 409.
        let error = bulk_confirm(&pool, admin, &selection, 3, &BulkAction::Delete)
            .await
            .expect_err("mniejsza liczba także daje 409");
        assert!(matches!(error, AppError::Conflict(_)));
        assert!(user_exists(&pool, s1).await);

        bulk_close(pool, path).await;
    }

    #[tokio::test]
    async fn bulk_ids_covering_protected_accounts_conflict() {
        let (pool, path) = bulk_pool("protected").await;
        let admin = insert_user(&pool, "admin-3", "admin", true).await;
        let other_admin = insert_user(&pool, "admin-4", "admin", true).await;
        let s1 = insert_user(&pool, "uczen-5", "student", true).await;
        let old_admin = insert_user(&pool, "admin-5", "admin", false).await;

        // Własne konto w liście ID -> 409.
        let selection = ids_selection(vec![s1, admin]);
        let error = bulk_confirm(&pool, admin, &selection, 2, &BulkAction::Delete)
            .await
            .expect_err("własne konto jest chronione");
        assert!(matches!(error, AppError::Conflict(message) if message.contains("chronione")));
        assert!(user_exists(&pool, s1).await);

        // Aktywny administrator w liście ID -> 409.
        let selection = ids_selection(vec![s1, other_admin]);
        let error = bulk_confirm(&pool, admin, &selection, 2, &BulkAction::Delete)
            .await
            .expect_err("aktywny admin jest chroniony");
        assert!(matches!(error, AppError::Conflict(_)));
        assert!(user_exists(&pool, other_admin).await);

        // Nieistniejące ID w liście -> rozbieżność zbioru -> 409.
        let selection = ids_selection(vec![s1, 99_999]);
        let error = bulk_confirm(&pool, admin, &selection, 2, &BulkAction::Delete)
            .await
            .expect_err("zbior ID musi się zgadzać");
        assert!(matches!(error, AppError::Conflict(_)));
        assert!(user_exists(&pool, s1).await);

        // Nieaktywny administrator NIE jest chroniony jako aktywny admin
        // (ostatni aktywny administrator nadal zostaje chroniony — tu jest
        // jeszcze admin-3), więc usunięcie przechodzi.
        let selection = ids_selection(vec![old_admin]);
        let result = bulk_confirm(&pool, admin, &selection, 1, &BulkAction::Delete)
            .await
            .expect("nieaktywny admin może być usunięty");
        assert_eq!(result.count, 1);
        assert!(!user_exists(&pool, old_admin).await);

        bulk_close(pool, path).await;
    }

    #[tokio::test]
    async fn bulk_delete_rolls_back_completely_when_statement_fails() {
        let (pool, path) = bulk_pool("atomic").await;
        let admin = insert_user(&pool, "admin-6", "admin", true).await;
        let s1 = insert_user(&pool, "uczen-6", "student", true).await;
        let s2 = insert_user(&pool, "uczen-7", "student", true).await;
        let o1 = insert_order(&pool, Some(s1), "BUL021").await;
        let o2 = insert_order(&pool, Some(s2), "BUL022").await;
        insert_session(&pool, s1, "tok-bulk-3").await;

        // Trigger wymuszający błąd w trakcie usuwania kont: cały skrypt
        // musi wycofać się w całości — bez „połowy” usuniętych kont.
        sqlx::query(
            "CREATE TRIGGER bulk_test_abort BEFORE DELETE ON users
             FOR EACH ROW BEGIN SELECT RAISE(ABORT, 'wymuszony błąd testu'); END",
        )
        .execute(&pool)
        .await
        .unwrap();

        let selection = ids_selection(vec![s1, s2]);
        let error = bulk_confirm(&pool, admin, &selection, 2, &BulkAction::Delete)
            .await
            .expect_err("trigger musi przerwać akcję");
        assert!(matches!(
            error,
            AppError::Internal | AppError::Unavailable(_)
        ));

        // Atomowość: żadne konto, zamówienie ani sesja nie zostały ruszone.
        assert!(user_exists(&pool, s1).await);
        assert!(user_exists(&pool, s2).await);
        assert_eq!(order_owner(&pool, o1).await, Some(s1));
        assert_eq!(order_owner(&pool, o2).await, Some(s2));
        assert_eq!(session_count(&pool, s1).await, 1);

        bulk_close(pool, path).await;
    }

    #[tokio::test]
    async fn bulk_deactivate_disables_accounts_and_sessions() {
        let (pool, path) = bulk_pool("deactivate").await;
        let admin = insert_user(&pool, "admin-7", "admin", true).await;
        let s1 = insert_user(&pool, "uczen-8", "student", true).await;
        let s2 = insert_user(&pool, "uczen-9", "student", true).await;
        let order = insert_order(&pool, Some(s1), "BUL031").await;
        insert_session(&pool, s1, "tok-bulk-4").await;

        let selection = ids_selection(vec![s1, s2]);
        let result = bulk_confirm(&pool, admin, &selection, 2, &BulkAction::Deactivate)
            .await
            .expect("dezaktywacja udana");

        assert_eq!(result.action, "deactivate");
        assert_eq!(result.count, 2);
        // Konta istnieją, ale są nieaktywne.
        assert!(!user_active(&pool, s1).await);
        assert!(!user_active(&pool, s2).await);
        // Sesje dezaktywowanych kont usunięte.
        assert_eq!(session_count(&pool, s1).await, 0);
        // Historia zamówień bez zmian (konta nie usunięte).
        assert_eq!(order_owner(&pool, order).await, Some(s1));
        assert!(user_active(&pool, admin).await);

        bulk_close(pool, path).await;
    }

    #[tokio::test]
    async fn bulk_filter_selection_excludes_actor_and_protected_admins() {
        let (pool, path) = bulk_pool("filter").await;
        let admin = insert_user(&pool, "admin-8", "admin", true).await;
        let s1 = insert_user(&pool, "uczen-10", "student", true).await;
        let s2 = insert_user(&pool, "uczen-11", "student", true).await;
        insert_user(&pool, "kuchnia-1", "kitchen", true).await;

        // Filtr „rola = student" nie obejmuje kuchni ani administratorów.
        let selection = filter_selection("student");
        let (ids, _accounts, _total, _open) = bulk_resolve(&pool, admin, &selection).await.unwrap();
        assert_eq!(
            ids,
            vec![s1, s2]
                .into_iter()
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>()
        );

        // Podgląd wskazuje 2 konta; dezaktywacja tym samym filtrem przechodzi.
        let result = bulk_confirm(&pool, admin, &selection, 2, &BulkAction::Deactivate)
            .await
            .expect("filtr zgodny z podglądem");
        assert_eq!(result.count, 2);
        assert!(!user_active(&pool, s1).await);
        assert!(!user_active(&pool, s2).await);

        // Wybór bez żadnego kryterium jest odrzucany.
        let empty = NormalizedSelection {
            ids: None,
            role: None,
            q: None,
            active: None,
        };
        let error = bulk_confirm(&pool, admin, &empty, 1, &BulkAction::Deactivate)
            .await
            .expect_err("pusty wybór jest błędem");
        assert!(matches!(
            error,
            AppError::BadRequest(_) | AppError::Conflict(_)
        ));

        // Ochrona ostatniego aktywnego admina: filtr role=admin nie zwraca
        // aktywnych administratorów.
        let admin_filter = filter_selection("admin");
        let (ids, ..) = bulk_resolve(&pool, admin, &admin_filter).await.unwrap();
        assert!(ids.is_empty());

        bulk_close(pool, path).await;
    }

    #[test]
    fn bulk_selection_requires_a_criterion_or_ids() {
        let request = BulkSelectionRequest {
            ids: None,
            role: None,
            q: None,
            active: None,
        };
        assert!(normalize_selection(&request).is_err());

        let request = BulkSelectionRequest {
            ids: Some(vec![]),
            role: None,
            q: None,
            active: None,
        };
        assert!(normalize_selection(&request).is_err());

        let request = BulkSelectionRequest {
            ids: Some(vec![0, -5]),
            role: None,
            q: None,
            active: None,
        };
        assert!(normalize_selection(&request).is_err());

        let request = BulkSelectionRequest {
            ids: Some(vec![3, 1, 3]),
            role: None,
            q: None,
            active: None,
        };
        let selection = normalize_selection(&request).unwrap();
        assert_eq!(selection.ids, Some(vec![1, 3]));

        let request = BulkSelectionRequest {
            ids: None,
            role: Some("hacker".to_string()),
            q: None,
            active: None,
        };
        assert!(normalize_selection(&request).is_err());
    }
}
