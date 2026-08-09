use sqlx::{FromRow, Sqlite, SqlitePool, Transaction};

use crate::attendance::AttendanceSession;

use super::{RevertPreview, RevertPreviewResult, SnapshotRow};

#[derive(Debug, FromRow)]
pub(super) struct ChangeRow {
    pub(super) id: i64,
    pub(super) operation: String,
    pub(super) session_id: i64,
    pub(super) before_started_at: Option<i64>,
    pub(super) before_ended_at: Option<i64>,
    pub(super) before_note: Option<String>,
    pub(super) before_deleted_at: Option<i64>,
    pub(super) after_started_at: i64,
    pub(super) after_ended_at: Option<i64>,
    pub(super) after_note: Option<String>,
    pub(super) after_deleted_at: Option<i64>,
}

pub(super) struct ChangeInput<'a> {
    pub(super) guild_id: i64,
    pub(super) user_id: i64,
    pub(super) session_id: i64,
    pub(super) kind: &'a str,
    pub(super) before: Option<&'a SnapshotRow>,
    pub(super) after: &'a SnapshotRow,
    pub(super) created_at: i64,
}

pub(super) fn snapshot(session: &AttendanceSession) -> SnapshotRow {
    SnapshotRow {
        started_at: session.started_at,
        ended_at: session.ended_at,
        note: session.note.clone(),
        deleted_at: session.deleted_at,
    }
}

pub(super) async fn insert_change(
    tx: &mut Transaction<'_, Sqlite>,
    input: ChangeInput<'_>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO attendance_changes (
            guild_id, user_id, session_id, kind,
            before_started_at, before_ended_at, before_note, before_deleted_at,
            after_started_at, after_ended_at, after_note, after_deleted_at,
            created_at
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(input.guild_id)
    .bind(input.user_id)
    .bind(input.session_id)
    .bind(input.kind)
    .bind(input.before.map(|value| value.started_at))
    .bind(input.before.and_then(|value| value.ended_at))
    .bind(input.before.and_then(|value| value.note.as_deref()))
    .bind(input.before.and_then(|value| value.deleted_at))
    .bind(input.after.started_at)
    .bind(input.after.ended_at)
    .bind(input.after.note.as_deref())
    .bind(input.after.deleted_at)
    .bind(input.created_at)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub(super) fn matches_after(current: &SnapshotRow, change: &ChangeRow) -> bool {
    current.started_at == change.after_started_at
        && current.ended_at == change.after_ended_at
        && current.note.as_deref() == change.after_note.as_deref()
        && current.deleted_at == change.after_deleted_at
}

pub async fn latest_revert_preview(
    pool: &SqlitePool,
    guild_id: i64,
    user_id: i64,
) -> Result<RevertPreviewResult, sqlx::Error> {
    let Some(change) = sqlx::query_as::<_, ChangeRow>(
        "SELECT id, kind AS operation, session_id,
                before_started_at, before_ended_at, before_note, before_deleted_at,
                after_started_at, after_ended_at, after_note, after_deleted_at
         FROM attendance_changes
         WHERE guild_id = ? AND user_id = ? AND reverted_at IS NULL
         ORDER BY id DESC LIMIT 1",
    )
    .bind(guild_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await?
    else {
        return Ok(RevertPreviewResult::NothingToRevert);
    };
    let Some(current) = sqlx::query_as::<_, SnapshotRow>(
        "SELECT started_at, ended_at, note, deleted_at
         FROM attendance_sessions
         WHERE id = ? AND guild_id = ? AND user_id = ?",
    )
    .bind(change.session_id)
    .bind(guild_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await?
    else {
        return Ok(RevertPreviewResult::Conflict);
    };
    if !matches_after(&current, &change) {
        return Ok(RevertPreviewResult::Conflict);
    }
    Ok(RevertPreviewResult::Available(RevertPreview {
        change_id: change.id,
        operation: change.operation,
        session_id: change.session_id,
        current,
        before_started_at: change.before_started_at,
        before_ended_at: change.before_ended_at,
        before_note: change.before_note,
        before_deleted_at: change.before_deleted_at,
    }))
}
