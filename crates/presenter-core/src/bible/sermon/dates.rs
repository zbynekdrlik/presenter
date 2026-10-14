//! Sermon defaults that depend on the date (#851 spec §7.6, decided default 7).
//! The server computes them from ITS clock and sends them, so a phone's clock
//! never decides.

use chrono::{Datelike, Days, NaiveDate};

/// The next Sunday; today when today is a Sunday.
pub fn default_service_date(today: NaiveDate) -> NaiveDate {
    let ahead = (7 - today.weekday().num_days_from_sunday()) % 7;
    today
        .checked_add_days(Days::new(u64::from(ahead)))
        .unwrap_or(today)
}

/// "Kázeň 19. 10. 2026".
pub fn default_sermon_title(date: NaiveDate) -> String {
    let (day, month, year) = (date.day(), date.month(), date.year());
    format!("Kázeň {day}. {month}. {year}")
}
