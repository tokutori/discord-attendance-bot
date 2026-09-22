use chrono::NaiveDate;
use sqlx::FromRow;
use thiserror::Error;

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
/// A database row whose nullable state fields do not match the requested operation.
pub enum SessionStateError {
    #[error("活動記録が終了済み状態ではない")]
    NotCompleted,
    #[error("活動記録が活動中状態ではない")]
    NotActive,
}

/// A persisted attendance period.
///
/// The database invariant is that `ended_at` and `open_since` are mutually
/// exclusive: completed rows have only `ended_at`, while active rows have only
/// `open_since`. Use the checked accessors when a caller requires one state.
#[derive(Debug, Clone, FromRow)]
pub struct AttendanceSession {
    pub id: i64,
    pub guild_id: i64,
    pub user_id: i64,
    pub display_name: String,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub open_since: Option<i64>,
    pub note: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub deleted_at: Option<i64>,
}

impl AttendanceSession {
    /// Returns the end timestamp, rejecting an active row instead of guessing.
    pub fn completed_end(&self) -> Result<i64, SessionStateError> {
        self.ended_at.ok_or(SessionStateError::NotCompleted)
    }

    /// Returns the timestamp from which the current active period began.
    pub fn active_since(&self) -> Result<i64, SessionStateError> {
        self.open_since.ok_or(SessionStateError::NotActive)
    }

    pub fn duration_seconds_at(&self, now: i64) -> i64 {
        self.ended_at
            .unwrap_or(now)
            .saturating_sub(self.started_at)
            .max(0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct YearMonth {
    pub year: i32,
    pub month: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DailyAttendance {
    pub date: NaiveDate,
    pub total_seconds: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonthlyAttendance {
    pub year_month: YearMonth,
    pub total_seconds: i64,
    pub session_count: usize,
    pub elapsed_calendar_days: u32,
    pub daily_totals: Vec<DailyAttendance>,
}

impl MonthlyAttendance {
    pub fn average_per_session(&self) -> i64 {
        if self.session_count == 0 {
            0
        } else {
            self.total_seconds / self.session_count as i64
        }
    }

    pub fn average_per_day(&self) -> i64 {
        if self.elapsed_calendar_days == 0 {
            0
        } else {
            self.total_seconds / i64::from(self.elapsed_calendar_days)
        }
    }

    pub fn average_per_week(&self) -> i64 {
        if self.elapsed_calendar_days == 0 {
            0
        } else {
            self.total_seconds.saturating_mul(7) / i64::from(self.elapsed_calendar_days)
        }
    }
}
