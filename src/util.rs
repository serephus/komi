//! Small time helpers. Every timestamp in the database is a Unix timestamp in
//! seconds, which keeps the SQL boring and the formatting logic in one place.

use time::macros::format_description;
use time::OffsetDateTime;

pub fn now_unix() -> i64 {
    OffsetDateTime::now_utc().unix_timestamp()
}

/// `YYYY-MM-DD`, used for SGF.
pub fn date_ymd(timestamp: i64) -> String {
    let format = format_description!("[year]-[month]-[day]");
    OffsetDateTime::from_unix_timestamp(timestamp)
        .ok()
        .and_then(|dt| dt.format(&format).ok())
        .unwrap_or_else(|| "1970-01-01".to_owned())
}

/// `YYYY-MM-DD HH:MM`, for the move list.
pub fn date_time(timestamp: i64) -> String {
    let format = format_description!("[year]-[month]-[day] [hour]:[minute]");
    OffsetDateTime::from_unix_timestamp(timestamp)
        .ok()
        .and_then(|dt| dt.format(&format).ok())
        .unwrap_or_else(|| "?".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_are_zero_padded() {
        let timestamp = OffsetDateTime::from_unix_timestamp(1_700_000_000)
            .unwrap()
            .unix_timestamp();
        assert_eq!(date_ymd(timestamp).len(), 10);
        assert!(date_ymd(timestamp).starts_with("2023-11"));
        assert_eq!(date_time(timestamp).len(), 16);
        assert_eq!(date_ymd(0), "1970-01-01");
    }
}
