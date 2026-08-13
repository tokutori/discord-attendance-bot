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
                .expect("inserted row"),
        )),
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
            Ok(StartOutcome::AlreadyActive(
                repository::open_session(pool, guild_id, user_id)
                    .await?
                    .ok_or(ServiceError::OpenSessionConflict)?,
            ))
        }
        Err(e) if is_overlap_error(&e) => Err(ServiceError::OverlappingSession),
        Err(e) => Err(e.into()),
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
        Err(error) if is_overlap_error(&error) => Err(ServiceError::OverlappingSession),
        Err(error) if is_end_before_start_error(&error) => Err(ServiceError::EndBeforeStart),
        Err(error) => Err(error.into()),
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
    let removed_end = latest.ended_at.expect("completed row");
    match repository::reopen_session(pool, latest.id, guild_id, user_id, now).await {
        Ok(1) => Ok(ContinueOutcome::Continued {
            session: repository::get_owned(pool, latest.id, guild_id, user_id)
                .await?
                .expect("reopened row"),
            removed_end,
        }),
        Ok(_) => Ok(repository::open_session(pool, guild_id, user_id)
            .await?
            .map(ContinueOutcome::AlreadyActive)
            .unwrap_or(ContinueOutcome::NothingToContinue)),
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
            Ok(ContinueOutcome::AlreadyActive(
                repository::open_session(pool, guild_id, user_id)
                    .await?
                    .ok_or(ServiceError::OpenSessionConflict)?,
            ))
        }
        Err(e) if is_overlap_error(&e) => Err(ServiceError::OverlappingSession),
        Err(e) => Err(e.into()),
    }
}

fn is_overlap_error(error: &sqlx::Error) -> bool {
    error
        .to_string()
        .contains("attendance session overlaps an existing session")
}

fn is_end_before_start_error(error: &sqlx::Error) -> bool {
    error.to_string().contains("attendance end before start")
}
