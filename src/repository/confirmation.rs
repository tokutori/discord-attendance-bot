use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    time::{SystemTime, UNIX_EPOCH},
};

use sqlx::{FromRow, Sqlite, SqlitePool, Transaction};

use super::{
    ConfirmationInput, ConfirmationRequest, ConfirmationResult, SnapshotRow,
    change::{ChangeInput, ChangeRow, insert_change, matches_after},
};

pub const CONFIRMATION_TTL_SECONDS: i64 = 5 * 60;

#[derive(Debug, FromRow)]
struct ConfirmationRow {
    id: i64,
    action: String,
    session_id: i64,
    change_id: Option<i64>,
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
        "DELETE FROM pending_attendance_actions
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
            "INSERT INTO pending_attendance_actions (
                guild_id, user_id, code, action, session_id, change_id,
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
        .bind(input.change_id)
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
                    change_id: input.change_id,
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

async fn consume_confirmation(
    tx: &mut Transaction<'_, Sqlite>,
    confirmation_id: i64,
    now: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE pending_attendance_actions SET consumed_at = ?
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
        "SELECT id, action, session_id, change_id,
                expected_started_at, expected_ended_at, expected_note, expected_deleted_at,
                target_started_at, target_ended_at, target_note,
                expires_at, consumed_at
         FROM pending_attendance_actions
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
            insert_change(
                &mut tx,
                ChangeInput {
                    guild_id,
                    user_id,
                    session_id: confirmation.session_id,
                    kind: "edit",
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
            insert_change(
                &mut tx,
                ChangeInput {
                    guild_id,
                    user_id,
                    session_id: confirmation.session_id,
                    kind: "delete",
                    before: Some(&current),
                    after: &after,
                    created_at: now,
                },
            )
            .await?;
        }
        "revert" => {
            let Some(change_id) = confirmation.change_id else {
                consume_confirmation(&mut tx, confirmation.id, now).await?;
                tx.commit().await?;
                return Ok(ConfirmationResult::Conflict);
            };
            let Some(change) = sqlx::query_as::<_, ChangeRow>(
                "SELECT id, kind AS operation, session_id,
                        before_started_at, before_ended_at, before_note, before_deleted_at,
                        after_started_at, after_ended_at, after_note, after_deleted_at
                 FROM attendance_changes
                 WHERE id = ? AND guild_id = ? AND user_id = ? AND reverted_at IS NULL",
            )
            .bind(change_id)
            .bind(guild_id)
            .bind(user_id)
            .fetch_optional(&mut *tx)
            .await?
            else {
                consume_confirmation(&mut tx, confirmation.id, now).await?;
                tx.commit().await?;
                return Ok(ConfirmationResult::Conflict);
            };
            if change.session_id != confirmation.session_id || !matches_after(&current, &change) {
                consume_confirmation(&mut tx, confirmation.id, now).await?;
                tx.commit().await?;
                return Ok(ConfirmationResult::Conflict);
            }
            let rows_affected = if change.operation == "start" {
                sqlx::query(
                    "UPDATE attendance_sessions SET deleted_at = ?, updated_at = ?
                     WHERE id = ? AND guild_id = ? AND user_id = ? AND deleted_at IS NULL",
                )
                .bind(now)
                .bind(now)
                .bind(change.session_id)
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
                .bind(change.before_started_at)
                .bind(change.before_ended_at)
                .bind(&change.before_note)
                .bind(change.before_deleted_at)
                .bind(now)
                .bind(change.session_id)
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
            sqlx::query("UPDATE attendance_changes SET reverted_at = ? WHERE id = ?")
                .bind(now)
                .bind(change.id)
                .execute(&mut *tx)
                .await?;
            operation_name = Some(change.operation);
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
