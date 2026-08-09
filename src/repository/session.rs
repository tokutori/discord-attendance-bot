use sqlx::SqlitePool;

use crate::attendance::AttendanceSession;

use super::{
    SnapshotRow,
    change::{ChangeInput, insert_change, snapshot},
};

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

pub async fn latest_completed(
    pool: &SqlitePool,
    guild_id: i64,
    user_id: i64,
) -> Result<Option<AttendanceSession>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM attendance_sessions WHERE guild_id = ? AND user_id = ? AND ended_at IS NOT NULL AND deleted_at IS NULL ORDER BY ended_at DESC LIMIT 1")
        .bind(guild_id).bind(user_id).fetch_optional(pool).await
}

pub async fn insert_session(
    pool: &SqlitePool,
    guild_id: i64,
    user_id: i64,
    display_name: &str,
    started_at: i64,
    note: Option<&str>,
    now: i64,
) -> Result<i64, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let result = sqlx::query("INSERT INTO attendance_sessions (guild_id,user_id,display_name,started_at,ended_at,note,created_at,updated_at) VALUES (?,?,?,?,NULL,?,?,?)")
        .bind(guild_id).bind(user_id).bind(display_name).bind(started_at).bind(note).bind(now).bind(now).execute(&mut *tx).await?;
    let id = result.last_insert_rowid();
    let after = SnapshotRow {
        started_at,
        ended_at: None,
        note: note.map(str::to_owned),
        deleted_at: None,
    };
    insert_change(
        &mut tx,
        ChangeInput {
            guild_id,
            user_id,
            session_id: id,
            kind: "start",
            before: None,
            after: &after,
            created_at: now,
        },
    )
    .await?;
    tx.commit().await?;
    Ok(id)
}

pub async fn close_session(
    pool: &SqlitePool,
    id: i64,
    ended_at: i64,
    note: Option<&str>,
    now: i64,
) -> Result<u64, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let Some(existing) = sqlx::query_as::<_, AttendanceSession>(
        "SELECT * FROM attendance_sessions WHERE id = ? AND ended_at IS NULL AND deleted_at IS NULL",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?
    else {
        return Ok(0);
    };
    let after = SnapshotRow {
        started_at: existing.started_at,
        ended_at: Some(ended_at),
        note: note.map(str::to_owned).or(existing.note.clone()),
        deleted_at: existing.deleted_at,
    };
    let result = sqlx::query("UPDATE attendance_sessions SET ended_at = ?, note = ?, updated_at = ? WHERE id = ? AND ended_at IS NULL AND deleted_at IS NULL")
        .bind(ended_at).bind(&after.note).bind(now).bind(id).execute(&mut *tx).await?;
    if result.rows_affected() == 1 {
        insert_change(
            &mut tx,
            ChangeInput {
                guild_id: existing.guild_id,
                user_id: existing.user_id,
                session_id: id,
                kind: "end",
                before: Some(&snapshot(&existing)),
                after: &after,
                created_at: now,
            },
        )
        .await?;
        tx.commit().await?;
    }
    Ok(result.rows_affected())
}

pub async fn reopen_session(pool: &SqlitePool, id: i64, now: i64) -> Result<u64, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let Some(existing) = sqlx::query_as::<_, AttendanceSession>(
        "SELECT * FROM attendance_sessions WHERE id = ? AND ended_at IS NOT NULL AND deleted_at IS NULL",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?
    else {
        return Ok(0);
    };
    let after = SnapshotRow {
        started_at: existing.started_at,
        ended_at: None,
        note: existing.note.clone(),
        deleted_at: existing.deleted_at,
    };
    let result = sqlx::query("UPDATE attendance_sessions SET ended_at = NULL, updated_at = ? WHERE id = ? AND ended_at IS NOT NULL AND deleted_at IS NULL")
        .bind(now).bind(id).execute(&mut *tx).await?;
    if result.rows_affected() == 1 {
        insert_change(
            &mut tx,
            ChangeInput {
                guild_id: existing.guild_id,
                user_id: existing.user_id,
                session_id: id,
                kind: "continue",
                before: Some(&snapshot(&existing)),
                after: &after,
                created_at: now,
            },
        )
        .await?;
        tx.commit().await?;
    }
    Ok(result.rows_affected())
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
