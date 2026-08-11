use chrono::{DateTime, Utc};
use sqlx::{FromRow, Sqlite, SqlitePool, Transaction};

use crate::{attendance::AttendanceSession, time};

use super::{
    AutoEndCorrection, AutoEndNotice, SnapshotRow,
    change::{ChangeInput, insert_change, snapshot},
    transaction::begin_immediate,
};

pub(super) async fn mark_active_auto_end_corrected(
    tx: &mut Transaction<'_, Sqlite>,
    session_id: i64,
    now: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE attendance_auto_end_events
         SET corrected_at = ?
         WHERE session_id = ? AND corrected_at IS NULL",
    )
    .bind(now)
    .bind(session_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub(super) async fn restore_auto_end_for_snapshot(
    tx: &mut Transaction<'_, Sqlite>,
    session_id: i64,
    state: &SnapshotRow,
    now: i64,
) -> Result<(), sqlx::Error> {
    mark_active_auto_end_corrected(tx, session_id, now).await?;
    let Some(ended_at) = state.ended_at.filter(|_| state.deleted_at.is_none()) else {
        return Ok(());
    };
    sqlx::query(
        "UPDATE attendance_auto_end_events
         SET corrected_at = NULL
         WHERE id = (
             SELECT id FROM attendance_auto_end_events
             WHERE session_id = ? AND automatic_ended_at = ?
             ORDER BY applied_at DESC, id DESC LIMIT 1
         )",
    )
    .bind(session_id)
    .bind(ended_at)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub async fn apply_due_auto_ends(
    pool: &SqlitePool,
    guild_id: i64,
    now: i64,
) -> Result<Vec<AutoEndNotice>, sqlx::Error> {
    let now_utc = DateTime::<Utc>::from_timestamp(now, 0).unwrap_or_else(Utc::now);
    let mut tx = begin_immediate(pool).await?;
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
        let Some(automatic_ended_at) = session
            .open_since
            .and_then(|open_since| time::auto_end_timestamp(open_since, now_utc))
        else {
            continue;
        };
        let result = sqlx::query(
            "UPDATE attendance_sessions
             SET ended_at = ?, open_since = NULL, updated_at = ?
             WHERE id = ? AND guild_id = ? AND user_id = ?
               AND ended_at IS NULL AND deleted_at IS NULL",
        )
        .bind(automatic_ended_at)
        .bind(now)
        .bind(session.id)
        .bind(guild_id)
        .bind(session.user_id)
        .execute(&mut *tx)
        .await?;
        if result.rows_affected() != 1 {
            continue;
        }
        let change_id_at_application = sqlx::query_scalar::<_, i64>(
            "SELECT COALESCE(MAX(id), 0) FROM attendance_changes
             WHERE guild_id = ? AND user_id = ?",
        )
        .bind(guild_id)
        .bind(session.user_id)
        .fetch_one(&mut *tx)
        .await?;
        let event = sqlx::query(
            "INSERT INTO attendance_auto_end_events (
                session_id, guild_id, user_id, automatic_ended_at, applied_at,
                change_id_at_application
             ) VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(session.id)
        .bind(guild_id)
        .bind(session.user_id)
        .bind(automatic_ended_at)
        .bind(now)
        .bind(change_id_at_application)
        .execute(&mut *tx)
        .await?;
        notices.push(AutoEndNotice {
            event_id: event.last_insert_rowid(),
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
         WHERE s.guild_id = ? AND s.user_id = ? AND s.ended_at = a.automatic_ended_at
           AND s.deleted_at IS NULL AND a.corrected_at IS NULL
           AND NOT EXISTS (
               SELECT 1 FROM attendance_changes later
               WHERE later.guild_id = a.guild_id AND later.user_id = a.user_id
                 AND later.id > a.change_id_at_application
           )
         ORDER BY a.applied_at DESC, a.id DESC LIMIT 1",
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
         WHERE session_id = ? AND automatic_ended_at = ? AND corrected_at IS NULL
         ORDER BY applied_at DESC, id DESC LIMIT 1",
    )
    .bind(session.id)
    .bind(session.ended_at)
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
    let mut tx = begin_immediate(pool).await?;
    let Some(event) = sqlx::query_as::<_, AutoEndEventIdentity>(
        "SELECT a.id, a.automatic_ended_at FROM attendance_auto_end_events a
         WHERE a.session_id = ? AND a.guild_id = ? AND a.user_id = ?
           AND a.corrected_at IS NULL
           AND NOT EXISTS (
               SELECT 1 FROM attendance_changes later
               WHERE later.guild_id = a.guild_id AND later.user_id = a.user_id
                 AND later.id > a.change_id_at_application
           )
         ORDER BY a.applied_at DESC, a.id DESC LIMIT 1",
    )
    .bind(id)
    .bind(guild_id)
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await?
    else {
        return Ok(None);
    };
    let Some(existing) = sqlx::query_as::<_, AttendanceSession>(
        "SELECT * FROM attendance_sessions
         WHERE id = ? AND guild_id = ? AND user_id = ?
           AND ended_at = ? AND deleted_at IS NULL",
    )
    .bind(id)
    .bind(guild_id)
    .bind(user_id)
    .bind(event.automatic_ended_at)
    .fetch_optional(&mut *tx)
    .await?
    else {
        return Ok(None);
    };
    let after = SnapshotRow {
        started_at: existing.started_at,
        ended_at: Some(ended_at),
        open_since: None,
        note: note.map(str::to_owned).or(existing.note.clone()),
        deleted_at: existing.deleted_at,
    };
    let result = sqlx::query(
        "UPDATE attendance_sessions
         SET ended_at = ?, open_since = NULL, note = ?, updated_at = ?
         WHERE id = ? AND guild_id = ? AND user_id = ?
           AND ended_at = ? AND deleted_at IS NULL",
    )
    .bind(ended_at)
    .bind(&after.note)
    .bind(now)
    .bind(id)
    .bind(guild_id)
    .bind(user_id)
    .bind(event.automatic_ended_at)
    .execute(&mut *tx)
    .await?;
    if result.rows_affected() != 1 {
        return Ok(None);
    }
    let corrected = sqlx::query(
        "UPDATE attendance_auto_end_events SET corrected_at = ?
         WHERE id = ? AND corrected_at IS NULL",
    )
    .bind(now)
    .bind(event.id)
    .execute(&mut *tx)
    .await?;
    if corrected.rows_affected() != 1 {
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
    tx.commit().await?;
    Ok(Some(AutoEndCorrection {
        session: AttendanceSession {
            ended_at: Some(ended_at),
            note: after.note,
            updated_at: now,
            ..existing
        },
        automatic_ended_at: event.automatic_ended_at,
    }))
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

pub async fn acknowledge_auto_end_notice(
    pool: &SqlitePool,
    event_id: i64,
    guild_id: i64,
    user_id: i64,
    now: i64,
) -> Result<bool, sqlx::Error> {
    Ok(sqlx::query(
        "UPDATE attendance_auto_end_events SET notified_at = ?
         WHERE id = ? AND guild_id = ? AND user_id = ? AND notified_at IS NULL",
    )
    .bind(now)
    .bind(event_id)
    .bind(guild_id)
    .bind(user_id)
    .execute(pool)
    .await?
    .rows_affected()
        == 1)
}

#[derive(Debug, FromRow)]
struct AutoEndEventIdentity {
    id: i64,
    automatic_ended_at: i64,
}

#[derive(Debug, FromRow)]
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
