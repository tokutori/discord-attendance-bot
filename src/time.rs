use chrono::{DateTime, LocalResult, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};
use chrono_tz::{Asia::Tokyo, Tz};
use thiserror::Error;

use crate::attendance::YearMonth;

pub const DISPLAY_TIMEZONE: Tz = Tokyo;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ParseTimeError {
    #[error("時刻は HH:MM 形式で指定する必要がある")]
    InvalidTime,
    #[error("日時は YYYY-MM-DD HH:MM 形式で指定する必要がある")]
    InvalidDateTime,
    #[error("指定した日時を日本時間として解釈できない")]
    AmbiguousOrInvalidLocalTime,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ParseYearMonthError {
    #[error("年月は YYYY-MM 形式で指定する必要がある")]
    InvalidFormat,
    #[error("月は1から12の範囲で指定する必要がある")]
    InvalidMonth,
}

pub fn parse_year_month(input: &str) -> Result<YearMonth, ParseYearMonthError> {
    let (year, month) = input
        .split_once('-')
        .ok_or(ParseYearMonthError::InvalidFormat)?;
    if year.len() != 4 || month.len() != 2 {
        return Err(ParseYearMonthError::InvalidFormat);
    }
    let year = year
        .parse()
        .map_err(|_| ParseYearMonthError::InvalidFormat)?;
    let month: u32 = month
        .parse()
        .map_err(|_| ParseYearMonthError::InvalidFormat)?;
    if !(1..=12).contains(&month) {
        return Err(ParseYearMonthError::InvalidMonth);
    }
    Ok(YearMonth { year, month })
}

fn local_to_timestamp(value: NaiveDateTime) -> Result<i64, ParseTimeError> {
    match DISPLAY_TIMEZONE.from_local_datetime(&value) {
        LocalResult::Single(dt) => Ok(dt.timestamp()),
        _ => Err(ParseTimeError::AmbiguousOrInvalidLocalTime),
    }
}

pub fn parse_today_time(input: &str, now_utc: DateTime<Utc>) -> Result<i64, ParseTimeError> {
    let time =
        NaiveTime::parse_from_str(input, "%H:%M").map_err(|_| ParseTimeError::InvalidTime)?;
    let date = now_utc.with_timezone(&DISPLAY_TIMEZONE).date_naive();
    local_to_timestamp(date.and_time(time))
}

pub fn parse_full_datetime(input: &str) -> Result<i64, ParseTimeError> {
    let value = NaiveDateTime::parse_from_str(input, "%Y-%m-%d %H:%M")
        .map_err(|_| ParseTimeError::InvalidDateTime)?;
    local_to_timestamp(value)
}

pub fn format_datetime(timestamp: i64) -> String {
    DateTime::<Utc>::from_timestamp(timestamp, 0)
        .map(|dt| {
            dt.with_timezone(&DISPLAY_TIMEZONE)
                .format("%Y年%-m月%-d日 %H:%M")
                .to_string()
        })
        .unwrap_or_else(|| "不正な時刻".into())
}

pub fn format_history_range(started_at: i64, ended_at: Option<i64>) -> String {
    let start = DateTime::<Utc>::from_timestamp(started_at, 0)
        .unwrap()
        .with_timezone(&DISPLAY_TIMEZONE);
    match ended_at {
        None => format!("{} ～ 現在", start.format("%Y/%m/%d %H:%M")),
        Some(end) => {
            let end = DateTime::<Utc>::from_timestamp(end, 0)
                .unwrap()
                .with_timezone(&DISPLAY_TIMEZONE);
            if start.date_naive() == end.date_naive() {
                format!(
                    "{}\n{} ～ {}",
                    start.format("%Y/%m/%d"),
                    start.format("%H:%M"),
                    end.format("%H:%M")
                )
            } else {
                format!(
                    "{} ～ {}",
                    start.format("%Y/%m/%d %H:%M"),
                    end.format("%Y/%m/%d %H:%M")
                )
            }
        }
    }
}

pub fn format_duration(seconds: i64) -> String {
    let minutes = seconds.max(0) / 60;
    let hours = minutes / 60;
    let minutes = minutes % 60;
    match (hours, minutes) {
        (0, m) => format!("{m}分"),
        (h, 0) => format!("{h}時間"),
        (h, m) => format!("{h}時間{m:02}分"),
    }
}

pub fn month_bounds(ym: YearMonth) -> anyhow::Result<(i64, i64)> {
    let start_date = NaiveDate::from_ymd_opt(ym.year, ym.month, 1)
        .ok_or_else(|| anyhow::anyhow!("invalid year-month"))?;
    let (next_year, next_month) = if ym.month == 12 {
        (ym.year + 1, 1)
    } else {
        (ym.year, ym.month + 1)
    };
    let end_date = NaiveDate::from_ymd_opt(next_year, next_month, 1)
        .ok_or_else(|| anyhow::anyhow!("invalid next month"))?;
    Ok((
        local_to_timestamp(start_date.and_hms_opt(0, 0, 0).unwrap())?,
        local_to_timestamp(end_date.and_hms_opt(0, 0, 0).unwrap())?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn parses_year_month() {
        assert_eq!(
            parse_year_month("2026-08").unwrap(),
            YearMonth {
                year: 2026,
                month: 8
            }
        );
        assert!(parse_year_month("2026-8").is_err());
        assert!(parse_year_month("2026-13").is_err());
    }

    #[test]
    fn parses_today_time_in_jst() {
        let now = Utc.with_ymd_and_hms(2026, 8, 6, 10, 0, 0).unwrap();
        let ts = parse_today_time("13:30", now).unwrap();
        assert_eq!(format_datetime(ts), "2026年8月6日 13:30");
    }
}
