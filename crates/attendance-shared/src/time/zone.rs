//! Chrono interoperability backed exclusively by the bundled Jiff TZDB.
use std::{fmt, str::FromStr, sync::LazyLock};

use chrono::{
    Datelike, FixedOffset, LocalResult, NaiveDate, NaiveDateTime, Offset, TimeZone, Timelike,
};
use jiff::tz::{AmbiguousOffset, TimeZoneDatabase};

static DATABASE: LazyLock<TimeZoneDatabase> = LazyLock::new(TimeZoneDatabase::bundled);

#[derive(Clone, Debug)]
pub struct Tz(pub(super) jiff::tz::TimeZone);

impl FromStr for Tz {
    type Err = jiff::Error;
    fn from_str(name: &str) -> Result<Self, Self::Err> {
        DATABASE.get(name).map(Self)
    }
}

impl fmt::Display for Tz {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0.iana_name().unwrap_or("UTC"))
    }
}

#[derive(Clone, Debug)]
pub struct ZoneOffset {
    zone: Tz,
    fixed: FixedOffset,
}

impl Offset for ZoneOffset {
    fn fix(&self) -> FixedOffset {
        self.fixed
    }
}

impl fmt::Display for ZoneOffset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.fixed.fmt(f)
    }
}

impl Tz {
    fn offset(&self, offset: jiff::tz::Offset) -> ZoneOffset {
        ZoneOffset {
            zone: self.clone(),
            // All named IANA offsets fit Chrono's range (less than 24 hours).
            fixed: FixedOffset::east_opt(offset.seconds()).expect("IANA UTC offset"),
        }
    }

    /// Checked conversion at application boundaries. Chrono has a wider range
    /// than Jiff; callers must validate before using Chrono's infallible adapter.
    pub fn datetime(&self, timestamp: i64) -> anyhow::Result<chrono::DateTime<Self>> {
        jiff::Timestamp::from_second(timestamp)?;
        let utc = chrono::DateTime::from_timestamp(timestamp, 0)
            .ok_or_else(|| anyhow::anyhow!("invalid timestamp"))?;
        Ok(utc.with_timezone(self))
    }
}

impl TimeZone for Tz {
    type Offset = ZoneOffset;
    fn from_offset(offset: &Self::Offset) -> Self {
        offset.zone.clone()
    }
    fn offset_from_local_date(&self, date: &NaiveDate) -> LocalResult<Self::Offset> {
        self.offset_from_local_datetime(&date.and_hms_opt(0, 0, 0).expect("midnight"))
    }
    fn offset_from_local_datetime(&self, date: &NaiveDateTime) -> LocalResult<Self::Offset> {
        let Ok(year) = i16::try_from(date.year()) else {
            return LocalResult::None;
        };
        let Ok(civil) = jiff::civil::DateTime::new(
            year,
            date.month() as i8,
            date.day() as i8,
            date.hour() as i8,
            date.minute() as i8,
            date.second() as i8,
            date.nanosecond() as i32,
        ) else {
            return LocalResult::None;
        };
        let ambiguous = self.0.to_ambiguous_timestamp(civil);
        // Reject candidates outside Jiff's timestamp range as well as gaps.
        match ambiguous.offset() {
            AmbiguousOffset::Unambiguous { offset } if ambiguous.unambiguous().is_ok() => {
                LocalResult::Single(self.offset(offset))
            }
            AmbiguousOffset::Fold { before, after }
                if ambiguous.earlier().is_ok() && ambiguous.later().is_ok() =>
            {
                LocalResult::Ambiguous(self.offset(before), self.offset(after))
            }
            _ => LocalResult::None,
        }
    }
    fn offset_from_utc_date(&self, date: &NaiveDate) -> Self::Offset {
        self.offset_from_utc_datetime(&date.and_hms_opt(0, 0, 0).expect("midnight"))
    }
    fn offset_from_utc_datetime(&self, date: &NaiveDateTime) -> Self::Offset {
        let timestamp = jiff::Timestamp::from_second(date.and_utc().timestamp())
            .expect("validate timestamp with Tz::datetime before Chrono conversion");
        self.offset(self.0.to_offset(timestamp))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};

    #[test]
    fn adapter_preserves_fold_gap_subseconds_and_zone_identity() {
        let zone: Tz = "America/New_York".parse().unwrap();
        let gap = NaiveDate::from_ymd_opt(2026, 3, 8)
            .unwrap()
            .and_hms_opt(2, 30, 0)
            .unwrap();
        assert!(zone.from_local_datetime(&gap).single().is_none());
        let fold = NaiveDate::from_ymd_opt(2026, 11, 1)
            .unwrap()
            .and_hms_nano_opt(1, 30, 0, 123)
            .unwrap();
        let LocalResult::Ambiguous(first, last) = zone.from_local_datetime(&fold) else {
            panic!("fold")
        };
        assert_eq!(last.timestamp() - first.timestamp(), 3600);
        assert_eq!(first.timestamp_subsec_nanos(), 123);
        assert_eq!(first.naive_local(), fold);
        assert_eq!(last.naive_local(), fold);
        assert_eq!(first.timezone().to_string(), zone.to_string());
        assert_eq!(first.offset().fix().local_minus_utc(), -4 * 3600);
        assert_eq!(last.offset().fix().local_minus_utc(), -5 * 3600);
    }

    #[test]
    fn application_conversions_reject_the_wider_chrono_range() {
        let outside = DateTime::<Utc>::MAX_UTC.timestamp();
        assert!("UTC".parse::<Tz>().unwrap().datetime(outside).is_err());
        assert_eq!(super::super::format_datetime(outside), "不正な時刻");
        assert_eq!(
            super::super::format_history_range(outside, None),
            "不正な時刻"
        );
        assert!(super::super::auto_end_timestamp(outside, Utc::now()).is_err());
    }
}
