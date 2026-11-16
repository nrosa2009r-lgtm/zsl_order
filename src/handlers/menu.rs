// SPDX-License-Identifier: MIT
use crate::auth::AuthUser;
use crate::db;
use crate::error::AppError;
use crate::state::AppState;
use crate::util;
use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use sqlx::{Connection, FromRow};
use std::sync::Arc;

#[derive(Clone, Debug, FromRow, Serialize)]
pub struct MenuItemView {
    pub id: i64,
    pub name: String,
    pub description: String,
    pub price_cents: i64,
    pub category: String,
    pub allergens: String,
    pub available: bool,
    pub sort_order: i64,
    /// NULL = bez limitu; 0 = wyprzedane.
    pub stock_remaining: Option<i64>,
    /// Domyślny stan na dzień (NULL = brak automatycznego resetu).
    pub stock_default: Option<i64>,
    /// Dzień, którego dotyczy `stock_remaining` (`RRRR-MM-DD`).
    pub stock_date: Option<String>,
}

#[derive(Serialize)]
pub struct StudentMenuResponse {
    pub items: Vec<MenuItemView>,
    pub ordering_enabled: bool,
    pub payment_info: String,
    pub max_order_items: i64,
    pub cancel_window_minutes: i64,
    pub pickup_limit_minutes: i64,
    pub max_uncollected: i64,
}

#[derive(Deserialize)]
pub struct MenuItemInput {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub price_cents: i64,
    #[serde(default = "default_category")]
    pub category: String,
    #[serde(default)]
    pub allergens: String,
    #[serde(default = "default_true")]
    pub available: bool,
    #[serde(default)]
    pub sort_order: i64,
    /// Stan na dziś; brak/null = bez limitu.
    #[serde(default)]
    pub stock_remaining: Option<i64>,
    /// Domyślny stan dzienny; brak/null = bez automatycznego resetu.
    #[serde(default)]
    pub stock_default: Option<i64>,
}

fn default_category() -> String {
    "Dania".to_string()
}

fn default_true() -> bool {
    true
}

pub async fn student_menu(
    State(state): State<Arc<AppState>>,
) -> Result<Json<StudentMenuResponse>, AppError> {
    // Leniwy reset dobowy: pierwszy odczyt menu po północy przywraca
    // domyślne stany, zanim uczeń zobaczy dostępność.
    crate::stock::reset_daily_stock(&state.db, util::today()).await?;

    let items = sqlx::query_as::<_, MenuItemView>(
        "SELECT id, name, description, price_cents, category, allergens, available,
                sort_order, stock_remaining, stock_default, stock_date
         FROM menu_items
         WHERE available = 1
         ORDER BY sort_order ASC, name ASC",
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(StudentMenuResponse {
        items,
        ordering_enabled: db::get_setting_or(&state.db, "ordering_enabled", "1").await == "1",
        payment_info: db::get_setting_or(
            &state.db,
            "payment_info",
            "Płatność przy odbiorze: gotówka lub karta.",
        )
        .await,
        max_order_items: setting_i64(&state, "max_order_items", 20).await,
        cancel_window_minutes: setting_i64(&state, "cancel_window_minutes", 30).await,
        pickup_limit_minutes: setting_i64(&state, "pickup_limit_minutes", 120).await,
        max_uncollected: setting_i64(&state, "max_uncollected", 0).await,
    }))
}

pub async fn staff_menu(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<MenuItemView>>, AppError> {
    crate::stock::reset_daily_stock(&state.db, util::today()).await?;

    let items = sqlx::query_as::<_, MenuItemView>(
        "SELECT id, name, description, price_cents, category, allergens, available,
                sort_order, stock_remaining, stock_default, stock_date
         FROM menu_items
         ORDER BY available DESC, sort_order ASC, name ASC",
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(items))
}

pub async fn create_menu_item(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Json(body): Json<MenuItemInput>,
) -> Result<(StatusCode, Json<MenuItemView>), AppError> {
    let input = validate_input(body)?;
    let now = util::now();
    let stock_date = stock_date_for(&input);

    let result = sqlx::query(
        "INSERT INTO menu_items
         (name, description, price_cents, category, allergens, available, sort_order,
          created_at, updated_at, stock_remaining, stock_default, stock_date)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&input.name)
    .bind(&input.description)
    .bind(input.price_cents)
    .bind(&input.category)
    .bind(&input.allergens)
    .bind(input.available)
    .bind(input.sort_order)
    .bind(now)
    .bind(now)
    .bind(input.stock_remaining)
    .bind(input.stock_default)
    .bind(&stock_date)
    .execute(&state.db)
    .await?;

    let id = result.last_insert_rowid();
    db::audit(
        &state.db,
        Some(user.id),
        "menu_item_created",
        &format!("Pozycja menu id={id}"),
    )
    .await;

    let item = get_item(&state, id).await?;
    Ok((StatusCode::CREATED, Json(item)))
}

pub async fn update_menu_item(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i64>,
    Json(body): Json<MenuItemInput>,
) -> Result<Json<MenuItemView>, AppError> {
    let input = validate_input(body)?;
    let now = util::now();
    let stock_date = stock_date_for(&input);

    let result = sqlx::query(
        "UPDATE menu_items
         SET name = ?, description = ?, price_cents = ?, category = ?, allergens = ?,
             available = ?, sort_order = ?, updated_at = ?,
             stock_remaining = ?, stock_default = ?, stock_date = ?
         WHERE id = ?",
    )
    .bind(&input.name)
    .bind(&input.description)
    .bind(input.price_cents)
    .bind(&input.category)
    .bind(&input.allergens)
    .bind(input.available)
    .bind(input.sort_order)
    .bind(now)
    .bind(input.stock_remaining)
    .bind(input.stock_default)
    .bind(&stock_date)
    .bind(id)
    .execute(&state.db)
    .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::not_found("Nie znaleziono pozycji menu."));
    }

    db::audit(
        &state.db,
        Some(user.id),
        "menu_item_updated",
        &format!("Pozycja menu id={id}"),
    )
    .await;

    Ok(Json(get_item(&state, id).await?))
}

#[derive(Deserialize)]
pub struct StockUpdateRequest {
    /// Zmiana względna, np. 1 lub -1. Wymaga pozycji z limitem.
    #[serde(default)]
    pub delta: Option<i64>,
    /// Ustawienie bezwzględne stanu na dziś.
    #[serde(default)]
    pub set: Option<i64>,
    /// `true` = bez limitu (NULL).
    #[serde(default)]
    pub unlimited: bool,
}

/// Szybka korekta stanu na dziś z panelu menu. `delta` jest liczona na
/// serwerze na podstawie odczytanego stanu, a warunkowy `UPDATE` pilnuje,
/// żeby równoległa zmiana nie została po cichu nadpisana — druga osoba
/// dostaje `409` i może odświeżyć listę.
pub async fn update_menu_stock(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i64>,
    Json(body): Json<StockUpdateRequest>,
) -> Result<Json<MenuItemView>, AppError> {
    let actions = usize::from(body.delta.is_some())
        + usize::from(body.set.is_some())
        + usize::from(body.unlimited);
    if actions != 1 {
        return Err(AppError::bad(
            "Podaj dokładnie jedną operację: delta, set albo unlimited.",
        ));
    }
    if let Some(delta) = body.delta
        && !(-1000..=1000).contains(&delta)
    {
        return Err(AppError::bad("Zmiana stanu musi wynosić od -1000 do 1000."));
    }
    if let Some(set) = body.set
        && !(0..=100_000).contains(&set)
    {
        return Err(AppError::bad("Stan musi wynosić od 0 do 100000 sztuk."));
    }

    let today = util::today();
    let today_key = crate::stock::date_key(today);
    let mut connection = state.db.acquire().await?;
    let mut transaction = connection.begin_with("BEGIN IMMEDIATE").await?;
    crate::stock::reset_daily_stock(&mut *transaction, today).await?;

    let current: Option<(String, Option<i64>, Option<i64>)> =
        sqlx::query_as("SELECT name, stock_remaining, stock_default FROM menu_items WHERE id = ?")
            .bind(id)
            .fetch_optional(&mut *transaction)
            .await?;
    let Some((name, old, stock_default)) = current else {
        transaction.rollback().await?;
        return Err(AppError::not_found("Nie znaleziono pozycji menu."));
    };

    let new_value = if body.unlimited {
        None
    } else if let Some(value) = body.set {
        Some(value)
    } else {
        let delta = body.delta.unwrap_or(0);
        let Some(old_value) = old else {
            transaction.rollback().await?;
            return Err(AppError::conflict(
                "Ta pozycja nie ma limitu. Najpierw ustaw stan, aby używać +/−.",
            ));
        };
        let next = old_value + delta;
        if next < 0 {
            transaction.rollback().await?;
            return Err(AppError::conflict("Stan nie może być ujemny."));
        }
        if next > 100_000 {
            transaction.rollback().await?;
            return Err(AppError::conflict(
                "Stan nie może przekraczać 100000 sztuk.",
            ));
        }
        Some(next)
    };

    // Data obowiązywania stanu. Przy „Bez limitu” zachowujemy dzisiejszą datę,
    // jeśli pozycja ma domyślny stan — jutro reset przywróci limit. Bez
    // domyślnego stanu data nie jest potrzebna.
    let stock_date = (new_value.is_some() || stock_default.is_some()).then(|| today_key.clone());
    let result = sqlx::query(
        "UPDATE menu_items
         SET stock_remaining = ?, stock_date = ?, updated_at = ?
         WHERE id = ? AND stock_remaining IS ?",
    )
    .bind(new_value)
    .bind(&stock_date)
    .bind(util::now())
    .bind(id)
    .bind(old)
    .execute(&mut *transaction)
    .await?;

    if result.rows_affected() == 0 {
        transaction.rollback().await?;
        return Err(AppError::conflict(
            "Stan został właśnie zmieniony przez inną osobę. Odśwież listę.",
        ));
    }

    transaction.commit().await?;

    let describe = |value: Option<i64>| match value {
        Some(value) => value.to_string(),
        None => "bez limitu".to_string(),
    };
    db::audit(
        &state.db,
        Some(user.id),
        "menu_stock_changed",
        &format!(
            "Pozycja menu id={id} „{name}”, stan: {} -> {}",
            describe(old),
            describe(new_value)
        ),
    )
    .await;

    Ok(Json(get_item(&state, id).await?))
}

pub async fn hide_menu_item(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, AppError> {
    let result = sqlx::query("UPDATE menu_items SET available = 0, updated_at = ? WHERE id = ?")
        .bind(util::now())
        .bind(id)
        .execute(&state.db)
        .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::not_found("Nie znaleziono pozycji menu."));
    }

    db::audit(
        &state.db,
        Some(user.id),
        "menu_item_hidden",
        &format!("Wycofano pozycję menu id={id}"),
    )
    .await;

    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn get_item(state: &Arc<AppState>, id: i64) -> Result<MenuItemView, AppError> {
    sqlx::query_as::<_, MenuItemView>(
        "SELECT id, name, description, price_cents, category, allergens, available,
                sort_order, stock_remaining, stock_default, stock_date
         FROM menu_items WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::not_found("Nie znaleziono pozycji menu."))
}

async fn setting_i64(state: &Arc<AppState>, key: &str, default: i64) -> i64 {
    db::get_setting_or(&state.db, key, &default.to_string())
        .await
        .parse()
        .unwrap_or(default)
}

struct ValidatedMenuInput {
    name: String,
    description: String,
    price_cents: i64,
    category: String,
    allergens: String,
    available: bool,
    sort_order: i64,
    stock_remaining: Option<i64>,
    stock_default: Option<i64>,
}

/// Data, do której odnosi się dzisiejszy stan. Ustawiamy ją także wtedy, gdy
/// podano wyłącznie `stock_default`: inaczej pozycja „bez limitu na dziś, ale
/// z domyślnym stanem” nigdy nie zostałaby zresetowana następnego dnia.
fn stock_date_for(input: &ValidatedMenuInput) -> Option<String> {
    (input.stock_remaining.is_some() || input.stock_default.is_some())
        .then(|| crate::stock::date_key(util::today()))
}

fn validate_input(body: MenuItemInput) -> Result<ValidatedMenuInput, AppError> {
    let name = body.name.trim().to_string();
    if name.is_empty() || name.chars().count() > 80 {
        return Err(AppError::bad("Nazwa pozycji musi mieć od 1 do 80 znaków."));
    }
    let description = body.description.trim().to_string();
    if description.chars().count() > 240 {
        return Err(AppError::bad("Opis może mieć maksymalnie 240 znaków."));
    }
    if !(0..=100_000).contains(&body.price_cents) {
        return Err(AppError::bad(
            "Cena musi być między 0 a 1000 zł (w groszach).",
        ));
    }
    let category = body.category.trim().to_string();
    if category.is_empty() || category.chars().count() > 40 {
        return Err(AppError::bad("Kategoria musi mieć od 1 do 40 znaków."));
    }
    let allergens = body.allergens.trim().to_string();
    if allergens.chars().count() > 160 {
        return Err(AppError::bad(
            "Informacja o alergenach może mieć maksymalnie 160 znaków.",
        ));
    }
    if !(-10_000..=10_000).contains(&body.sort_order) {
        return Err(AppError::bad("Nieprawidłowa kolejność sortowania."));
    }
    for (label, value) in [
        ("Stan na dziś", body.stock_remaining),
        ("Domyślny stan dzienny", body.stock_default),
    ] {
        if let Some(value) = value
            && !(0..=100_000).contains(&value)
        {
            return Err(AppError::bad(format!(
                "{label} musi wynosić od 0 do 100000 sztuk."
            )));
        }
    }

    Ok(ValidatedMenuInput {
        name,
        description,
        price_cents: body.price_cents,
        category,
        allergens,
        available: body.available,
        sort_order: body.sort_order,
        stock_remaining: body.stock_remaining,
        stock_default: body.stock_default,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(remaining: Option<i64>, default: Option<i64>) -> ValidatedMenuInput {
        ValidatedMenuInput {
            name: "Test".into(),
            description: String::new(),
            price_cents: 500,
            category: "Dania".into(),
            allergens: String::new(),
            available: true,
            sort_order: 0,
            stock_remaining: remaining,
            stock_default: default,
        }
    }

    #[test]
    fn stock_date_is_set_when_any_limit_is_configured() {
        let today = crate::stock::date_key(util::today());
        assert_eq!(stock_date_for(&input(Some(5), None)), Some(today.clone()));
        assert_eq!(stock_date_for(&input(None, Some(10))), Some(today.clone()));
        assert_eq!(stock_date_for(&input(Some(5), Some(10))), Some(today));
        assert_eq!(stock_date_for(&input(None, None)), None);
    }
}
