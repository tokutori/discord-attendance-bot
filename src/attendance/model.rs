use chrono::NaiveDate;
use sqlx::FromRow;

#[derive(Debug, Clone, FromRow)]
pub struct AttendanceSession {
    pub id: i64,
    pub guild_id: i64,
    pub user_id: i64,
    pub display_name: String,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub note: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub deleted_at: Option<i64>,
}

impl AttendanceSession {
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
    pub daily_totals: Vec<DailyAttendance>,
}
