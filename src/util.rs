use chrono::{DateTime, Duration as ChronoDuration, LocalResult, NaiveDate, TimeZone, Utc};
use chrono_tz::Europe::Warsaw;
use chrono_tz::Tz;

/// Strefa czasowa szkoły. Wszystkie znaczniki w bazie pozostają w UTC,
/// ale doby, raporty i godziny prezentowane użytkownikowi liczymy
/// wyłącznie w `Europe/Warsaw` — niezależnie od strefy serwera.
pub const APP_TZ: Tz = Warsaw;

/// Znacznik czasu Unix (sekundy). Wszystkie czasy w bazie są w UTC.
pub fn now() -> i64 {
    Utc::now().timestamp()
}

/// Bieżąca data w strefie szkoły.
pub fn today() -> NaiveDate {
    warsaw_now().date_naive()
}

/// Bieżący czas w strefie szkoły.
pub fn warsaw_now() -> DateTime<Tz> {
    Utc::now().with_timezone(&APP_TZ)
}

/// Początek bieżącego dnia w strefie szkoły (dla statystyk dziennych).
pub fn start_of_today() -> i64 {
    start_of_local_date(today())
}

/// Początek wskazanego dnia w strefie szkoły (Unix UTC).
pub fn start_of_local_date(date: NaiveDate) -> i64 {
    let naive = match date.and_hms_opt(0, 0, 0) {
        Some(value) => value,
        None => return now(),
    };
    match APP_TZ.from_local_datetime(&naive) {
        LocalResult::Single(value) => value.timestamp(),
        // W Europie zmiana czasu wypada o 02:00/03:00, więc północ nie jest
        // niejednoznaczna. Wybieramy wcześniejszy wariant na wypadek zmian
        // historycznych.
        LocalResult::Ambiguous(earliest, _) => earliest.timestamp(),
        // Teoretyczna luka DST o północy: traktujemy czas jak UTC, aby
        // funkcja zawsze zwróciła sensowny znacznik.
        LocalResult::None => Utc.from_utc_datetime(&naive).timestamp(),
    }
}

/// Data w strefie szkoły odpowiadająca znacznikowi czasu UTC.
pub fn local_date(timestamp: i64) -> Option<NaiveDate> {
    Utc.timestamp_opt(timestamp, 0)
        .single()
        .map(|value| value.with_timezone(&APP_TZ).date_naive())
}

/// Godzina w strefie szkoły (0–23) odpowiadająca znacznikowi czasu UTC.
pub fn local_hour(timestamp: i64) -> Option<u32> {
    use chrono::Timelike;
    Utc.timestamp_opt(timestamp, 0)
        .single()
        .map(|value| value.with_timezone(&APP_TZ).hour())
}

/// Najbliższe wystąpienie godziny `hour:minute` w strefie szkoły, ściśle
/// po `now`. Funkcja obsługuje zmianę czasu: wybiera wcześniejszy wariant
/// godziny niejednoznacznej i pomija godzinę, która nie istnieje.
pub fn next_daily_run(now: DateTime<Tz>, hour: u32, minute: u32) -> DateTime<Tz> {
    let today = now.date_naive();
    for offset in 0..3 {
        let date = today + ChronoDuration::days(offset);
        let Some(naive) = date.and_hms_opt(hour, minute, 0) else {
            continue;
        };
        let candidate = match APP_TZ.from_local_datetime(&naive) {
            LocalResult::Single(value) => value,
            LocalResult::Ambiguous(earliest, _) => earliest,
            LocalResult::None => continue,
        };
        if candidate > now {
            return candidate;
        }
    }
    now + ChronoDuration::hours(24)
}

/// Czas do najbliższego uruchomienia zadania o `hour:minute` czasu szkoły.
pub fn duration_until_next_daily_run(hour: u32, minute: u32) -> std::time::Duration {
    let now = warsaw_now();
    let next = next_daily_run(now, hour, minute);
    (next - now)
        .to_std()
        .unwrap_or(std::time::Duration::from_secs(60))
}

/// Data i godzina `RRRR-MM-DD HH:MM` w strefie szkoły.
pub fn format_local_datetime(timestamp: i64) -> String {
    Utc.timestamp_opt(timestamp, 0)
        .single()
        .map(|value| {
            value
                .with_timezone(&APP_TZ)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_default()
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
    use chrono::Duration;

    fn date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).expect("poprawna data")
    }

    #[test]
    fn next_daily_run_is_strictly_in_the_future() {
        let before = APP_TZ.with_ymd_and_hms(2026, 11, 11, 2, 0, 0).unwrap();
        let next = next_daily_run(before, 3, 30);
        assert_eq!(local_date(next.timestamp()), Some(date(2026, 11, 11)));
        assert_eq!(format_local_datetime(next.timestamp()), "2026-11-11 03:30");
        assert!(next > before);

        let after = APP_TZ.with_ymd_and_hms(2026, 11, 11, 4, 0, 0).unwrap();
        let next = next_daily_run(after, 3, 30);
        assert_eq!(local_date(next.timestamp()), Some(date(2026, 11, 12)));
        assert_eq!(format_local_datetime(next.timestamp()), "2026-11-12 03:30");
    }

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

    #[test]
    fn midnight_belongs_to_the_warsaw_day() {
        let day = date(2026, 11, 10);
        let start = start_of_local_date(day);
        let end = start_of_local_date(day + Duration::days(1));

        // 10 listopada 2026 to czas zimowy (UTC+1).
        assert_eq!(
            start,
            Utc.with_ymd_and_hms(2026, 11, 9, 23, 0, 0)
                .unwrap()
                .timestamp()
        );
        assert_eq!(end - start, 86_400);
        assert_eq!(local_date(start), Some(day));
        assert_eq!(local_date(end - 1), Some(day));
        assert_eq!(local_date(end), Some(day + Duration::days(1)));

        assert_eq!(local_date(start - 60), Some(day - Duration::days(1)));
        assert_eq!(local_date(end + 60), Some(day + Duration::days(1)));
    }

    #[test]
    fn spring_forward_day_has_23_hours() {
        let day = date(2026, 3, 29);
        let start = start_of_local_date(day);
        let end = start_of_local_date(day + Duration::days(1));

        assert_eq!(
            start,
            Utc.with_ymd_and_hms(2026, 3, 28, 23, 0, 0)
                .unwrap()
                .timestamp()
        );
        assert_eq!(end - start, 23 * 3_600);
        assert_eq!(local_date(start), Some(day));
        assert_eq!(local_date(end - 1), Some(day));
    }

    #[test]
    fn fall_back_day_has_25_hours() {
        let day = date(2026, 10, 25);
        let start = start_of_local_date(day);
        let end = start_of_local_date(day + Duration::days(1));

        assert_eq!(
            start,
            Utc.with_ymd_and_hms(2026, 10, 24, 22, 0, 0)
                .unwrap()
                .timestamp()
        );
        assert_eq!(end - start, 25 * 3_600);
        assert_eq!(local_date(start), Some(day));
        assert_eq!(local_date(end - 1), Some(day));
    }

    #[test]
    fn local_hour_follows_warsaw_time() {
        // 23:59 czasu warszawskiego (UTC+1) i 00:01 następnego dnia.
        let before_midnight = Utc
            .with_ymd_and_hms(2026, 11, 10, 22, 59, 0)
            .unwrap()
            .timestamp();
        let after_midnight = Utc
            .with_ymd_and_hms(2026, 11, 10, 23, 1, 0)
            .unwrap()
            .timestamp();

        assert_eq!(local_hour(before_midnight), Some(23));
        assert_eq!(local_date(before_midnight), Some(date(2026, 11, 10)));
        assert_eq!(local_hour(after_midnight), Some(0));
        assert_eq!(local_date(after_midnight), Some(date(2026, 11, 11)));
        assert_eq!(format_local_datetime(before_midnight), "2026-11-10 23:59");
    }
}
