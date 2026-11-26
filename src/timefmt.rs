use chrono::{Local, TimeZone};

pub fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

pub fn format_local(ms: i64) -> String {
    Local
        .timestamp_millis_opt(ms)
        .single()
        .map(|time| time.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_epoch_as_local_datetime() {
        let text = format_local(0);
        assert_eq!(text.len(), 19);
        assert_eq!(&text[4..5], "-");
        assert_eq!(&text[10..11], " ");
        assert_eq!(&text[13..14], ":");
    }

    #[test]
    fn now_is_positive() {
        assert!(now_ms() > 0);
    }
}
