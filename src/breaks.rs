// SPDX-License-Identifier: MIT
//! Przerwy (okna wydawania) i dzień odbioru zamówienia.
//!
//! Przerwy są wspólne dla wszystkich dni: „Długa przerwa” 11:30–11:50
//! obowiązuje codziennie. Zamówienie wskazuje przerwę (`break_id`) oraz
//! dzień odbioru (`pickup_date`). Limity „na przerwę” liczą się dla pary
//! (uczeń, dzień, przerwa), dzięki czemu zamówienie na jutro nie blokuje
//! dzisiejszego.

use crate::util;
use chrono::NaiveDate;
use sqlx::{FromRow, SqlitePool};

#[derive(Clone, Debug, FromRow, serde::Serialize)]
pub struct BreakView {
    pub id: i64,
    pub name: String,
    pub start_time: String,
    pub end_time: String,
    pub active: bool,
    pub sort_order: i64,
}

pub async fn list_all(pool: &SqlitePool) -> Result<Vec<BreakView>, sqlx::Error> {
    sqlx::query_as(
        "SELECT id, name, start_time, end_time, active, sort_order
         FROM breaks
         ORDER BY sort_order ASC, start_time ASC, name ASC",
    )
    .fetch_all(pool)
    .await
}

pub async fn list_active(pool: &SqlitePool) -> Result<Vec<BreakView>, sqlx::Error> {
    sqlx::query_as(
        "SELECT id, name, start_time, end_time, active, sort_order
         FROM breaks
         WHERE active = 1
         ORDER BY sort_order ASC, start_time ASC, name ASC",
    )
    .fetch_all(pool)
    .await
}

/// Walidacja pól przerwy. Zwraca komunikat gotowy do pokazania w API.
pub fn validate(name: &str, start_time: &str, end_time: &str) -> Result<(), String> {
    if name.chars().count() > 60 {
        return Err("Nazwa przerwy może mieć maksymalnie 60 znaków.".to_string());
    }
    if name.is_empty() {
        return Err("Nazwa przerwy nie może być pusta.".to_string());
    }
    if !valid_clock_time(start_time) {
        return Err("Początek przerwy musi być w formacie HH:MM.".to_string());
    }
    if !valid_clock_time(end_time) {
        return Err("Koniec przerwy musi być w formacie HH:MM.".to_string());
    }
    if start_time >= end_time {
        return Err("Koniec przerwy musi być później niż początek.".to_string());
    }
    Ok(())
}

/// Ścisła walidacja godziny `HH:MM` (bez pustej wartości, w odróżnieniu
/// od opcjonalnej godziny odbioru).
pub fn valid_clock_time(value: &str) -> bool {
    !value.is_empty() && util::valid_pickup_time(value)
}

/// Czy godzina mieści się w oknie przerwy (włącznie z końcami).
pub fn time_within(time: &str, start_time: &str, end_time: &str) -> bool {
    valid_clock_time(time) && time >= start_time && time <= end_time
}

/// Dzień odbioru zamówienia: jawny `pickup_date`, a dla starych wierszy
/// dzień złożenia w strefie szkoły.
pub fn order_date(created_at: i64, pickup_date: Option<&str>) -> Option<NaiveDate> {
    pickup_date
        .and_then(|value| NaiveDate::parse_from_str(value.trim(), "%Y-%m-%d").ok())
        .or_else(|| util::local_date(created_at))
}

/// Moment, po którym zamówienie z wybraną przerwą uznajemy za
/// nieodebrane: koniec przerwy + `pickup_limit_minutes`, w strefie
/// szkoły. `0` oznacza wyłączone auto-oznaczanie.
pub fn uncollected_deadline(
    pickup_date: NaiveDate,
    end_time: &str,
    limit_minutes: i64,
) -> Option<i64> {
    if limit_minutes <= 0 {
        return None;
    }
    let end = util::local_datetime_timestamp(pickup_date, end_time)?;
    Some(end + limit_minutes * 60)
}

/// Czy zamówienie dotyczy dzisiejszego dnia (a więc czy wolno ruszać
/// dzisiejszy stan magazynowy)?
pub fn is_order_from_today(created_at: i64, pickup_date: Option<&str>) -> bool {
    order_date(created_at, pickup_date) == Some(util::today())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_break_window() {
        assert!(validate("Długa przerwa", "11:30", "11:50").is_ok());
        assert!(validate("", "11:30", "11:50").is_err());
        assert!(validate("Przerwa", "25:00", "26:00").is_err());
        assert!(validate("Przerwa", "11:50", "11:30").is_err());
        assert!(validate("Przerwa", "11:30", "11:30").is_err());
        assert!(validate(&"x".repeat(61), "11:30", "11:50").is_err());
    }

    #[test]
    fn checks_time_inside_window() {
        assert!(time_within("11:30", "11:30", "11:50"));
        assert!(time_within("11:40", "11:30", "11:50"));
        assert!(time_within("11:50", "11:30", "11:50"));
        assert!(!time_within("11:29", "11:30", "11:50"));
        assert!(!time_within("11:51", "11:30", "11:50"));
        assert!(!time_within("", "11:30", "11:50"));
    }

    #[test]
    fn deadline_is_end_of_break_plus_limit_on_dst_days() {
        // Wiosenna zmiana czasu: 11:50 CEST to 09:50 UTC.
        let spring = NaiveDate::from_ymd_opt(2026, 3, 29).unwrap();
        let spring_end = util::local_datetime_timestamp(spring, "11:50").unwrap();
        assert_eq!(
            uncollected_deadline(spring, "11:50", 30),
            Some(spring_end + 30 * 60)
        );
        // Jesienna zmiana czasu: 11:50 CET to 10:50 UTC.
        let autumn = NaiveDate::from_ymd_opt(2026, 10, 25).unwrap();
        let autumn_end = util::local_datetime_timestamp(autumn, "11:50").unwrap();
        assert_eq!(
            uncollected_deadline(autumn, "11:50", 30),
            Some(autumn_end + 30 * 60)
        );
        assert_eq!(uncollected_deadline(spring, "11:50", 0), None);
        assert_eq!(uncollected_deadline(spring, "zła", 30), None);
    }

    #[test]
    fn falls_back_to_created_day_for_old_orders() {
        let today = util::today();
        let created = util::start_of_local_date(today) + 3600;
        assert_eq!(order_date(created, None), Some(today));
        assert_eq!(
            order_date(created, Some("2030-01-02")),
            NaiveDate::from_ymd_opt(2030, 1, 2)
        );
        assert_eq!(order_date(created, Some("nie-data")), Some(today));
        assert!(is_order_from_today(created, None));
    }
}
