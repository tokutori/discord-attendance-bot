use crate::{ReadDatabase, sql};
use attendance_shared::{attendance::AttendanceSession, records::*};
pub async fn open_session(
    database: &ReadDatabase,
    guild_id: i64,
    user_id: i64,
) -> Result<Option<AttendanceSession>, sqlx::Error> {
    sql::open_session(&database.pool, guild_id, user_id).await
}

pub async fn active_sessions(
    database: &ReadDatabase,
    guild_id: i64,
) -> Result<Vec<AttendanceSession>, sqlx::Error> {
    sql::active_sessions(&database.pool, guild_id).await
}

pub async fn active_attendance_members(
    database: &ReadDatabase,
    guild_id: i64,
) -> Result<Vec<ActiveAttendanceMember>, sqlx::Error> {
    sql::active_attendance_members(&database.pool, guild_id).await
}

pub async fn latest_completed(
    database: &ReadDatabase,
    guild_id: i64,
    user_id: i64,
) -> Result<Option<AttendanceSession>, sqlx::Error> {
    sql::latest_completed(&database.pool, guild_id, user_id).await
}

pub async fn history(
    database: &ReadDatabase,
    guild_id: i64,
    user_id: i64,
    limit: i64,
) -> Result<Vec<AttendanceSession>, sqlx::Error> {
    sql::history(&database.pool, guild_id, user_id, limit).await
}

pub async fn get_owned(
    database: &ReadDatabase,
    id: i64,
    guild_id: i64,
    user_id: i64,
) -> Result<Option<AttendanceSession>, sqlx::Error> {
    sql::get_owned(&database.pool, id, guild_id, user_id).await
}

pub async fn overlapping_completed(
    database: &ReadDatabase,
    guild_id: i64,
    user_id: i64,
    range_start: i64,
    range_end: i64,
) -> Result<Vec<AttendanceSession>, sqlx::Error> {
    sql::overlapping_completed(&database.pool, guild_id, user_id, range_start, range_end).await
}

pub async fn overlapping_for_export(
    database: &ReadDatabase,
    guild_id: i64,
    range_start: i64,
    range_end: i64,
) -> Result<Vec<AttendanceSession>, sqlx::Error> {
    sql::overlapping_for_export(&database.pool, guild_id, range_start, range_end).await
}

pub async fn get_user_profile(
    database: &ReadDatabase,
    guild_id: i64,
    user_id: i64,
) -> Result<Option<UserProfile>, sqlx::Error> {
    sql::get_user_profile(&database.pool, guild_id, user_id).await
}

pub async fn user_profiles_for_export(
    database: &ReadDatabase,
    guild_id: i64,
) -> Result<Vec<UserProfile>, sqlx::Error> {
    sql::user_profiles_for_export(&database.pool, guild_id).await
}

pub async fn peek_auto_end_notice(
    database: &ReadDatabase,
    guild_id: i64,
    user_id: i64,
) -> Result<Option<AutoEndNotice>, sqlx::Error> {
    sql::peek_auto_end_notice(&database.pool, guild_id, user_id).await
}
