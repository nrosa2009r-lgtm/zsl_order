// SPDX-License-Identifier: MIT
//! API przerw: administrator zarządza, kuchnia i uczeń tylko czytają.

use crate::auth::AuthUser;
use crate::breaks::{self, BreakView};
use crate::db;
use crate::error::AppError;
use crate::state::AppState;
use crate::util;
use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::Deserialize;
use std::sync::Arc;

#[derive(Deserialize)]
pub struct BreakInput {
    pub name: String,
    pub start_time: String,
    pub end_time: String,
    #[serde(default = "default_true")]
    pub active: bool,
    #[serde(default)]
    pub sort_order: i64,
}

fn default_true() -> bool {
    true
}

pub async fn admin_breaks(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<BreakView>>, AppError> {
    Ok(Json(breaks::list_all(&state.db).await?))
}

/// Lista dla kuchni: tylko aktywne przerwy, bez danych administracyjnych
/// (choć przerwy nie zawierają danych osobowych).
pub async fn staff_breaks(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<BreakView>>, AppError> {
    Ok(Json(breaks::list_active(&state.db).await?))
}

pub async fn create_break(
    State(state): State<Arc<AppState>>,
    Extension(actor): Extension<AuthUser>,
    Json(body): Json<BreakInput>,
) -> Result<(StatusCode, Json<BreakView>), AppError> {
    let input = validate(body)?;
    let now = util::now();
    let result = sqlx::query(
        "INSERT INTO breaks (name, start_time, end_time, active, sort_order, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&input.name)
    .bind(&input.start_time)
    .bind(&input.end_time)
    .bind(input.active)
    .bind(input.sort_order)
    .bind(now)
    .bind(now)
    .execute(&state.db)
    .await
    .map_err(map_break_error)?;

    let id = result.last_insert_rowid();
    db::audit(
        &state.db,
        Some(actor.id),
        "break_created",
        &format!(
            "Przerwa id={id}, {} {}-{}",
            input.name, input.start_time, input.end_time
        ),
    )
    .await;

    Ok((StatusCode::CREATED, Json(get_break(&state, id).await?)))
}

pub async fn update_break(
    State(state): State<Arc<AppState>>,
    Extension(actor): Extension<AuthUser>,
    Path(id): Path<i64>,
    Json(body): Json<BreakInput>,
) -> Result<Json<BreakView>, AppError> {
    let input = validate(body)?;
    let result = sqlx::query(
        "UPDATE breaks
         SET name = ?, start_time = ?, end_time = ?, active = ?, sort_order = ?, updated_at = ?
         WHERE id = ?",
    )
    .bind(&input.name)
    .bind(&input.start_time)
    .bind(&input.end_time)
    .bind(input.active)
    .bind(input.sort_order)
    .bind(util::now())
    .bind(id)
    .execute(&state.db)
    .await
    .map_err(map_break_error)?;

    if result.rows_affected() == 0 {
        return Err(AppError::not_found("Nie znaleziono przerwy."));
    }

    db::audit(
        &state.db,
        Some(actor.id),
        "break_updated",
        &format!(
            "Przerwa id={id}, {} {}-{}, aktywna={}",
            input.name, input.start_time, input.end_time, input.active
        ),
    )
    .await;

    Ok(Json(get_break(&state, id).await?))
}

/// Usunięcie jest dozwolone tylko dla przerwy bez zamówień. Przerwę,
/// która ma historię, należy wyłączyć (`active = false`) — dzięki temu
/// raporty i karty zamówień zachowują nazwę przerwy.
pub async fn delete_break(
    State(state): State<Arc<AppState>>,
    Extension(actor): Extension<AuthUser>,
    Path(id): Path<i64>,
) -> Result<StatusCode, AppError> {
    let used: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM orders WHERE break_id = ?")
        .bind(id)
        .fetch_one(&state.db)
        .await?;
    if used > 0 {
        return Err(AppError::conflict(
            "Ta przerwa ma zamówienia. Wyłącz ją zamiast usuwać, aby zachować historię.",
        ));
    }

    let result = sqlx::query("DELETE FROM breaks WHERE id = ?")
        .bind(id)
        .execute(&state.db)
        .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::not_found("Nie znaleziono przerwy."));
    }

    db::audit(
        &state.db,
        Some(actor.id),
        "break_deleted",
        &format!("Przerwa id={id}"),
    )
    .await;

    Ok(StatusCode::NO_CONTENT)
}

struct ValidBreak {
    name: String,
    start_time: String,
    end_time: String,
    active: bool,
    sort_order: i64,
}

fn validate(body: BreakInput) -> Result<ValidBreak, AppError> {
    let name = body.name.trim().to_string();
    let start_time = body.start_time.trim().to_string();
    let end_time = body.end_time.trim().to_string();
    breaks::validate(&name, &start_time, &end_time).map_err(AppError::bad)?;
    if !(-1000..=1000).contains(&body.sort_order) {
        return Err(AppError::bad(
            "Kolejność przerwy musi wynosić od -1000 do 1000.",
        ));
    }
    Ok(ValidBreak {
        name,
        start_time,
        end_time,
        active: body.active,
        sort_order: body.sort_order,
    })
}

fn map_break_error(error: sqlx::Error) -> AppError {
    if let sqlx::Error::Database(database) = &error
        && database.message().contains("breaks.name")
    {
        return AppError::conflict("Przerwa o tej nazwie już istnieje.");
    }
    AppError::from(error)
}

async fn get_break(state: &Arc<AppState>, id: i64) -> Result<BreakView, AppError> {
    sqlx::query_as(
        "SELECT id, name, start_time, end_time, active, sort_order FROM breaks WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::not_found("Nie znaleziono przerwy."))
}
