use super::Tz;
use crate::attendance::YearMonth;
use chrono::{NaiveDate, TimeZone};

#[derive(Debug)]
struct DaySegment {
    date: NaiveDate,
    start: i64,
    end: i64,
}

/// Exact UTC intervals whose local dates belong to a month. A local date can
/// recur after another date: neither days nor months are necessarily one interval.
#[derive(Debug)]
pub struct MonthCalendar {
    segments: Vec<DaySegment>,
}

impl MonthCalendar {
    pub fn new(ym: YearMonth, timezone: Tz) -> anyhow::Result<Self> {
        let first = NaiveDate::from_ymd_opt(ym.year, ym.month, 1)
            .ok_or_else(|| anyhow::anyhow!("invalid month"))?;
        let next = first
            .checked_add_months(chrono::Months::new(1))
            .ok_or_else(|| anyhow::anyhow!("invalid next month"))?;
        let local_start = first
            .and_hms_opt(0, 0, 0)
            .expect("midnight")
            .and_utc()
            .timestamp();
        let local_end = next
            .and_hms_opt(0, 0, 0)
            .expect("midnight")
            .and_utc()
            .timestamp();
        // These are documented type bounds, not sampled/assumed IANA offsets.
        let envelope_start = local_start - i64::from(jiff::tz::Offset::MAX.seconds());
        let envelope_end = local_end - i64::from(jiff::tz::Offset::MIN.seconds());
        let beginning = jiff::Timestamp::from_second(envelope_start)?;
        jiff::Timestamp::from_second(envelope_end)?;
        let mut cursor = envelope_start;
        let mut offset = i64::from(timezone.0.to_offset(beginning).seconds());
        let mut segments = Vec::new();
        for transition in timezone.0.following(beginning) {
            let end = transition.timestamp().as_second().min(envelope_end);
            append_days(&mut segments, cursor, end, offset, local_start, local_end)?;
            cursor = end;
            if cursor == envelope_end {
                break;
            }
            offset = i64::from(transition.offset().seconds());
        }
        append_days(
            &mut segments,
            cursor,
            envelope_end,
            offset,
            local_start,
            local_end,
        )?;
        anyhow::ensure!(
            !segments.is_empty(),
            "month has no representable local dates"
        );
        Ok(Self { segments })
    }

    /// SQL candidate envelope. Callers must still intersect with `overlaps`:
    /// the envelope can contain intervals belonging to a neighbouring month.
    pub fn query_bounds(&self) -> (i64, i64) {
        (
            self.segments.first().expect("nonempty").start,
            self.segments.last().expect("nonempty").end,
        )
    }

    pub fn overlaps(&self, start: i64, end: i64) -> impl Iterator<Item = (NaiveDate, i64)> + '_ {
        self.segments.iter().filter_map(move |segment| {
            let seconds = end
                .min(segment.end)
                .saturating_sub(start.max(segment.start));
            (seconds > 0).then_some((segment.date, seconds))
        })
    }
}

fn append_days(
    segments: &mut Vec<DaySegment>,
    start: i64,
    end: i64,
    offset: i64,
    month_start: i64,
    month_end: i64,
) -> anyhow::Result<()> {
    let mut local = (start + offset).max(month_start);
    let local_end = (end + offset).min(month_end);
    while local < local_end {
        let date = chrono::DateTime::from_timestamp(local, 0)
            .ok_or_else(|| anyhow::anyhow!("invalid civil day"))?
            .date_naive();
        let next_midnight = (local.div_euclid(86400) + 1) * 86400;
        let segment_end = next_midnight.min(local_end);
        segments.push(DaySegment {
            date,
            start: local - offset,
            end: segment_end - offset,
        });
        local = segment_end;
    }
    Ok(())
}

pub(super) fn next_date_boundary(timezone: Tz, now: i64) -> anyhow::Result<i64> {
    let timestamp = jiff::Timestamp::from_second(now)?;
    let current = timezone.datetime(now)?.date_naive();
    let mut cursor = now;
    // At each transition or midnight inspect the actual resulting date.
    // Thus a repeated midnight cannot send the scheduler back into the past.
    for transition in timezone.0.following(timestamp) {
        let local = timezone.datetime(cursor)?;
        let offset = i64::from(chrono::Offset::fix(local.offset()).local_minus_utc());
        let midnight = ((cursor + offset).div_euclid(86400) + 1) * 86400 - offset;
        let candidate = midnight.min(transition.timestamp().as_second());
        if timezone.datetime(candidate)?.date_naive() != current {
            return Ok(candidate);
        }
        cursor = candidate;
    }
    let local = timezone.datetime(cursor)?;
    let date = local
        .date_naive()
        .succ_opt()
        .ok_or_else(|| anyhow::anyhow!("date overflow"))?;
    timezone
        .from_local_datetime(&date.and_hms_opt(0, 0, 0).expect("midnight"))
        .earliest()
        .map(|value| value.timestamp())
        .ok_or_else(|| anyhow::anyhow!("invalid midnight"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Datelike, Utc};

    fn utc(hour: u32, minute: u32) -> i64 {
        Utc.with_ymd_and_hms(2009, 11, 1, hour, minute, 0)
            .unwrap()
            .timestamp()
    }

    #[test]
    fn rollback_month_membership_matches_every_utc_second() {
        let zone: Tz = "America/Goose_Bay".parse().unwrap();
        let mut totals = Vec::new();
        for (month, expected) in [(10, 10_740), (11, 3_660)] {
            let calendar =
                MonthCalendar::new(YearMonth { year: 2009, month }, zone.clone()).unwrap();
            let actual = calendar
                .overlaps(utc(1, 0), utc(5, 0))
                .map(|(_, seconds)| seconds)
                .sum::<i64>();
            let reference = (utc(1, 0)..utc(5, 0))
                .filter(|second| zone.datetime(*second).unwrap().month() == month)
                .count() as i64;
            assert_eq!(actual, expected);
            assert_eq!(actual, reference);
            totals.push(actual);
            assert_eq!(
                calendar
                    .overlaps(utc(3, 10), utc(3, 20))
                    .map(|(_, s)| s)
                    .sum::<i64>(),
                if month == 10 { 600 } else { 0 }
            );
            assert_eq!(
                calendar
                    .overlaps(utc(3, 0), utc(3, 1))
                    .map(|(_, s)| s)
                    .sum::<i64>(),
                if month == 11 { 60 } else { 0 }
            );
        }
        assert_eq!(totals.iter().sum::<i64>(), utc(5, 0) - utc(1, 0));
    }

    #[test]
    fn scheduler_always_moves_to_the_next_actual_date_change() {
        let zone: Tz = "America/Goose_Bay".parse().unwrap();
        for (now, expected) in [
            (utc(2, 59), utc(3, 0)),
            (utc(3, 0), utc(3, 1)),
            (utc(3, 10), utc(4, 0)),
        ] {
            assert_eq!(next_date_boundary(zone.clone(), now).unwrap(), expected);
        }
        for zone in ["UTC", "Asia/Tokyo", "America/Havana", "Pacific/Apia"] {
            let zone: Tz = zone.parse().unwrap();
            let now = utc(3, 10);
            let next = next_date_boundary(zone.clone(), now).unwrap();
            assert!(next > now);
            assert_eq!(
                zone.datetime(next - 1).unwrap().date_naive(),
                zone.datetime(now).unwrap().date_naive()
            );
            assert_ne!(
                zone.datetime(next).unwrap().date_naive(),
                zone.datetime(now).unwrap().date_naive()
            );
        }
    }

    #[test]
    fn unsupported_calendar_ranges_return_errors() {
        for ym in [
            YearMonth {
                year: 9999,
                month: 12,
            },
            YearMonth {
                year: i32::MAX,
                month: 1,
            },
        ] {
            assert!(MonthCalendar::new(ym, "UTC".parse().unwrap()).is_err());
        }
        for ym in [
            YearMonth { year: 0, month: 1 },
            YearMonth {
                year: 9999,
                month: 11,
            },
        ] {
            assert!(MonthCalendar::new(ym, "UTC".parse().unwrap()).is_ok());
        }
    }
}
