use crate::db;
use crate::error::AppError;
use crate::state::AppState;
use axum::{Json, extract::State};
use serde::Serialize;
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
