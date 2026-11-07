use chrono::{Local, NaiveDate, TimeZone};

/// Znacznik czasu Unix (sekundy). Wszystkie czasy w bazie są w UTC.
pub fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Początek bieżącego dnia w czasie lokalnym (dla statystyk dziennych).
pub fn start_of_today() -> i64 {
    start_of_local_date(Local::now().date_naive())
}

/// Początek wskazanego dnia w czasie lokalnym (Unix UTC).
pub fn start_of_local_date(date: NaiveDate) -> i64 {
    let naive = match date.and_hms_opt(0, 0, 0) {
        Some(value) => value,
        None => return Local::now().timestamp(),
    };
    Local
        .from_local_datetime(&naive)
        .earliest()
        .map(|value| value.timestamp())
        .unwrap_or_else(|| Local::now().timestamp())
}

/// Data lokalna odpowiadająca znacznikowi czasu UTC.
pub fn local_date(timestamp: i64) -> Option<NaiveDate> {
    Local
        .timestamp_opt(timestamp, 0)
        .single()
        .map(|value| value.date_naive())
}

/// Godzina lokalna (0–23) odpowiadająca znacznikowi czasu UTC.
pub fn local_hour(timestamp: i64) -> Option<u32> {
    use chrono::Timelike;
    Local
        .timestamp_opt(timestamp, 0)
        .single()
        .map(|value| value.hour())
}

/// Akceptujemy tylko godzinę w formacie HH:MM (opcjonalne pole odbioru).
pub fn valid_pickup_time(value: &str) -> bool {
    if value.is_empty() {
        return true;
    }
    let bytes = value.as_bytes();
    if bytes.len() != 5 || bytes[2] != b':' {
        return false;
    }
    let hour = &value[0..2];
    let minute = &value[3..5];
    if !hour.bytes().all(|b| b.is_ascii_digit()) || !minute.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    match (hour.parse::<u32>(), minute.parse::<u32>()) {
        (Ok(h), Ok(m)) => h < 24 && m < 60,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pickup_time_validation() {
        assert!(valid_pickup_time(""));
        assert!(valid_pickup_time("08:30"));
        assert!(valid_pickup_time("23:59"));
        assert!(!valid_pickup_time("24:00"));
        assert!(!valid_pickup_time("7:30"));
        assert!(!valid_pickup_time("12-30"));
        assert!(!valid_pickup_time("12:60"));
    }
}
