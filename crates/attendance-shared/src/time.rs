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
/// use attendance_shared::time::format_duration;
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

/// Resolves automatic calendar boundaries: earliest occurrence for overlaps,
/// first valid local second after a gap. Manual input remains strict.
fn resolve_scheduled_local(timezone: Tz, value: NaiveDateTime) -> anyhow::Result<i64> {
    if let Some(resolved) = timezone.from_local_datetime(&value).earliest() {
        return Ok(resolved.timestamp());
    }
    // IANA date-line changes can skip a whole date. Bound the search so an
    // unresolvable date produces an error instead of blocking the scheduler.
    for minute in 1..=48 * 60 {
        let candidate = value
            .checked_add_signed(chrono::Duration::minutes(minute))
            .ok_or_else(|| anyhow::anyhow!("calendar boundary overflow"))?;
        if timezone
            .from_local_datetime(&candidate)
            .earliest()
            .is_some()
        {
            let mut low = (minute - 1) * 60;
            let mut high = minute * 60;
            while high - low > 1 {
                let middle = (low + high) / 2;
                let probe = value
                    .checked_add_signed(chrono::Duration::seconds(middle))
                    .ok_or_else(|| anyhow::anyhow!("calendar boundary overflow"))?;
                if timezone.from_local_datetime(&probe).earliest().is_some() {
                    high = middle;
                } else {
                    low = middle;
                }
            }
            let boundary = value
                .checked_add_signed(chrono::Duration::seconds(high))
                .ok_or_else(|| anyhow::anyhow!("calendar boundary overflow"))?;
            return timezone
                .from_local_datetime(&boundary)
                .earliest()
                .map(|resolved| resolved.timestamp())
                .ok_or_else(|| anyhow::anyhow!("unresolvable calendar boundary"));
        }
    }
    anyhow::bail!("no valid local time within 48 hours of {value} in {timezone}")
}

/// Start of a local date, using the automatic calendar-boundary policy.
pub fn day_boundary_in(timezone: Tz, date: NaiveDate) -> anyhow::Result<i64> {
    resolve_scheduled_local(
        timezone,
        date.and_hms_opt(0, 0, 0)
            .ok_or_else(|| anyhow::anyhow!("invalid midnight"))?,
    )
}

/// Returns the Unix timestamps of the local month start and following month start.
pub fn month_bounds(ym: YearMonth) -> anyhow::Result<(i64, i64)> {
    month_bounds_in(ym, display_timezone())
}

/// Month boundaries in an explicit timezone, including offset transitions.
pub fn month_bounds_in(ym: YearMonth, timezone: Tz) -> anyhow::Result<(i64, i64)> {
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
        day_boundary_in(timezone, start_date)?,
        day_boundary_in(timezone, end_date)?,
    ))
}

/// Computes the next midnight in the display timezone.
pub fn next_midnight_timestamp(now: DateTime<Utc>) -> anyhow::Result<i64> {
    let timezone = display_timezone();
    let local_date = now.with_timezone(&timezone).date_naive();
    let next_date = local_date
        .succ_opt()
        .ok_or_else(|| anyhow::anyhow!("could not calculate next local date"))?;
    day_boundary_in(timezone, next_date)
}

/// Computes the configured 21:00 automatic end for an older active period.
///
/// # Examples
///
/// ```
/// use attendance_shared::time::{auto_end_timestamp, DISPLAY_TIMEZONE};
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
/// assert_eq!(auto_end_timestamp(started, after_midnight).unwrap(), Some(started + 3 * 3600));
/// ```
pub fn auto_end_timestamp(started_at: i64, now: DateTime<Utc>) -> anyhow::Result<Option<i64>> {
    auto_end_timestamp_with_policy(started_at, now, time_policy())
}

fn auto_end_timestamp_with_policy(
    started_at: i64,
    now: DateTime<Utc>,
    policy: TimePolicy,
) -> anyhow::Result<Option<i64>> {
    let Some(cutoff_time) = policy.auto_end_time else {
        return Ok(None);
    };
    let started = DateTime::<Utc>::from_timestamp(started_at, 0)
        .ok_or_else(|| anyhow::anyhow!("invalid auto-end start timestamp"))?
        .with_timezone(&policy.timezone);
    let local_now = now.with_timezone(&policy.timezone);
    if started.date_naive() >= local_now.date_naive() {
        return Ok(None);
    }
    let cutoff_date = if started.time() < cutoff_time {
        started.date_naive()
    } else {
        started
            .date_naive()
            .succ_opt()
            .ok_or_else(|| anyhow::anyhow!("invalid auto-end date"))?
    };
    let mut cutoff = resolve_scheduled_local(policy.timezone, cutoff_date.and_time(cutoff_time))?;
    // During a repeated hour the earliest cutoff may already precede the
    // second occurrence of the start time. Never end before the active period.
    if cutoff <= started_at {
        let next_date = cutoff_date
            .succ_opt()
            .ok_or_else(|| anyhow::anyhow!("invalid next auto-end date"))?;
        cutoff = resolve_scheduled_local(policy.timezone, next_date.and_time(cutoff_time))?;
    }
    anyhow::ensure!(cutoff > started_at, "auto-end cutoff did not advance");
    Ok((cutoff <= now.timestamp()).then_some(cutoff))
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
            auto_end_timestamp(started.timestamp(), after_midnight).unwrap(),
            Some(expected)
        );
        assert_eq!(
            auto_end_timestamp(started.timestamp(), started).unwrap(),
            None
        );
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

        assert_eq!(
            auto_end_timestamp(started.timestamp(), before_cutoff).unwrap(),
            None
        );
        assert_eq!(
            auto_end_timestamp(started.timestamp(), after_cutoff).unwrap(),
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
            auto_end_timestamp(started.timestamp(), after_midnight).unwrap(),
            None
        );
        let after_next_cutoff = Tokyo
            .with_ymd_and_hms(2026, 8, 10, 0, 0, 1)
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(
            auto_end_timestamp(started.timestamp(), after_next_cutoff).unwrap(),
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
            auto_end_timestamp_with_policy(started.timestamp(), next_day, policy).unwrap(),
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
            )
            .unwrap(),
            None
        );
    }
    #[test]
    fn automatic_cutoffs_resolve_gaps_and_overlaps_without_hiding_errors() {
        let timezone = chrono_tz::America::New_York;
        for (month, day, hour, minute, expected_hour) in [(3, 7, 2, 30, 3), (10, 31, 1, 30, 1)] {
            let started = timezone
                .with_ymd_and_hms(2026, month, day, 23, 0, 0)
                .unwrap();
            let date = started.date_naive().succ_opt().unwrap();
            let expected = timezone
                .from_local_datetime(
                    &date
                        .and_hms_opt(expected_hour, if month == 3 { 0 } else { minute }, 0)
                        .unwrap(),
                )
                .earliest()
                .unwrap()
                .timestamp();
            let policy = TimePolicy {
                timezone,
                auto_end_time: NaiveTime::from_hms_opt(hour, minute, 0),
            };
            for days in [2, 3, 30] {
                let now = started.with_timezone(&Utc) + chrono::Duration::days(days);
                assert_eq!(
                    auto_end_timestamp_with_policy(started.timestamp(), now, policy).unwrap(),
                    Some(expected)
                );
            }
            assert!(
                local_to_timestamp_in(timezone, date.and_hms_opt(hour, minute, 0).unwrap())
                    .is_err()
            );
        }
        assert!(auto_end_timestamp(i64::MAX, Utc::now()).is_err());
    }

    #[test]
    fn calendar_boundaries_handle_short_long_and_missing_dates() {
        let havana = chrono_tz::America::Havana;
        for (month, day, hours) in [(3, 8, 23), (11, 1, 25)] {
            let date = NaiveDate::from_ymd_opt(2026, month, day).unwrap();
            let start = day_boundary_in(havana, date).unwrap();
            let end = day_boundary_in(havana, date.succ_opt().unwrap()).unwrap();
            assert_eq!(end - start, hours * 3600);
            if day == 1 {
                assert_eq!(
                    month_bounds_in(YearMonth { year: 2026, month }, havana)
                        .unwrap()
                        .0,
                    start
                );
            }
        }
        let skipped = NaiveDate::from_ymd_opt(2011, 12, 30).unwrap();
        assert_eq!(
            day_boundary_in(chrono_tz::Pacific::Apia, skipped).unwrap(),
            day_boundary_in(chrono_tz::Pacific::Apia, skipped.succ_opt().unwrap()).unwrap()
        );
    }
    #[test]
    fn repeated_hour_start_uses_the_next_future_cutoff() {
        let timezone = chrono_tz::America::New_York;
        let started = timezone
            .with_ymd_and_hms(2026, 11, 1, 1, 15, 0)
            .latest()
            .unwrap();
        let policy = TimePolicy {
            timezone,
            auto_end_time: NaiveTime::from_hms_opt(1, 30, 0),
        };
        let expected = timezone.with_ymd_and_hms(2026, 11, 2, 1, 30, 0).unwrap();
        assert_eq!(
            auto_end_timestamp_with_policy(
                started.timestamp(),
                expected.with_timezone(&Utc),
                policy
            )
            .unwrap(),
            Some(expected.timestamp())
        );
        let before = expected.with_timezone(&Utc) - chrono::Duration::seconds(1);
        assert_eq!(
            auto_end_timestamp_with_policy(started.timestamp(), before, policy).unwrap(),
            None
        );
    }
}
