use attendance_shared::{attendance::AttendanceSession, records::*};
use sqlx::SqlitePool;

pub async fn open_session(
    pool: &SqlitePool,
    guild_id: i64,
    user_id: i64,
) -> Result<Option<AttendanceSession>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM attendance_sessions WHERE guild_id = ? AND user_id = ? AND ended_at IS NULL AND deleted_at IS NULL LIMIT 1")
        .bind(guild_id).bind(user_id).fetch_optional(pool).await
}

pub async fn active_sessions(
    pool: &SqlitePool,
    guild_id: i64,
) -> Result<Vec<AttendanceSession>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM attendance_sessions WHERE guild_id = ? AND ended_at IS NULL AND deleted_at IS NULL ORDER BY started_at ASC")
        .bind(guild_id)
        .fetch_all(pool)
        .await
}

pub async fn active_attendance_members(
    pool: &SqlitePool,
    guild_id: i64,
) -> Result<Vec<ActiveAttendanceMember>, sqlx::Error> {
    sqlx::query_as(
        "SELECT sessions.user_id, sessions.display_name,
                profiles.generation, profiles.real_name, profiles.role, profiles.name_reading
         FROM attendance_sessions sessions
         LEFT JOIN attendance_user_profiles profiles
           ON profiles.guild_id = sessions.guild_id AND profiles.user_id = sessions.user_id
         WHERE sessions.guild_id = ? AND sessions.ended_at IS NULL AND sessions.deleted_at IS NULL",
    )
    .bind(guild_id)
    .fetch_all(pool)
    .await
}

pub async fn latest_completed(
    pool: &SqlitePool,
    guild_id: i64,
    user_id: i64,
) -> Result<Option<AttendanceSession>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM attendance_sessions WHERE guild_id = ? AND user_id = ? AND ended_at IS NOT NULL AND deleted_at IS NULL ORDER BY ended_at DESC LIMIT 1")
        .bind(guild_id).bind(user_id).fetch_optional(pool).await
}

pub async fn history(
    pool: &SqlitePool,
    guild_id: i64,
    user_id: i64,
    limit: i64,
) -> Result<Vec<AttendanceSession>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM attendance_sessions WHERE guild_id = ? AND user_id = ? AND deleted_at IS NULL ORDER BY CASE WHEN ended_at IS NULL THEN 0 ELSE 1 END, started_at DESC LIMIT ?")
        .bind(guild_id).bind(user_id).bind(limit).fetch_all(pool).await
}

pub async fn get_owned(
    pool: &SqlitePool,
    id: i64,
    guild_id: i64,
    user_id: i64,
) -> Result<Option<AttendanceSession>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM attendance_sessions WHERE id = ? AND guild_id = ? AND user_id = ? AND deleted_at IS NULL")
        .bind(id).bind(guild_id).bind(user_id).fetch_optional(pool).await
}

pub async fn overlapping_completed(
    pool: &SqlitePool,
    guild_id: i64,
    user_id: i64,
    range_start: i64,
    range_end: i64,
) -> Result<Vec<AttendanceSession>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM attendance_sessions WHERE guild_id = ? AND user_id = ? AND deleted_at IS NULL AND ended_at IS NOT NULL AND started_at < ? AND ended_at > ? ORDER BY started_at ASC")
        .bind(guild_id).bind(user_id).bind(range_end).bind(range_start).fetch_all(pool).await
}

pub async fn overlapping_for_export(
    pool: &SqlitePool,
    guild_id: i64,
    range_start: i64,
    range_end: i64,
) -> Result<Vec<AttendanceSession>, sqlx::Error> {
    sqlx::query_as(
        "SELECT * FROM attendance_sessions
         WHERE guild_id = ? AND deleted_at IS NULL
           AND started_at < ? AND (ended_at IS NULL OR ended_at > ?)
         ORDER BY user_id ASC, started_at ASC",
    )
    .bind(guild_id)
    .bind(range_end)
    .bind(range_start)
    .fetch_all(pool)
    .await
}

pub async fn get_user_profile(
    pool: &SqlitePool,
    guild_id: i64,
    user_id: i64,
) -> Result<Option<UserProfile>, sqlx::Error> {
    sqlx::query_as(
        "SELECT guild_id, user_id, generation, real_name, role, name_reading, updated_at
         FROM attendance_user_profiles
         WHERE guild_id = ? AND user_id = ?",
    )
    .bind(guild_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await
}

pub async fn user_profiles_for_export(
    pool: &SqlitePool,
    guild_id: i64,
) -> Result<Vec<UserProfile>, sqlx::Error> {
    sqlx::query_as(
        "SELECT guild_id, user_id, generation, real_name, role, name_reading, updated_at
         FROM attendance_user_profiles
         WHERE guild_id = ?
         ORDER BY generation, real_name, user_id",
    )
    .bind(guild_id)
    .fetch_all(pool)
    .await
}

pub async fn peek_auto_end_notice(
    pool: &SqlitePool,
    guild_id: i64,
    user_id: i64,
) -> Result<Option<AutoEndNotice>, sqlx::Error> {
    sqlx::query_as::<_, AutoEndNoticeRow>(
        "SELECT id, session_id, automatic_ended_at, applied_at, corrected_at
         FROM attendance_auto_end_events
         WHERE guild_id = ? AND user_id = ? AND notified_at IS NULL
         ORDER BY applied_at ASC, id ASC LIMIT 1",
    )
    .bind(guild_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .map(|notice| notice.map(Into::into))
}

#[derive(Debug, sqlx::FromRow)]
struct AutoEndNoticeRow {
    id: i64,
    session_id: i64,
    automatic_ended_at: i64,
    applied_at: i64,
    corrected_at: Option<i64>,
}

impl From<AutoEndNoticeRow> for AutoEndNotice {
    fn from(value: AutoEndNoticeRow) -> Self {
        Self {
            event_id: value.id,
            session_id: value.session_id,
            automatic_ended_at: value.automatic_ended_at,
            applied_at: value.applied_at,
            corrected_at: value.corrected_at,
        }
    }
}
