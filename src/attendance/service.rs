use serde::{Deserialize, Serialize};
use sqlx::{Sqlite, SqlitePool, Transaction};
use thiserror::Error;

use crate::{attendance::AttendanceSession, repository};

#[derive(Debug, Serialize, Deserialize)]
pub enum StartOutcome {
    Started(AttendanceSession),
    AlreadyActive(AttendanceSession),
}
#[derive(Debug, Serialize, Deserialize)]
pub enum EndOutcome {
    Ended(AttendanceSession),
    AutoEndedCorrected {
        session: AttendanceSession,
        automatic_end: i64,
    },
    AlreadyInactive(Option<AttendanceSession>),
}
#[derive(Debug)]
pub enum ContinueOutcome {
    Continued {
        session: AttendanceSession,
        removed_end: i64,
    },
    AlreadyActive(AttendanceSession),
    NothingToContinue,
}

#[derive(Debug, Error)]
pub enum ServiceError {
    #[error("終了時刻は開始時刻以降である必要がある")]
    EndBeforeStart,
    #[error("活動中の記録を複数作ることはできない")]
    OpenSessionConflict,
    #[error("活動記録の時間帯が別の記録と重複している")]
    OverlappingSession,
    #[error("未来の時刻は指定できない")]
    FutureTime,
    #[error("活動記録の内部状態が不正である: {0}")]
    Invariant(&'static str),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

pub async fn start(
    pool: &SqlitePool,
    guild_id: i64,
    user_id: i64,
    display_name: &str,
    started_at: i64,
    note: Option<&str>,
    now: i64,
) -> Result<StartOutcome, ServiceError> {
    let mut tx = repository::begin_immediate(pool).await?;
    let outcome = start_in_tx(
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
    Ok(outcome)
}

pub(crate) async fn start_in_tx(
    tx: &mut Transaction<'_, Sqlite>,
    guild_id: i64,
    user_id: i64,
    display_name: &str,
    started_at: i64,
    note: Option<&str>,
    now: i64,
) -> Result<StartOutcome, ServiceError> {
    if started_at > now {
        return Err(ServiceError::FutureTime);
    }
    let existing = sqlx::query_as::<_, AttendanceSession>(
        "SELECT * FROM attendance_sessions WHERE guild_id=? AND user_id=? AND ended_at IS NULL AND deleted_at IS NULL LIMIT 1"
    ).bind(guild_id).bind(user_id).fetch_optional(&mut **tx).await?;
    if let Some(existing) = existing {
        return Ok(StartOutcome::AlreadyActive(existing));
    }
    let id = repository::insert_session_in_tx(
        tx,
        guild_id,
        user_id,
        display_name,
        started_at,
        note,
        now,
    )
    .await
    .map_err(map_mutation_error)?;
    let session = sqlx::query_as::<_, AttendanceSession>(
        "SELECT * FROM attendance_sessions WHERE id=? AND guild_id=? AND user_id=?",
    )
    .bind(id)
    .bind(guild_id)
    .bind(user_id)
    .fetch_one(&mut **tx)
    .await?;
    Ok(StartOutcome::Started(session))
}

fn map_mutation_error(error: repository::SessionMutationError) -> ServiceError {
    match error {
        repository::SessionMutationError::Overlapping => ServiceError::OverlappingSession,
        repository::SessionMutationError::EndBeforeStart => ServiceError::EndBeforeStart,
        repository::SessionMutationError::Database(error) => error.into(),
        repository::SessionMutationError::Invariant(message) => ServiceError::Invariant(message),
    }
}

pub async fn end(
    pool: &SqlitePool,
    guild_id: i64,
    user_id: i64,
    ended_at: i64,
    note: Option<&str>,
    now: i64,
) -> Result<EndOutcome, ServiceError> {
    let mut tx = repository::begin_immediate(pool).await?;
    let outcome = end_in_tx(&mut tx, guild_id, user_id, ended_at, note, now).await?;
    tx.commit().await?;
    Ok(outcome)
}

pub(crate) async fn end_in_tx(
    tx: &mut Transaction<'_, Sqlite>,
    guild_id: i64,
    user_id: i64,
    ended_at: i64,
    note: Option<&str>,
    now: i64,
) -> Result<EndOutcome, ServiceError> {
    if ended_at > now {
        return Err(ServiceError::FutureTime);
    }
    match repository::end_or_correct_session_in_tx(tx, guild_id, user_id, ended_at, note, now).await
    {
        Ok(repository::EndSessionResult::Ended(session)) => Ok(EndOutcome::Ended(session)),
        Ok(repository::EndSessionResult::AutoEndedCorrected {
            session,
            automatic_ended_at,
        }) => Ok(EndOutcome::AutoEndedCorrected {
            session,
            automatic_end: automatic_ended_at,
        }),
        Ok(repository::EndSessionResult::AlreadyInactive(latest)) => {
            Ok(EndOutcome::AlreadyInactive(latest))
        }
        Ok(repository::EndSessionResult::EndBeforeStart) => Err(ServiceError::EndBeforeStart),
        Err(repository::SessionMutationError::Overlapping) => Err(ServiceError::OverlappingSession),
        Err(repository::SessionMutationError::EndBeforeStart) => Err(ServiceError::EndBeforeStart),
        Err(repository::SessionMutationError::Database(error)) => Err(error.into()),
        Err(repository::SessionMutationError::Invariant(message)) => {
            Err(ServiceError::Invariant(message))
        }
    }
}

pub async fn continue_activity(
    pool: &SqlitePool,
    guild_id: i64,
    user_id: i64,
    now: i64,
) -> Result<ContinueOutcome, ServiceError> {
    if let Some(open) = repository::open_session(pool, guild_id, user_id).await? {
        return Ok(ContinueOutcome::AlreadyActive(open));
    }
    let Some(latest) = repository::latest_completed(pool, guild_id, user_id).await? else {
        return Ok(ContinueOutcome::NothingToContinue);
    };
    let Some(removed_end) = latest.ended_at else {
        return Ok(ContinueOutcome::NothingToContinue);
    };
    match repository::reopen_session(pool, latest.id, guild_id, user_id, now).await {
        Ok(1) => Ok(ContinueOutcome::Continued {
            session: repository::get_owned(pool, latest.id, guild_id, user_id)
                .await?
                .ok_or(ServiceError::Invariant(
                    "reopened session could not be read back",
                ))?,
            removed_end,
        }),
        Ok(_) => Ok(repository::open_session(pool, guild_id, user_id)
            .await?
            .map(ContinueOutcome::AlreadyActive)
            .unwrap_or(ContinueOutcome::NothingToContinue)),
        Err(repository::SessionMutationError::Database(e))
            if e.as_database_error()
                .is_some_and(|database_error| database_error.is_unique_violation()) =>
        {
            Ok(ContinueOutcome::AlreadyActive(
                repository::open_session(pool, guild_id, user_id)
                    .await?
                    .ok_or(ServiceError::OpenSessionConflict)?,
            ))
        }
        Err(repository::SessionMutationError::Overlapping) => Err(ServiceError::OverlappingSession),
        Err(repository::SessionMutationError::Database(e)) => Err(e.into()),
        Err(repository::SessionMutationError::EndBeforeStart) => Err(ServiceError::EndBeforeStart),
        Err(repository::SessionMutationError::Invariant(message)) => {
            Err(ServiceError::Invariant(message))
        }
    }
}
