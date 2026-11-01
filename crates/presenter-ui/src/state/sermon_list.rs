//! The sermon list's pure half (#851 spec §7.6): dates, counts and the split
//! into upcoming and older. Today always comes from the server, never the phone.

use chrono::{Datelike, NaiveDate, Weekday};

use crate::api::sermons::SermonSummary;

/// "Staršie" keeps this many rows.
const OLDER_ROWS: usize = 20;

/// `ne 19. 10.`
pub fn short_date(date: NaiveDate) -> String {
    let day = match date.weekday() {
        Weekday::Mon => "po",
        Weekday::Tue => "ut",
        Weekday::Wed => "st",
        Weekday::Thu => "št",
        Weekday::Fri => "pi",
        Weekday::Sat => "so",
        Weekday::Sun => "ne",
    };
    format!("{day} {}. {}.", date.day(), date.month())
}

fn counted(count: usize, one: &str, few: &str, many: &str) -> String {
    let word = match count {
        1 => one,
        2..=4 => few,
        _ => many,
    };
    format!("{count} {word}")
}

/// `3 položky · 4 slajdy`
pub fn count_text(items: usize, slides: usize) -> String {
    format!(
        "{} · {}",
        counted(items, "položka", "položky", "položiek"),
        counted(slides, "slajd", "slajdy", "slajdov")
    )
}

/// Upcoming (`service_date ≥ today`, soonest first) and the last 20 older ones
/// (newest first).
pub fn split_sermons(
    sermons: &[SermonSummary],
    today: NaiveDate,
) -> (Vec<SermonSummary>, Vec<SermonSummary>) {
    let (mut upcoming, mut older): (Vec<_>, Vec<_>) = sermons
        .iter()
        .cloned()
        .partition(|sermon| sermon.service_date >= today);
    upcoming.sort_by(|a, b| {
        a.service_date
            .cmp(&b.service_date)
            .then_with(|| a.title.cmp(&b.title))
    });
    older.sort_by(|a, b| {
        b.service_date
            .cmp(&a.service_date)
            .then_with(|| a.title.cmp(&b.title))
    });
    older.truncate(OLDER_ROWS);
    (upcoming, older)
}

/// "Moje": only the sermons this Access e-mail wrote.
pub fn only_mine(sermons: &[SermonSummary], email: &str) -> Vec<SermonSummary> {
    sermons
        .iter()
        .filter(|sermon| sermon.author_email.as_deref() == Some(email))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).expect("a date")
    }

    fn sermon(title: &str, service_date: NaiveDate, author: Option<&str>) -> SermonSummary {
        SermonSummary {
            id: title.to_string(),
            title: title.to_string(),
            service_date,
            author_email: author.map(str::to_string),
            item_count: 0,
            slide_count: 0,
            revision: 0,
        }
    }

    #[test]
    fn a_date_reads_as_the_weekday_day_and_month() {
        assert_eq!(short_date(date(2026, 10, 18)), "ne 18. 10.");
        assert_eq!(short_date(date(2026, 11, 2)), "po 2. 11.");
        assert_eq!(short_date(date(2026, 11, 5)), "št 5. 11.");
    }

    #[test]
    fn counts_follow_slovak_plurals() {
        assert_eq!(count_text(1, 1), "1 položka · 1 slajd");
        assert_eq!(count_text(3, 4), "3 položky · 4 slajdy");
        assert_eq!(count_text(5, 12), "5 položiek · 12 slajdov");
        assert_eq!(count_text(0, 0), "0 položiek · 0 slajdov");
    }

    #[test]
    fn today_is_upcoming_and_the_soonest_comes_first() {
        let today = date(2026, 11, 1);
        let list = [
            sermon("neskôr", date(2026, 11, 8), None),
            sermon("dnes", today, None),
            sermon("včera", date(2026, 10, 31), None),
            sermon("dávno", date(2026, 1, 1), None),
        ];
        let (upcoming, older) = split_sermons(&list, today);
        let titles =
            |rows: &[SermonSummary]| rows.iter().map(|r| r.title.clone()).collect::<Vec<_>>();
        assert_eq!(titles(&upcoming), ["dnes", "neskôr"]);
        assert_eq!(titles(&older), ["včera", "dávno"]);
    }

    #[test]
    fn older_keeps_the_newest_20() {
        let today = date(2026, 11, 1);
        let list: Vec<_> = (1..=25)
            .map(|day| sermon(&day.to_string(), date(2026, 1, day), None))
            .collect();
        let (_, older) = split_sermons(&list, today);
        assert_eq!(older.len(), 20);
        assert_eq!(older[0].title, "25");
        assert_eq!(older[19].title, "6");
    }

    #[test]
    fn mine_needs_the_same_email() {
        let day = date(2026, 11, 1);
        let list = [
            sermon("moja", day, Some("a@b.sk")),
            sermon("cudzia", day, Some("c@d.sk")),
            sermon("bez autora", day, None),
        ];
        let mine = only_mine(&list, "a@b.sk");
        assert_eq!(mine.len(), 1);
        assert_eq!(mine[0].title, "moja");
    }
}
