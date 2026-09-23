pub use attendance_query::sql::{
    active_attendance_members, active_sessions, get_owned, history, latest_completed, open_session,
    overlapping_completed, overlapping_for_export,
};
use sqlx::{Sqlite, SqlitePool, Transaction};

use crate::attendance::AttendanceSession;

use super::{
    ChangeOperation, EndSessionResult, SnapshotRow,
    auto_end::{correct_auto_ended_session_in_tx, mark_active_auto_end_corrected},
    change::{ChangeInput, insert_change, snapshot},
    transaction::begin_immediate,
};

pub(super) async fn has_session_overlap(
    tx: &mut Transaction<'_, Sqlite>,
    guild_id: i64,
    user_id: i64,
    excluded_session_id: i64,
    started_at: i64,
    ended_at: Option<i64>,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT EXISTS (
            SELECT 1 FROM attendance_sessions
            WHERE guild_id = ? AND user_id = ? AND id != ? AND deleted_at IS NULL
              AND started_at < COALESCE(?, 9223372036854775807)
              AND COALESCE(ended_at, 9223372036854775807) > ?
         )",
    )
    .bind(guild_id)
    .bind(user_id)
    .bind(excluded_session_id)
    .bind(ended_at)
    .bind(started_at)
    .fetch_one(&mut **tx)
    .await
}

pub async fn insert_session(
    pool: &SqlitePool,
    guild_id: i64,
    user_id: i64,
    display_name: &str,
    started_at: i64,
    note: Option<&str>,
    now: i64,
) -> Result<i64, super::SessionMutationError> {
    let mut tx = begin_immediate(pool).await?;
    let id = insert_session_in_tx(
        &mut tx,
        guild_id,
        user_id,
        display_name,
        started_at,
        note,
        now,
    )
    .await?;
    tx.commit().await?;
    Ok(id)
}

pub(crate) async fn insert_session_in_tx(
    tx: &mut Transaction<'_, Sqlite>,
    guild_id: i64,
    user_id: i64,
    display_name: &str,
    started_at: i64,
    note: Option<&str>,
    now: i64,
) -> Result<i64, super::SessionMutationError> {
    if has_session_overlap(tx, guild_id, user_id, -1, started_at, None).await? {
        return Err(super::SessionMutationError::Overlapping);
    }
    let result = sqlx::query("INSERT INTO attendance_sessions (guild_id,user_id,display_name,started_at,ended_at,open_since,note,created_at,updated_at) VALUES (?,?,?,?,NULL,?,?,?,?)")
        .bind(guild_id).bind(user_id).bind(display_name).bind(started_at).bind(started_at).bind(note).bind(now).bind(now).execute(&mut **tx).await
        ?;
    let id = result.last_insert_rowid();
    let after = SnapshotRow {
        started_at,
        ended_at: None,
        open_since: Some(started_at),
        note: note.map(str::to_owned),
        deleted_at: None,
    };
    insert_change(
        tx,
        ChangeInput {
            guild_id,
            user_id,
            session_id: id,
            kind: ChangeOperation::Start,
            before: None,
            after: &after,
            created_at: now,
        },
    )
    .await?;
    Ok(id)
}

pub async fn close_session(
    pool: &SqlitePool,
    id: i64,
    guild_id: i64,
    user_id: i64,
    ended_at: i64,
    note: Option<&str>,
    now: i64,
) -> Result<u64, super::SessionMutationError> {
    let mut tx = begin_immediate(pool).await?;
    let Some(existing) = sqlx::query_as::<_, AttendanceSession>(
        "SELECT * FROM attendance_sessions
         WHERE id = ? AND guild_id = ? AND user_id = ?
           AND ended_at IS NULL AND deleted_at IS NULL",
    )
    .bind(id)
    .bind(guild_id)
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await?
    else {
        return Ok(0);
    };
    if ended_at < existing.started_at {
        return Err(super::SessionMutationError::EndBeforeStart);
    }
    let after = SnapshotRow {
        started_at: existing.started_at,
        ended_at: Some(ended_at),
        open_since: None,
        note: note.map(str::to_owned).or(existing.note.clone()),
        deleted_at: existing.deleted_at,
    };
    if has_session_overlap(
        &mut tx,
        guild_id,
        user_id,
        id,
        existing.started_at,
        Some(ended_at),
    )
    .await?
    {
        return Err(super::SessionMutationError::Overlapping);
    }
    let result = sqlx::query("UPDATE attendance_sessions SET ended_at = ?, open_since = NULL, note = ?, updated_at = ? WHERE id = ? AND guild_id = ? AND user_id = ? AND ended_at IS NULL AND deleted_at IS NULL")
        .bind(ended_at).bind(&after.note).bind(now).bind(id).bind(guild_id).bind(user_id).execute(&mut *tx).await?;
    if result.rows_affected() == 1 {
        insert_change(
            &mut tx,
            ChangeInput {
                guild_id: existing.guild_id,
                user_id: existing.user_id,
                session_id: id,
                kind: ChangeOperation::End,
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

pub async fn end_or_correct_session(
    pool: &SqlitePool,
    guild_id: i64,
    user_id: i64,
    ended_at: i64,
    note: Option<&str>,
    now: i64,
) -> Result<EndSessionResult, super::SessionMutationError> {
    let mut tx = begin_immediate(pool).await?;
    let outcome =
        end_or_correct_session_in_tx(&mut tx, guild_id, user_id, ended_at, note, now).await?;
    tx.commit().await?;
    Ok(outcome)
}

pub(crate) async fn end_or_correct_session_in_tx(
    tx: &mut Transaction<'_, Sqlite>,
    guild_id: i64,
    user_id: i64,
    ended_at: i64,
    note: Option<&str>,
    now: i64,
) -> Result<EndSessionResult, super::SessionMutationError> {
    if let Some(existing) = sqlx::query_as::<_, AttendanceSession>(
        "SELECT * FROM attendance_sessions
         WHERE guild_id = ? AND user_id = ?
           AND ended_at IS NULL AND deleted_at IS NULL LIMIT 1",
    )
    .bind(guild_id)
    .bind(user_id)
    .fetch_optional(&mut **tx)
    .await?
    {
        if ended_at < existing.started_at {
            return Ok(EndSessionResult::EndBeforeStart);
        }
        if has_session_overlap(
            tx,
            guild_id,
            user_id,
            existing.id,
            existing.started_at,
            Some(ended_at),
        )
        .await?
        {
            return Err(super::SessionMutationError::Overlapping);
        }
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
               AND ended_at IS NULL AND deleted_at IS NULL",
        )
        .bind(ended_at)
        .bind(&after.note)
        .bind(now)
        .bind(existing.id)
        .bind(guild_id)
        .bind(user_id)
        .execute(&mut **tx)
        .await?;
        if result.rows_affected() != 1 {
            return Ok(EndSessionResult::AlreadyInactive(None));
        }
        insert_change(
            tx,
            ChangeInput {
                guild_id,
                user_id,
                session_id: existing.id,
                kind: ChangeOperation::End,
                before: Some(&snapshot(&existing)),
                after: &after,
                created_at: now,
            },
        )
        .await?;
        return Ok(EndSessionResult::Ended(AttendanceSession {
            ended_at: Some(ended_at),
            open_since: None,
            note: after.note,
            updated_at: now,
            ..existing
        }));
    }

    if let Some(corrected) =
        correct_auto_ended_session_in_tx(tx, None, guild_id, user_id, ended_at, note, now).await?
    {
        return Ok(EndSessionResult::AutoEndedCorrected {
            session: corrected.session,
            automatic_ended_at: corrected.automatic_ended_at,
        });
    }

    let latest = sqlx::query_as::<_, AttendanceSession>(
        "SELECT * FROM attendance_sessions
         WHERE guild_id = ? AND user_id = ? AND ended_at IS NOT NULL AND deleted_at IS NULL
         ORDER BY ended_at DESC LIMIT 1",
    )
    .bind(guild_id)
    .bind(user_id)
    .fetch_optional(&mut **tx)
    .await?;
    Ok(EndSessionResult::AlreadyInactive(latest))
}

pub async fn reopen_session(
    pool: &SqlitePool,
    id: i64,
    guild_id: i64,
    user_id: i64,
    now: i64,
) -> Result<u64, super::SessionMutationError> {
    let mut tx = begin_immediate(pool).await?;
    let Some(existing) = sqlx::query_as::<_, AttendanceSession>(
        "SELECT * FROM attendance_sessions
         WHERE id = ? AND guild_id = ? AND user_id = ?
           AND ended_at IS NOT NULL AND deleted_at IS NULL",
    )
    .bind(id)
    .bind(guild_id)
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await?
    else {
        return Ok(0);
    };
    if now < existing.started_at {
        return Err(super::SessionMutationError::EndBeforeStart);
    }
    let after = SnapshotRow {
        started_at: existing.started_at,
        ended_at: None,
        open_since: Some(now),
        note: existing.note.clone(),
        deleted_at: existing.deleted_at,
    };
    if has_session_overlap(&mut tx, guild_id, user_id, id, existing.started_at, None).await? {
        return Err(super::SessionMutationError::Overlapping);
    }
    let result = sqlx::query("UPDATE attendance_sessions SET ended_at = NULL, open_since = ?, updated_at = ? WHERE id = ? AND guild_id = ? AND user_id = ? AND ended_at IS NOT NULL AND deleted_at IS NULL")
        .bind(now).bind(now).bind(id).bind(guild_id).bind(user_id).execute(&mut *tx).await?;
    if result.rows_affected() == 1 {
        mark_active_auto_end_corrected(&mut tx, id, now).await?;
        insert_change(
            &mut tx,
            ChangeInput {
                guild_id: existing.guild_id,
                user_id: existing.user_id,
                session_id: id,
                kind: ChangeOperation::Continue,
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
