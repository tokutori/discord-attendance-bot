use std::sync::OnceLock;

use chrono::{DateTime, LocalResult, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};
use chrono_tz::{Asia::Tokyo, Tz};
use thiserror::Error;

use crate::attendance::YearMonth;

pub const DISPLAY_TIMEZONE: Tz = Tokyo;

#[derive(Debug, Clone, Copy)]
pub struct TimePolicy {
    pub timezone: Tz,
    pub auto_end_time: Option<NaiveTime>,
}

impl Default for TimePolicy {
    fn default() -> Self {
        Self {
            timezone: DISPLAY_TIMEZONE,
            auto_end_time: NaiveTime::from_hms_opt(21, 0, 0),
        }
    }
}

static TIME_POLICY: OnceLock<TimePolicy> = OnceLock::new();

pub fn install_time_policy(policy: TimePolicy) -> anyhow::Result<()> {
    TIME_POLICY
        .set(policy)
        .map_err(|_| anyhow::anyhow!("time policy is already configured"))
}

pub fn time_policy() -> TimePolicy {
    TIME_POLICY.get().copied().unwrap_or_default()
}

pub fn display_timezone() -> Tz {
    time_policy().timezone
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ParseTimeError {
    #[error("時刻は HH:MM 形式で指定する必要がある")]
    InvalidTime,
    #[error("日時は YYYY-MM-DD HH:MM 形式で指定する必要がある")]
    InvalidDateTime,
    #[error("指定した日時を設定されたタイムゾーンで解釈できない")]
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
    local_to_timestamp_in(display_timezone(), value)
}

fn local_to_timestamp_in(timezone: Tz, value: NaiveDateTime) -> Result<i64, ParseTimeError> {
    match timezone.from_local_datetime(&value) {
        LocalResult::Single(dt) => Ok(dt.timestamp()),
        _ => Err(ParseTimeError::AmbiguousOrInvalidLocalTime),
    }
}

pub fn parse_today_time(input: &str, now_utc: DateTime<Utc>) -> Result<i64, ParseTimeError> {
    let time =
        NaiveTime::parse_from_str(input, "%H:%M").map_err(|_| ParseTimeError::InvalidTime)?;
    let timezone = display_timezone();
    let date = now_utc.with_timezone(&timezone).date_naive();
    local_to_timestamp(date.and_time(time))
}

pub fn parse_most_recent_time(input: &str, now_utc: DateTime<Utc>) -> Result<i64, ParseTimeError> {
    let time =
        NaiveTime::parse_from_str(input, "%H:%M").map_err(|_| ParseTimeError::InvalidTime)?;
    let timezone = display_timezone();
    let local_now = now_utc.with_timezone(&timezone);
    let mut date = local_now.date_naive();
    let today = local_to_timestamp(date.and_time(time))?;
    if today <= now_utc.timestamp() {
        return Ok(today);
    }
    date = date
        .pred_opt()
        .ok_or(ParseTimeError::AmbiguousOrInvalidLocalTime)?;
    local_to_timestamp(date.and_time(time))
}

pub fn parse_full_datetime(input: &str) -> Result<i64, ParseTimeError> {
    let value = NaiveDateTime::parse_from_str(input, "%Y-%m-%d %H:%M")
        .map_err(|_| ParseTimeError::InvalidDateTime)?;
    local_to_timestamp(value)
}

pub fn format_datetime(timestamp: i64) -> String {
    let timezone = display_timezone();
    DateTime::<Utc>::from_timestamp(timestamp, 0)
        .map(|dt| {
            dt.with_timezone(&timezone)
                .format("%Y年%-m月%-d日 %H:%M")
                .to_string()
        })
        .unwrap_or_else(|| "不正な時刻".into())
}

/// Formats a persisted interval without panicking on an invalid timestamp.
pub fn format_history_range(started_at: i64, ended_at: Option<i64>) -> String {
    let timezone = display_timezone();
    let Some(start) =
        DateTime::<Utc>::from_timestamp(started_at, 0).map(|value| value.with_timezone(&timezone))
    else {
        return "不正な時刻".into();
    };
    match ended_at {
        None => format!("{} ～ 現在", start.format("%Y/%m/%d %H:%M")),
        Some(end) => {
            if end < started_at {
                return "不正な時刻".into();
            }
            let Some(end) =
                DateTime::<Utc>::from_timestamp(end, 0).map(|value| value.with_timezone(&timezone))
            else {
                return "不正な時刻".into();
            };
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

/// Formats a duration as Japanese hours and minutes, clamping negative input.
///
/// # Examples
///
/// ```
/// use discord_attendance_bot::time::format_duration;
///
/// assert_eq!(format_duration(3_720), "1時間02分");
/// assert_eq!(format_duration(-1), "0分");
/// ```
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

/// Returns the Unix timestamps of the local month start and following month start.
pub fn month_bounds(ym: YearMonth) -> anyhow::Result<(i64, i64)> {
    let start_date = NaiveDate::from_ymd_opt(ym.year, ym.month, 1)
        .ok_or_else(|| anyhow::anyhow!("invalid year-month"))?;
    let (next_year, next_month) = if ym.month == 12 {
        (
            ym.year
                .checked_add(1)
                .ok_or_else(|| anyhow::anyhow!("invalid next year"))?,
            1,
        )
    } else {
        (ym.year, ym.month + 1)
    };
    let end_date = NaiveDate::from_ymd_opt(next_year, next_month, 1)
        .ok_or_else(|| anyhow::anyhow!("invalid next month"))?;
    Ok((
        local_to_timestamp(
            start_date
                .and_hms_opt(0, 0, 0)
                .ok_or_else(|| anyhow::anyhow!("invalid month start"))?,
        )?,
        local_to_timestamp(
            end_date
                .and_hms_opt(0, 0, 0)
                .ok_or_else(|| anyhow::anyhow!("invalid month end"))?,
        )?,
    ))
}

/// Computes the next midnight in the display timezone.
pub fn next_midnight_timestamp(now: DateTime<Utc>) -> anyhow::Result<i64> {
    let timezone = display_timezone();
    let local_date = now.with_timezone(&timezone).date_naive();
    let next_date = local_date
        .succ_opt()
        .ok_or_else(|| anyhow::anyhow!("could not calculate next local date"))?;
    let next_midnight = next_date
        .and_hms_opt(0, 0, 0)
        .ok_or_else(|| anyhow::anyhow!("invalid next midnight"))?;
    local_to_timestamp(next_midnight).map_err(Into::into)
}

/// Computes the configured 21:00 automatic end for an older active period.
///
/// # Examples
///
/// ```
/// use discord_attendance_bot::time::{auto_end_timestamp, DISPLAY_TIMEZONE};
///
/// use chrono::{TimeZone, Utc};
///
/// let started = DISPLAY_TIMEZONE
///     .with_ymd_and_hms(2026, 8, 8, 18, 0, 0)
///     .single()
///     .expect("fixed test date")
///     .timestamp();
/// let after_midnight = Utc
///     .with_ymd_and_hms(2026, 8, 8, 15, 0, 1)
///     .single()
///     .expect("fixed test date");
/// assert_eq!(auto_end_timestamp(started, after_midnight), Some(started + 3 * 3600));
/// ```
pub fn auto_end_timestamp(started_at: i64, now: DateTime<Utc>) -> Option<i64> {
    auto_end_timestamp_with_policy(started_at, now, time_policy())
}

fn auto_end_timestamp_with_policy(
    started_at: i64,
    now: DateTime<Utc>,
    policy: TimePolicy,
) -> Option<i64> {
    let cutoff_time = policy.auto_end_time?;
    let started = DateTime::<Utc>::from_timestamp(started_at, 0)?.with_timezone(&policy.timezone);
    let local_now = now.with_timezone(&policy.timezone);
    if started.date_naive() >= local_now.date_naive() {
        return None;
    }
    let cutoff_date = if started.time() < cutoff_time {
        started.date_naive()
    } else {
        started.date_naive().succ_opt()?
    };
    let cutoff = local_to_timestamp_in(policy.timezone, cutoff_date.and_time(cutoff_time)).ok()?;
    (cutoff <= now.timestamp()).then_some(cutoff)
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

    #[test]
    fn parses_future_clock_time_as_previous_day_for_end() {
        let now = Tokyo
            .with_ymd_and_hms(2026, 8, 9, 0, 5, 0)
            .unwrap()
            .with_timezone(&Utc);
        let ts = parse_most_recent_time("20:45", now).unwrap();
        assert_eq!(format_datetime(ts), "2026年8月8日 20:45");
    }

    #[test]
    fn calculates_previous_day_auto_end_at_21() {
        let started = Tokyo
            .with_ymd_and_hms(2026, 8, 8, 18, 0, 0)
            .unwrap()
            .with_timezone(&Utc);
        let after_midnight = Tokyo
            .with_ymd_and_hms(2026, 8, 9, 0, 0, 1)
            .unwrap()
            .with_timezone(&Utc);
        let expected = Tokyo
            .with_ymd_and_hms(2026, 8, 8, 21, 0, 0)
            .unwrap()
            .timestamp();
        assert_eq!(
            auto_end_timestamp(started.timestamp(), after_midnight),
            Some(expected)
        );
        assert_eq!(auto_end_timestamp(started.timestamp(), started), None);
    }

    #[test]
    fn start_after_21_auto_ends_at_next_21() {
        let started = Tokyo
            .with_ymd_and_hms(2026, 8, 8, 21, 1, 0)
            .unwrap()
            .with_timezone(&Utc);
        let before_cutoff = Tokyo
            .with_ymd_and_hms(2026, 8, 9, 0, 0, 1)
            .unwrap()
            .with_timezone(&Utc);
        let after_cutoff = Tokyo
            .with_ymd_and_hms(2026, 8, 10, 0, 0, 1)
            .unwrap()
            .with_timezone(&Utc);

        assert_eq!(auto_end_timestamp(started.timestamp(), before_cutoff), None);
        assert_eq!(
            auto_end_timestamp(started.timestamp(), after_cutoff),
            Some(
                Tokyo
                    .with_ymd_and_hms(2026, 8, 9, 21, 0, 0)
                    .unwrap()
                    .timestamp()
            )
        );
    }

    #[test]
    fn start_exactly_at_21_auto_ends_at_next_21() {
        let started = Tokyo
            .with_ymd_and_hms(2026, 8, 8, 21, 0, 0)
            .unwrap()
            .with_timezone(&Utc);
        let after_midnight = Tokyo
            .with_ymd_and_hms(2026, 8, 9, 0, 0, 1)
            .unwrap()
            .with_timezone(&Utc);

        assert_eq!(
            auto_end_timestamp(started.timestamp(), after_midnight),
            None
        );
        let after_next_cutoff = Tokyo
            .with_ymd_and_hms(2026, 8, 10, 0, 0, 1)
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(
            auto_end_timestamp(started.timestamp(), after_next_cutoff),
            Some(
                Tokyo
                    .with_ymd_and_hms(2026, 8, 9, 21, 0, 0)
                    .unwrap()
                    .timestamp()
            )
        );
    }

    #[test]
    fn applies_custom_timezone_and_cutoff_without_global_state() {
        let policy = TimePolicy {
            timezone: chrono_tz::UTC,
            auto_end_time: NaiveTime::from_hms_opt(17, 30, 0),
        };
        let started = Utc.with_ymd_and_hms(2026, 8, 8, 10, 0, 0).unwrap();
        let next_day = Utc.with_ymd_and_hms(2026, 8, 9, 0, 0, 1).unwrap();
        assert_eq!(
            auto_end_timestamp_with_policy(started.timestamp(), next_day, policy),
            Some(
                Utc.with_ymd_and_hms(2026, 8, 8, 17, 30, 0)
                    .unwrap()
                    .timestamp()
            )
        );
        assert_eq!(
            auto_end_timestamp_with_policy(
                started.timestamp(),
                next_day,
                TimePolicy {
                    auto_end_time: None,
                    ..policy
                }
            ),
            None
        );
    }
}
