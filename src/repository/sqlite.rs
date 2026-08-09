use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    time::{SystemTime, UNIX_EPOCH},
};

use sqlx::{FromRow, Sqlite, SqlitePool, Transaction};

use crate::attendance::AttendanceSession;

pub struct AttendanceUpdate<'a> {
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub note: Option<&'a str>,
}

pub const CONFIRMATION_TTL_SECONDS: i64 = 5 * 60;

#[derive(Debug, Clone)]
pub struct ConfirmationInput<'a> {
    pub action: &'a str,
    pub session_id: i64,
    pub operation_id: Option<i64>,
    pub expected: &'a SessionSnapshot,
    pub target_started_at: Option<i64>,
    pub target_ended_at: Option<i64>,
    pub target_note: Option<&'a str>,
    pub requested_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmationRequest {
    pub code: String,
    pub action: String,
    pub session_id: i64,
    pub operation_id: Option<i64>,
    pub target_started_at: Option<i64>,
    pub target_ended_at: Option<i64>,
    pub target_note: Option<String>,
    pub expires_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevertPreview {
    pub operation_id: i64,
    pub operation: String,
    pub session_id: i64,
    pub current: SessionSnapshot,
    pub before_started_at: Option<i64>,
    pub before_ended_at: Option<i64>,
    pub before_note: Option<String>,
    pub before_deleted_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfirmationResult {
    Confirmed {
        action: String,
        session_id: i64,
        operation: Option<String>,
    },
    NotFound,
    Expired,
    Conflict,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RevertPreviewResult {
    Available(RevertPreview),
    NothingToRevert,
    Conflict,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RevertResult {
    Reverted { operation: String, session_id: i64 },
    NothingToRevert,
    Conflict,
}

#[derive(Debug, FromRow)]
struct OperationRow {
    id: i64,
    operation: String,
    session_id: i64,
    before_started_at: Option<i64>,
    before_ended_at: Option<i64>,
    before_note: Option<String>,
    before_deleted_at: Option<i64>,
    after_started_at: i64,
    after_ended_at: Option<i64>,
    after_note: Option<String>,
    after_deleted_at: Option<i64>,
}

#[derive(Debug, FromRow)]
struct ConfirmationRow {
    id: i64,
    action: String,
    session_id: i64,
    operation_id: Option<i64>,
    expected_started_at: i64,
    expected_ended_at: Option<i64>,
    expected_note: Option<String>,
    expected_deleted_at: Option<i64>,
    target_started_at: Option<i64>,
    target_ended_at: Option<i64>,
    target_note: Option<String>,
    expires_at: i64,
    consumed_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct SnapshotRow {
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub note: Option<String>,
    pub deleted_at: Option<i64>,
}

pub type SessionSnapshot = SnapshotRow;

struct OperationInput<'a> {
    guild_id: i64,
    user_id: i64,
    session_id: i64,
    operation: &'a str,
    before: Option<&'a SnapshotRow>,
    after: &'a SnapshotRow,
    created_at: i64,
}

fn snapshot(session: &AttendanceSession) -> SnapshotRow {
    SnapshotRow {
        started_at: session.started_at,
        ended_at: session.ended_at,
        note: session.note.clone(),
        deleted_at: session.deleted_at,
    }
}

async fn insert_operation(
    tx: &mut Transaction<'_, Sqlite>,
    input: OperationInput<'_>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO attendance_operations (
            guild_id, user_id, session_id, operation,
            before_started_at, before_ended_at, before_note, before_deleted_at,
            after_started_at, after_ended_at, after_note, after_deleted_at,
            created_at
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(input.guild_id)
    .bind(input.user_id)
    .bind(input.session_id)
    .bind(input.operation)
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

fn confirmation_code(
    guild_id: i64,
    user_id: i64,
    session_id: i64,
    action: &str,
    requested_at: i64,
    nonce: u128,
    attempt: u64,
) -> String {
    let mut hasher = DefaultHasher::new();
    guild_id.hash(&mut hasher);
    user_id.hash(&mut hasher);
    session_id.hash(&mut hasher);
    action.hash(&mut hasher);
    requested_at.hash(&mut hasher);
    nonce.hash(&mut hasher);
    attempt.hash(&mut hasher);
    let mut value = hasher.finish();
    let alphabet = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let mut code = String::with_capacity(5);
    for _ in 0..5 {
        code.push(alphabet[(value & 31) as usize] as char);
        value >>= 5;
    }
    code
}

pub async fn create_confirmation(
    pool: &SqlitePool,
    guild_id: i64,
    user_id: i64,
    input: ConfirmationInput<'_>,
) -> Result<ConfirmationRequest, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let expires_at = input.requested_at + CONFIRMATION_TTL_SECONDS;
    sqlx::query(
        "DELETE FROM attendance_confirmations
         WHERE consumed_at IS NOT NULL OR expires_at <= ?",
    )
    .bind(input.requested_at)
    .execute(&mut *tx)
    .await?;

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    for attempt in 0..16 {
        let code = confirmation_code(
            guild_id,
            user_id,
            input.session_id,
            input.action,
            input.requested_at,
            nonce,
            attempt,
        );
        let result = sqlx::query(
            "INSERT INTO attendance_confirmations (
                guild_id, user_id, code, action, session_id, operation_id,
                expected_started_at, expected_ended_at, expected_note, expected_deleted_at,
                target_started_at, target_ended_at, target_note,
                requested_at, expires_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(guild_id)
        .bind(user_id)
        .bind(&code)
        .bind(input.action)
        .bind(input.session_id)
        .bind(input.operation_id)
        .bind(input.expected.started_at)
        .bind(input.expected.ended_at)
        .bind(input.expected.note.as_deref())
        .bind(input.expected.deleted_at)
        .bind(input.target_started_at)
        .bind(input.target_ended_at)
        .bind(input.target_note)
        .bind(input.requested_at)
        .bind(expires_at)
        .execute(&mut *tx)
        .await;
        match result {
            Ok(_) => {
                tx.commit().await?;
                return Ok(ConfirmationRequest {
                    code,
                    action: input.action.to_owned(),
                    session_id: input.session_id,
                    operation_id: input.operation_id,
                    target_started_at: input.target_started_at,
                    target_ended_at: input.target_ended_at,
                    target_note: input.target_note.map(str::to_owned),
                    expires_at,
                });
            }
            Err(sqlx::Error::Database(error)) if error.is_unique_violation() => continue,
            Err(error) => return Err(error),
        }
    }
    Err(sqlx::Error::Protocol(
        "could not allocate confirmation code".into(),
    ))
}

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
    insert_operation(
        &mut tx,
        OperationInput {
            guild_id,
            user_id,
            session_id: id,
            operation: "start",
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
        insert_operation(
            &mut tx,
            OperationInput {
                guild_id: existing.guild_id,
                user_id: existing.user_id,
                session_id: id,
                operation: "end",
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
        insert_operation(
            &mut tx,
            OperationInput {
                guild_id: existing.guild_id,
                user_id: existing.user_id,
                session_id: id,
                operation: "continue",
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

pub async fn update_owned(
    pool: &SqlitePool,
    id: i64,
    guild_id: i64,
    user_id: i64,
    update: AttendanceUpdate<'_>,
    now: i64,
) -> Result<u64, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let Some(existing) = sqlx::query_as::<_, AttendanceSession>(
        "SELECT * FROM attendance_sessions WHERE id = ? AND guild_id = ? AND user_id = ? AND deleted_at IS NULL",
    )
    .bind(id)
    .bind(guild_id)
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await?
    else {
        return Ok(0);
    };
    let after = SnapshotRow {
        started_at: update.started_at,
        ended_at: update.ended_at,
        note: update.note.map(str::to_owned),
        deleted_at: existing.deleted_at,
    };
    let result = sqlx::query("UPDATE attendance_sessions SET started_at = ?, ended_at = ?, note = ?, updated_at = ? WHERE id = ? AND guild_id = ? AND user_id = ? AND deleted_at IS NULL")
        .bind(after.started_at)
        .bind(after.ended_at)
        .bind(&after.note)
        .bind(now)
        .bind(id)
        .bind(guild_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    if result.rows_affected() == 1 {
        insert_operation(
            &mut tx,
            OperationInput {
                guild_id,
                user_id,
                session_id: id,
                operation: "edit",
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

pub async fn soft_delete_owned(
    pool: &SqlitePool,
    id: i64,
    guild_id: i64,
    user_id: i64,
    now: i64,
) -> Result<u64, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let Some(existing) = sqlx::query_as::<_, AttendanceSession>(
        "SELECT * FROM attendance_sessions WHERE id = ? AND guild_id = ? AND user_id = ? AND deleted_at IS NULL",
    )
    .bind(id)
    .bind(guild_id)
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await?
    else {
        return Ok(0);
    };
    let after = SnapshotRow {
        started_at: existing.started_at,
        ended_at: existing.ended_at,
        note: existing.note.clone(),
        deleted_at: Some(now),
    };
    let result = sqlx::query("UPDATE attendance_sessions SET deleted_at = ?, updated_at = ? WHERE id = ? AND guild_id = ? AND user_id = ? AND deleted_at IS NULL")
        .bind(now).bind(now).bind(id).bind(guild_id).bind(user_id).execute(&mut *tx).await?;
    if result.rows_affected() == 1 {
        insert_operation(
            &mut tx,
            OperationInput {
                guild_id,
                user_id,
                session_id: id,
                operation: "delete",
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

fn matches_after(current: &SnapshotRow, operation: &OperationRow) -> bool {
    current.started_at == operation.after_started_at
        && current.ended_at == operation.after_ended_at
        && current.note.as_deref() == operation.after_note.as_deref()
        && current.deleted_at == operation.after_deleted_at
}

pub async fn latest_revert_preview(
    pool: &SqlitePool,
    guild_id: i64,
    user_id: i64,
) -> Result<RevertPreviewResult, sqlx::Error> {
    let Some(operation) = sqlx::query_as::<_, OperationRow>(
        "SELECT id, operation, session_id,
                before_started_at, before_ended_at, before_note, before_deleted_at,
                after_started_at, after_ended_at, after_note, after_deleted_at
         FROM attendance_operations
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
    .bind(operation.session_id)
    .bind(guild_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await?
    else {
        return Ok(RevertPreviewResult::Conflict);
    };
    if !matches_after(&current, &operation) {
        return Ok(RevertPreviewResult::Conflict);
    }
    Ok(RevertPreviewResult::Available(RevertPreview {
        operation_id: operation.id,
        operation: operation.operation,
        session_id: operation.session_id,
        current,
        before_started_at: operation.before_started_at,
        before_ended_at: operation.before_ended_at,
        before_note: operation.before_note,
        before_deleted_at: operation.before_deleted_at,
    }))
}

async fn consume_confirmation(
    tx: &mut Transaction<'_, Sqlite>,
    confirmation_id: i64,
    now: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE attendance_confirmations SET consumed_at = ?
         WHERE id = ? AND consumed_at IS NULL",
    )
    .bind(now)
    .bind(confirmation_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub async fn confirm_confirmation(
    pool: &SqlitePool,
    guild_id: i64,
    user_id: i64,
    code: &str,
    now: i64,
) -> Result<ConfirmationResult, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let Some(confirmation) = sqlx::query_as::<_, ConfirmationRow>(
        "SELECT id, action, session_id, operation_id,
                expected_started_at, expected_ended_at, expected_note, expected_deleted_at,
                target_started_at, target_ended_at, target_note,
                expires_at, consumed_at
         FROM attendance_confirmations
         WHERE guild_id = ? AND user_id = ? AND code = ?",
    )
    .bind(guild_id)
    .bind(user_id)
    .bind(code)
    .fetch_optional(&mut *tx)
    .await?
    else {
        return Ok(ConfirmationResult::NotFound);
    };
    if confirmation.consumed_at.is_some() {
        return Ok(ConfirmationResult::NotFound);
    }
    if confirmation.expires_at <= now {
        return Ok(ConfirmationResult::Expired);
    }

    let Some(current) = sqlx::query_as::<_, SnapshotRow>(
        "SELECT started_at, ended_at, note, deleted_at
         FROM attendance_sessions
         WHERE id = ? AND guild_id = ? AND user_id = ?",
    )
    .bind(confirmation.session_id)
    .bind(guild_id)
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await?
    else {
        consume_confirmation(&mut tx, confirmation.id, now).await?;
        tx.commit().await?;
        return Ok(ConfirmationResult::Conflict);
    };
    let expected = SnapshotRow {
        started_at: confirmation.expected_started_at,
        ended_at: confirmation.expected_ended_at,
        note: confirmation.expected_note.clone(),
        deleted_at: confirmation.expected_deleted_at,
    };
    if current != expected {
        consume_confirmation(&mut tx, confirmation.id, now).await?;
        tx.commit().await?;
        return Ok(ConfirmationResult::Conflict);
    }

    let mut operation_name = None;
    match confirmation.action.as_str() {
        "edit" => {
            let Some(target_started_at) = confirmation.target_started_at else {
                consume_confirmation(&mut tx, confirmation.id, now).await?;
                tx.commit().await?;
                return Ok(ConfirmationResult::Conflict);
            };
            let after = SnapshotRow {
                started_at: target_started_at,
                ended_at: confirmation.target_ended_at,
                note: confirmation.target_note.clone(),
                deleted_at: current.deleted_at,
            };
            let result = sqlx::query(
                "UPDATE attendance_sessions
                 SET started_at = ?, ended_at = ?, note = ?, updated_at = ?
                 WHERE id = ? AND guild_id = ? AND user_id = ? AND deleted_at IS NULL",
            )
            .bind(after.started_at)
            .bind(after.ended_at)
            .bind(&after.note)
            .bind(now)
            .bind(confirmation.session_id)
            .bind(guild_id)
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
            if result.rows_affected() != 1 {
                consume_confirmation(&mut tx, confirmation.id, now).await?;
                tx.commit().await?;
                return Ok(ConfirmationResult::Conflict);
            }
            insert_operation(
                &mut tx,
                OperationInput {
                    guild_id,
                    user_id,
                    session_id: confirmation.session_id,
                    operation: "edit",
                    before: Some(&current),
                    after: &after,
                    created_at: now,
                },
            )
            .await?;
        }
        "delete" => {
            let after = SnapshotRow {
                started_at: current.started_at,
                ended_at: current.ended_at,
                note: current.note.clone(),
                deleted_at: Some(now),
            };
            let result = sqlx::query(
                "UPDATE attendance_sessions SET deleted_at = ?, updated_at = ?
                 WHERE id = ? AND guild_id = ? AND user_id = ? AND deleted_at IS NULL",
            )
            .bind(now)
            .bind(now)
            .bind(confirmation.session_id)
            .bind(guild_id)
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
            if result.rows_affected() != 1 {
                consume_confirmation(&mut tx, confirmation.id, now).await?;
                tx.commit().await?;
                return Ok(ConfirmationResult::Conflict);
            }
            insert_operation(
                &mut tx,
                OperationInput {
                    guild_id,
                    user_id,
                    session_id: confirmation.session_id,
                    operation: "delete",
                    before: Some(&current),
                    after: &after,
                    created_at: now,
                },
            )
            .await?;
        }
        "revert" => {
            let Some(operation_id) = confirmation.operation_id else {
                consume_confirmation(&mut tx, confirmation.id, now).await?;
                tx.commit().await?;
                return Ok(ConfirmationResult::Conflict);
            };
            let Some(operation) = sqlx::query_as::<_, OperationRow>(
                "SELECT id, operation, session_id,
                        before_started_at, before_ended_at, before_note, before_deleted_at,
                        after_started_at, after_ended_at, after_note, after_deleted_at
                 FROM attendance_operations
                 WHERE id = ? AND guild_id = ? AND user_id = ? AND reverted_at IS NULL",
            )
            .bind(operation_id)
            .bind(guild_id)
            .bind(user_id)
            .fetch_optional(&mut *tx)
            .await?
            else {
                consume_confirmation(&mut tx, confirmation.id, now).await?;
                tx.commit().await?;
                return Ok(ConfirmationResult::Conflict);
            };
            if operation.session_id != confirmation.session_id
                || !matches_after(&current, &operation)
            {
                consume_confirmation(&mut tx, confirmation.id, now).await?;
                tx.commit().await?;
                return Ok(ConfirmationResult::Conflict);
            }
            let rows_affected = if operation.operation == "start" {
                sqlx::query(
                    "UPDATE attendance_sessions SET deleted_at = ?, updated_at = ?
                     WHERE id = ? AND guild_id = ? AND user_id = ? AND deleted_at IS NULL",
                )
                .bind(now)
                .bind(now)
                .bind(operation.session_id)
                .bind(guild_id)
                .bind(user_id)
                .execute(&mut *tx)
                .await?
                .rows_affected()
            } else {
                sqlx::query(
                    "UPDATE attendance_sessions
                     SET started_at = ?, ended_at = ?, note = ?, deleted_at = ?, updated_at = ?
                     WHERE id = ? AND guild_id = ? AND user_id = ?",
                )
                .bind(operation.before_started_at)
                .bind(operation.before_ended_at)
                .bind(&operation.before_note)
                .bind(operation.before_deleted_at)
                .bind(now)
                .bind(operation.session_id)
                .bind(guild_id)
                .bind(user_id)
                .execute(&mut *tx)
                .await?
                .rows_affected()
            };
            if rows_affected != 1 {
                consume_confirmation(&mut tx, confirmation.id, now).await?;
                tx.commit().await?;
                return Ok(ConfirmationResult::Conflict);
            }
            sqlx::query("UPDATE attendance_operations SET reverted_at = ? WHERE id = ?")
                .bind(now)
                .bind(operation.id)
                .execute(&mut *tx)
                .await?;
            operation_name = Some(operation.operation);
        }
        _ => {
            consume_confirmation(&mut tx, confirmation.id, now).await?;
            tx.commit().await?;
            return Ok(ConfirmationResult::Conflict);
        }
    }

    consume_confirmation(&mut tx, confirmation.id, now).await?;
    tx.commit().await?;
    Ok(ConfirmationResult::Confirmed {
        action: confirmation.action,
        session_id: confirmation.session_id,
        operation: operation_name,
    })
}

pub async fn revert_latest(
    pool: &SqlitePool,
    guild_id: i64,
    user_id: i64,
    now: i64,
) -> Result<RevertResult, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let Some(operation) = sqlx::query_as::<_, OperationRow>(
        "SELECT id, operation, session_id,
                before_started_at, before_ended_at, before_note, before_deleted_at,
                after_started_at, after_ended_at, after_note, after_deleted_at
         FROM attendance_operations
         WHERE guild_id = ? AND user_id = ? AND reverted_at IS NULL
         ORDER BY id DESC LIMIT 1",
    )
    .bind(guild_id)
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await?
    else {
        return Ok(RevertResult::NothingToRevert);
    };

    let Some(current) = sqlx::query_as::<_, SnapshotRow>(
        "SELECT started_at, ended_at, note, deleted_at
         FROM attendance_sessions
         WHERE id = ? AND guild_id = ? AND user_id = ?",
    )
    .bind(operation.session_id)
    .bind(guild_id)
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await?
    else {
        return Ok(RevertResult::Conflict);
    };
    if !matches_after(&current, &operation) {
        return Ok(RevertResult::Conflict);
    }

    let rows_affected = if operation.operation == "start" {
        sqlx::query(
            "UPDATE attendance_sessions
             SET deleted_at = ?, updated_at = ?
             WHERE id = ? AND guild_id = ? AND user_id = ? AND deleted_at IS NULL",
        )
        .bind(now)
        .bind(now)
        .bind(operation.session_id)
        .bind(guild_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await?
        .rows_affected()
    } else {
        sqlx::query(
            "UPDATE attendance_sessions
             SET started_at = ?, ended_at = ?, note = ?, deleted_at = ?, updated_at = ?
             WHERE id = ? AND guild_id = ? AND user_id = ?",
        )
        .bind(operation.before_started_at)
        .bind(operation.before_ended_at)
        .bind(&operation.before_note)
        .bind(operation.before_deleted_at)
        .bind(now)
        .bind(operation.session_id)
        .bind(guild_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await?
        .rows_affected()
    };
    if rows_affected != 1 {
        return Ok(RevertResult::Conflict);
    }

    let marked = sqlx::query(
        "UPDATE attendance_operations SET reverted_at = ? WHERE id = ? AND reverted_at IS NULL",
    )
    .bind(now)
    .bind(operation.id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if marked != 1 {
        return Ok(RevertResult::Conflict);
    }

    tx.commit().await?;
    Ok(RevertResult::Reverted {
        operation: operation.operation,
        session_id: operation.session_id,
    })
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

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn test_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn reverts_all_mutating_operations_in_reverse_order() {
        let pool = test_pool().await;
        let id = insert_session(&pool, 1, 2, "Bem", 100, Some("start"), 100)
            .await
            .unwrap();
        close_session(&pool, id, 200, Some("end"), 200)
            .await
            .unwrap();
        reopen_session(&pool, id, 300).await.unwrap();
        update_owned(
            &pool,
            id,
            1,
            2,
            AttendanceUpdate {
                started_at: 150,
                ended_at: Some(250),
                note: Some("edit"),
            },
            400,
        )
        .await
        .unwrap();
        soft_delete_owned(&pool, id, 1, 2, 500).await.unwrap();

        assert_eq!(
            revert_latest(&pool, 1, 2, 600).await.unwrap(),
            RevertResult::Reverted {
                operation: "delete".into(),
                session_id: id,
            }
        );
        let session = get_owned(&pool, id, 1, 2).await.unwrap().unwrap();
        assert_eq!(
            (session.started_at, session.ended_at, session.note),
            (150, Some(250), Some("edit".into()))
        );

        assert_eq!(
            revert_latest(&pool, 1, 2, 700).await.unwrap(),
            RevertResult::Reverted {
                operation: "edit".into(),
                session_id: id,
            }
        );
        let session = get_owned(&pool, id, 1, 2).await.unwrap().unwrap();
        assert_eq!(
            (session.started_at, session.ended_at, session.note),
            (100, None, Some("end".into()))
        );

        assert_eq!(
            revert_latest(&pool, 1, 2, 800).await.unwrap(),
            RevertResult::Reverted {
                operation: "continue".into(),
                session_id: id,
            }
        );
        let session = get_owned(&pool, id, 1, 2).await.unwrap().unwrap();
        assert_eq!(
            (session.started_at, session.ended_at, session.note),
            (100, Some(200), Some("end".into()))
        );

        assert_eq!(
            revert_latest(&pool, 1, 2, 900).await.unwrap(),
            RevertResult::Reverted {
                operation: "end".into(),
                session_id: id,
            }
        );
        let session = get_owned(&pool, id, 1, 2).await.unwrap().unwrap();
        assert_eq!(
            (session.started_at, session.ended_at, session.note),
            (100, None, Some("start".into()))
        );

        assert_eq!(
            revert_latest(&pool, 1, 2, 1_000).await.unwrap(),
            RevertResult::Reverted {
                operation: "start".into(),
                session_id: id,
            }
        );
        assert!(get_owned(&pool, id, 1, 2).await.unwrap().is_none());
        assert_eq!(
            revert_latest(&pool, 1, 2, 1_100).await.unwrap(),
            RevertResult::NothingToRevert
        );
    }

    #[tokio::test]
    async fn confirmation_is_preview_only_until_confirmed_and_single_use() {
        let pool = test_pool().await;
        let id = insert_session(&pool, 1, 2, "Bem", 100, Some("old"), 100)
            .await
            .unwrap();
        let expected = SnapshotRow {
            started_at: 100,
            ended_at: None,
            note: Some("old".into()),
            deleted_at: None,
        };
        let request = create_confirmation(
            &pool,
            1,
            2,
            ConfirmationInput {
                action: "edit",
                session_id: id,
                operation_id: None,
                expected: &expected,
                target_started_at: Some(150),
                target_ended_at: Some(250),
                target_note: Some("new"),
                requested_at: 200,
            },
        )
        .await
        .unwrap();
        assert_eq!(request.code.len(), 5);
        assert_eq!(
            get_owned(&pool, id, 1, 2)
                .await
                .unwrap()
                .unwrap()
                .started_at,
            100
        );

        assert_eq!(
            confirm_confirmation(&pool, 1, 2, &request.code, 201)
                .await
                .unwrap(),
            ConfirmationResult::Confirmed {
                action: "edit".into(),
                session_id: id,
                operation: None,
            }
        );
        assert_eq!(
            confirm_confirmation(&pool, 1, 2, &request.code, 202)
                .await
                .unwrap(),
            ConfirmationResult::NotFound
        );

        let expected = SnapshotRow {
            started_at: 150,
            ended_at: Some(250),
            note: Some("new".into()),
            deleted_at: None,
        };
        let delete_request = create_confirmation(
            &pool,
            1,
            2,
            ConfirmationInput {
                action: "delete",
                session_id: id,
                operation_id: None,
                expected: &expected,
                target_started_at: None,
                target_ended_at: None,
                target_note: None,
                requested_at: 300,
            },
        )
        .await
        .unwrap();
        assert_eq!(
            confirm_confirmation(&pool, 1, 2, &delete_request.code, 301)
                .await
                .unwrap(),
            ConfirmationResult::Confirmed {
                action: "delete".into(),
                session_id: id,
                operation: None,
            }
        );

        let preview = match latest_revert_preview(&pool, 1, 2).await.unwrap() {
            RevertPreviewResult::Available(preview) => preview,
            result => panic!("unexpected preview result: {result:?}"),
        };
        let revert_request = create_confirmation(
            &pool,
            1,
            2,
            ConfirmationInput {
                action: "revert",
                session_id: preview.session_id,
                operation_id: Some(preview.operation_id),
                expected: &preview.current,
                target_started_at: None,
                target_ended_at: None,
                target_note: None,
                requested_at: 400,
            },
        )
        .await
        .unwrap();
        assert_eq!(
            confirm_confirmation(&pool, 1, 2, &revert_request.code, 401)
                .await
                .unwrap(),
            ConfirmationResult::Confirmed {
                action: "revert".into(),
                session_id: id,
                operation: Some("delete".into()),
            }
        );
        assert!(get_owned(&pool, id, 1, 2).await.unwrap().is_some());
    }
}
