use std::collections::BTreeMap;

use chrono::{DateTime, Datelike, Duration, NaiveDate, TimeZone, Utc};

use crate::{
    attendance::{AttendanceSession, DailyAttendance, MonthlyAttendance, YearMonth},
    time::{DISPLAY_TIMEZONE, month_bounds},
};

/// Returns the non-negative overlap between a session and a half-open range.
///
/// The calculation saturates at zero for disjoint or reversed ranges, so a
/// malformed persisted interval cannot create negative attendance totals.
///
/// # Examples
///
/// ```
/// use discord_attendance_bot::attendance::overlap_seconds;
///
/// assert_eq!(overlap_seconds(90, 120, 0, 100), 10);
/// assert_eq!(overlap_seconds(10, 20, 30, 40), 0);
/// ```
pub fn overlap_seconds(started_at: i64, ended_at: i64, range_start: i64, range_end: i64) -> i64 {
    ended_at
        .min(range_end)
        .saturating_sub(started_at.max(range_start))
        .max(0)
}

/// Aggregates completed sessions into calendar-day totals for one local month.
///
/// Active sessions are intentionally excluded; callers that need a live
/// preview must provide a completed snapshot first.
pub fn aggregate_monthly(
    sessions: &[AttendanceSession],
    year_month: YearMonth,
    now: DateTime<Utc>,
) -> anyhow::Result<MonthlyAttendance> {
    let (month_start, month_end) = month_bounds(year_month)?;
    let mut daily: BTreeMap<chrono::NaiveDate, i64> = BTreeMap::new();
    let mut total: i64 = 0;
    let mut count = 0;

    for session in sessions {
        let Some(ended_at) = session.ended_at else {
            continue;
        };
        let clipped_start = session.started_at.max(month_start);
        let clipped_end = ended_at.min(month_end);
        if clipped_end <= clipped_start {
            continue;
        }
        count += 1;
        total = total.saturating_add(clipped_end.saturating_sub(clipped_start));

        let mut cursor = clipped_start;
        while cursor < clipped_end {
            let cursor_dt = DateTime::<Utc>::from_timestamp(cursor, 0)
                .ok_or_else(|| anyhow::anyhow!("invalid session timestamp"))?
                .with_timezone(&DISPLAY_TIMEZONE);
            let date = cursor_dt.date_naive();
            let next_date = date
                .checked_add_signed(Duration::days(1))
                .ok_or_else(|| anyhow::anyhow!("invalid next calendar date"))?;
            let next_midnight_local = next_date
                .and_hms_opt(0, 0, 0)
                .ok_or_else(|| anyhow::anyhow!("invalid local midnight"))?;
            let next_midnight = DISPLAY_TIMEZONE
                .from_local_datetime(&next_midnight_local)
                .single()
                .ok_or_else(|| anyhow::anyhow!("invalid local midnight"))?
                .timestamp();
            let segment_end = clipped_end.min(next_midnight);
            let segment_seconds = segment_end.saturating_sub(cursor);
            let daily_total = daily.entry(date).or_default();
            *daily_total = daily_total.saturating_add(segment_seconds);
            cursor = segment_end;
        }
    }

    let first_date = NaiveDate::from_ymd_opt(year_month.year, year_month.month, 1)
        .ok_or_else(|| anyhow::anyhow!("invalid aggregation month"))?;
    let next_date = if year_month.month == 12 {
        let next_year = year_month
            .year
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("invalid next aggregation year"))?;
        NaiveDate::from_ymd_opt(next_year, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(year_month.year, year_month.month + 1, 1)
    }
    .ok_or_else(|| anyhow::anyhow!("invalid next aggregation month"))?;
    let local_today = now.with_timezone(&DISPLAY_TIMEZONE).date_naive();
    let elapsed_calendar_days = if local_today < first_date {
        0
    } else if local_today >= next_date {
        (next_date - first_date).num_days() as u32
    } else {
        local_today.day()
    };

    Ok(MonthlyAttendance {
        year_month,
        total_seconds: total,
        session_count: count,
        elapsed_calendar_days,
        daily_totals: daily
            .into_iter()
            .map(|(date, total_seconds)| DailyAttendance {
                date,
                total_seconds,
            })
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(started_at: i64, ended_at: i64) -> AttendanceSession {
        AttendanceSession {
            id: 1,
            guild_id: 1,
            user_id: 1,
            display_name: "x".into(),
            started_at,
            ended_at: Some(ended_at),
            open_since: None,
            note: None,
            created_at: 0,
            updated_at: 0,
            deleted_at: None,
        }
    }

    #[test]
    fn overlap_clips_range() {
        assert_eq!(overlap_seconds(90, 120, 0, 100), 10);
        assert_eq!(overlap_seconds(10, 20, 30, 40), 0);
    }

    #[test]
    fn monthly_aggregation_splits_days() {
        let ym = YearMonth {
            year: 2026,
            month: 8,
        };
        let tz = DISPLAY_TIMEZONE;
        let start = tz
            .with_ymd_and_hms(2026, 8, 6, 22, 0, 0)
            .single()
            .unwrap()
            .timestamp();
        let end = tz
            .with_ymd_and_hms(2026, 8, 7, 2, 0, 0)
            .single()
            .unwrap()
            .timestamp();
        let now = tz
            .with_ymd_and_hms(2026, 8, 7, 12, 0, 0)
            .single()
            .unwrap()
            .with_timezone(&Utc);
        let result = aggregate_monthly(&[session(start, end)], ym, now).unwrap();
        assert_eq!(result.total_seconds, 4 * 3600);
        assert_eq!(result.elapsed_calendar_days, 7);
        assert_eq!(result.average_per_day(), result.total_seconds / 7);
        assert_eq!(result.average_per_week(), result.total_seconds);
        assert_eq!(result.daily_totals.len(), 2);
        assert_eq!(result.daily_totals[0].total_seconds, 2 * 3600);
        assert_eq!(result.daily_totals[1].total_seconds, 2 * 3600);
    }

    #[test]
    fn past_month_uses_all_calendar_days() {
        let result = aggregate_monthly(
            &[],
            YearMonth {
                year: 2024,
                month: 2,
            },
            DISPLAY_TIMEZONE
                .with_ymd_and_hms(2026, 8, 9, 12, 0, 0)
                .single()
                .unwrap()
                .with_timezone(&Utc),
        )
        .unwrap();

        assert_eq!(result.elapsed_calendar_days, 29);
        assert_eq!(result.average_per_day(), 0);
        assert_eq!(result.average_per_week(), 0);
    }

    #[test]
    fn future_month_has_no_elapsed_calendar_days() {
        let result = aggregate_monthly(
            &[],
            YearMonth {
                year: 2026,
                month: 9,
            },
            DISPLAY_TIMEZONE
                .with_ymd_and_hms(2026, 8, 9, 12, 0, 0)
                .single()
                .unwrap()
                .with_timezone(&Utc),
        )
        .unwrap();

        assert_eq!(result.elapsed_calendar_days, 0);
        assert_eq!(result.average_per_day(), 0);
        assert_eq!(result.average_per_week(), 0);
    }
}
