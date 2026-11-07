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
use sqlx::FromRow;
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
}

#[derive(Serialize)]
pub struct StudentMenuResponse {
    pub items: Vec<MenuItemView>,
    pub ordering_enabled: bool,
    pub payment_info: String,
    pub max_order_items: i64,
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
    let items = sqlx::query_as::<_, MenuItemView>(
        "SELECT id, name, description, price_cents, category, allergens, available, sort_order
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
    }))
}

pub async fn staff_menu(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<MenuItemView>>, AppError> {
    let items = sqlx::query_as::<_, MenuItemView>(
        "SELECT id, name, description, price_cents, category, allergens, available, sort_order
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

    let result = sqlx::query(
        "INSERT INTO menu_items
         (name, description, price_cents, category, allergens, available, sort_order, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
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

    let result = sqlx::query(
        "UPDATE menu_items
         SET name = ?, description = ?, price_cents = ?, category = ?, allergens = ?,
             available = ?, sort_order = ?, updated_at = ?
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
        "SELECT id, name, description, price_cents, category, allergens, available, sort_order
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

    Ok(ValidatedMenuInput {
        name,
        description,
        price_cents: body.price_cents,
        category,
        allergens,
        available: body.available,
        sort_order: body.sort_order,
    })
}
