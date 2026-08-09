use chrono::{DateTime, Utc};
use sqlx::{FromRow, SqlitePool};

use crate::{attendance::AttendanceSession, time};

use super::{
    AutoEndCorrection, AutoEndNotice, SnapshotRow,
    change::{ChangeInput, insert_change, snapshot},
};

pub async fn apply_due_auto_ends(
    pool: &SqlitePool,
    guild_id: i64,
    now: i64,
) -> Result<Vec<AutoEndNotice>, sqlx::Error> {
    let now_utc = DateTime::<Utc>::from_timestamp(now, 0).unwrap_or_else(Utc::now);
    let mut tx = pool.begin().await?;
    let sessions = sqlx::query_as::<_, AttendanceSession>(
        "SELECT * FROM attendance_sessions
         WHERE guild_id = ? AND ended_at IS NULL AND deleted_at IS NULL
         ORDER BY started_at ASC",
    )
    .bind(guild_id)
    .fetch_all(&mut *tx)
    .await?;
    let mut notices = Vec::new();
    for session in sessions {
        let Some(automatic_ended_at) = time::auto_end_timestamp(session.started_at, now_utc) else {
            continue;
        };
        let result = sqlx::query(
            "UPDATE attendance_sessions
             SET ended_at = ?, updated_at = ?
             WHERE id = ? AND guild_id = ? AND ended_at IS NULL AND deleted_at IS NULL",
        )
        .bind(automatic_ended_at)
        .bind(now)
        .bind(session.id)
        .bind(guild_id)
        .execute(&mut *tx)
        .await?;
        if result.rows_affected() != 1 {
            continue;
        }
        sqlx::query(
            "INSERT INTO attendance_auto_end_events (
                session_id, guild_id, user_id, automatic_ended_at, applied_at
             ) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(session.id)
        .bind(guild_id)
        .bind(session.user_id)
        .bind(automatic_ended_at)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        notices.push(AutoEndNotice {
            session_id: session.id,
            automatic_ended_at,
            applied_at: now,
            corrected_at: None,
        });
    }
    tx.commit().await?;
    Ok(notices)
}

pub async fn latest_auto_ended(
    pool: &SqlitePool,
    guild_id: i64,
    user_id: i64,
) -> Result<Option<AutoEndCorrection>, sqlx::Error> {
    let Some(session) = sqlx::query_as::<_, AttendanceSession>(
        "SELECT s.* FROM attendance_sessions s
         INNER JOIN attendance_auto_end_events a ON a.session_id = s.id
         WHERE s.guild_id = ? AND s.user_id = ? AND s.ended_at IS NOT NULL
           AND s.deleted_at IS NULL AND a.corrected_at IS NULL
         ORDER BY a.applied_at DESC LIMIT 1",
    )
    .bind(guild_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await?
    else {
        return Ok(None);
    };
    let automatic_ended_at = sqlx::query_scalar::<_, i64>(
        "SELECT automatic_ended_at FROM attendance_auto_end_events
         WHERE session_id = ? AND corrected_at IS NULL",
    )
    .bind(session.id)
    .fetch_one(pool)
    .await?;
    Ok(Some(AutoEndCorrection {
        session,
        automatic_ended_at,
    }))
}

pub async fn correct_auto_ended_session(
    pool: &SqlitePool,
    id: i64,
    guild_id: i64,
    user_id: i64,
    ended_at: i64,
    note: Option<&str>,
    now: i64,
) -> Result<Option<AutoEndCorrection>, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let Some(existing) = sqlx::query_as::<_, AttendanceSession>(
        "SELECT s.* FROM attendance_sessions s
         INNER JOIN attendance_auto_end_events a ON a.session_id = s.id
         WHERE s.id = ? AND s.guild_id = ? AND s.user_id = ?
           AND s.ended_at IS NOT NULL AND s.deleted_at IS NULL
           AND a.corrected_at IS NULL
         LIMIT 1",
    )
    .bind(id)
    .bind(guild_id)
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await?
    else {
        return Ok(None);
    };
    let automatic_ended_at = sqlx::query_scalar::<_, i64>(
        "SELECT automatic_ended_at FROM attendance_auto_end_events
         WHERE session_id = ? AND corrected_at IS NULL",
    )
    .bind(id)
    .fetch_one(&mut *tx)
    .await?;
    let after = SnapshotRow {
        started_at: existing.started_at,
        ended_at: Some(ended_at),
        note: note.map(str::to_owned).or(existing.note.clone()),
        deleted_at: existing.deleted_at,
    };
    let result = sqlx::query(
        "UPDATE attendance_sessions
         SET ended_at = ?, note = ?, updated_at = ?
         WHERE id = ? AND guild_id = ? AND user_id = ? AND deleted_at IS NULL",
    )
    .bind(ended_at)
    .bind(&after.note)
    .bind(now)
    .bind(id)
    .bind(guild_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    if result.rows_affected() != 1 {
        return Ok(None);
    }
    insert_change(
        &mut tx,
        ChangeInput {
            guild_id,
            user_id,
            session_id: id,
            kind: "end",
            before: Some(&snapshot(&existing)),
            after: &after,
            created_at: now,
        },
    )
    .await?;
    sqlx::query("UPDATE attendance_auto_end_events SET corrected_at = ? WHERE session_id = ?")
        .bind(now)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Some(AutoEndCorrection {
        session: AttendanceSession {
            ended_at: Some(ended_at),
            note: after.note,
            updated_at: now,
            ..existing
        },
        automatic_ended_at,
    }))
}

pub async fn take_auto_end_notice(
    pool: &SqlitePool,
    guild_id: i64,
    user_id: i64,
    now: i64,
) -> Result<Option<AutoEndNotice>, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let notice = sqlx::query_as::<_, AutoEndNoticeRow>(
        "SELECT session_id, automatic_ended_at, applied_at, corrected_at
         FROM attendance_auto_end_events
         WHERE guild_id = ? AND user_id = ? AND notified_at IS NULL
         ORDER BY applied_at DESC LIMIT 1",
    )
    .bind(guild_id)
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(notice) = notice else {
        return Ok(None);
    };
    sqlx::query("UPDATE attendance_auto_end_events SET notified_at = ? WHERE session_id = ?")
        .bind(now)
        .bind(notice.session_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Some(AutoEndNotice {
        session_id: notice.session_id,
        automatic_ended_at: notice.automatic_ended_at,
        applied_at: notice.applied_at,
        corrected_at: notice.corrected_at,
    }))
}

#[derive(Debug, FromRow)]
struct AutoEndNoticeRow {
    session_id: i64,
    automatic_ended_at: i64,
    applied_at: i64,
    corrected_at: Option<i64>,
}
