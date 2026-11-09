use crate::db;
use crate::error::AppError;
use crate::state::AppState;
use axum::{Json, extract::State};
use serde::Serialize;
use serde_json::{Value, json};
use std::sync::Arc;

#[derive(Serialize)]
pub struct PublicInfo {
    pub cafeteria_name: String,
    pub ordering_enabled: bool,
    pub payment_info: String,
}

pub async fn info(State(state): State<Arc<AppState>>) -> Result<Json<PublicInfo>, AppError> {
    Ok(Json(PublicInfo {
        cafeteria_name: db::get_setting_or(&state.db, "cafeteria_name", "Stołówka ZSŁ").await,
        ordering_enabled: db::get_setting_or(&state.db, "ordering_enabled", "1").await == "1",
        payment_info: db::get_setting_or(
            &state.db,
            "payment_info",
            "Płatność przy odbiorze: gotówka lub karta.",
        )
        .await,
    }))
}

/// Prosty test żywotności dla monitoringu i systemd/Cloudflare. Nie ujawnia
/// wersji, konfiguracji ani danych. Sprawdza tylko, że baza odpowiada.
pub async fn healthz(State(state): State<Arc<AppState>>) -> Result<Json<Value>, AppError> {
    sqlx::query_scalar::<_, i64>("SELECT 1")
        .fetch_one(&state.db)
        .await
        .map_err(|error| AppError::internal(format!("healthz: {error}")))?;

    Ok(Json(json!({ "status": "ok" })))
}
