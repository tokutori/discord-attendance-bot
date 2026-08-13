use sqlx::SqlitePool;
use thiserror::Error;

use crate::{attendance::AttendanceSession, repository};

#[derive(Debug)]
pub enum StartOutcome {
    Started(AttendanceSession),
    AlreadyActive(AttendanceSession),
}
#[derive(Debug)]
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
    if started_at > now {
        return Err(ServiceError::FutureTime);
    }
    if let Some(existing) = repository::open_session(pool, guild_id, user_id).await? {
        return Ok(StartOutcome::AlreadyActive(existing));
    }
    match repository::insert_session(pool, guild_id, user_id, display_name, started_at, note, now)
        .await
    {
        Ok(id) => Ok(StartOutcome::Started(
            repository::get_owned(pool, id, guild_id, user_id)
                .await?
                .ok_or(ServiceError::Invariant(
                    "inserted session could not be read back",
                ))?,
        )),
        Err(repository::SessionMutationError::Database(e))
            if e.as_database_error()
                .is_some_and(|database_error| database_error.is_unique_violation()) =>
        {
            Ok(StartOutcome::AlreadyActive(
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

pub async fn end(
    pool: &SqlitePool,
    guild_id: i64,
    user_id: i64,
    ended_at: i64,
    note: Option<&str>,
    now: i64,
) -> Result<EndOutcome, ServiceError> {
    if ended_at > now {
        return Err(ServiceError::FutureTime);
    }
    match repository::end_or_correct_session(pool, guild_id, user_id, ended_at, note, now).await {
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
