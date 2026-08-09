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

#[derive(Debug)]
pub enum RevertOutcome {
    Reverted { operation: String, session_id: i64 },
    NothingToRevert,
    Conflict,
}

#[derive(Debug, Error)]
pub enum ServiceError {
    #[error("終了時刻は開始時刻以降である必要がある")]
    EndBeforeStart,
    #[error("活動中の記録を複数作ることはできない")]
    OpenSessionConflict,
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
    let Some(open) = repository::open_session(pool, guild_id, user_id).await? else {
        if let Some(auto_ended) = repository::latest_auto_ended(pool, guild_id, user_id).await? {
            if ended_at < auto_ended.session.started_at {
                return Err(ServiceError::EndBeforeStart);
            }
            if let Some(corrected) = repository::correct_auto_ended_session(
                pool,
                auto_ended.session.id,
                guild_id,
                user_id,
                ended_at,
                note,
                now,
            )
            .await?
            {
                return Ok(EndOutcome::AutoEndedCorrected {
                    session: corrected.session,
                    automatic_end: corrected.automatic_ended_at,
                });
            }
        }
        return Ok(EndOutcome::AlreadyInactive(
            repository::latest_completed(pool, guild_id, user_id).await?,
        ));
    };
    if ended_at < open.started_at {
        return Err(ServiceError::EndBeforeStart);
    }
    if repository::close_session(pool, open.id, ended_at, note, now).await? == 0 {
        return Ok(EndOutcome::AlreadyInactive(
            repository::latest_completed(pool, guild_id, user_id).await?,
        ));
    }
    Ok(EndOutcome::Ended(
        repository::get_owned(pool, open.id, guild_id, user_id)
            .await?
            .expect("closed row"),
    ))
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
    match repository::reopen_session(pool, latest.id, now).await {
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
        Err(e) => Err(e.into()),
    }
}

pub async fn revert(
    pool: &SqlitePool,
    guild_id: i64,
    user_id: i64,
    now: i64,
) -> Result<RevertOutcome, ServiceError> {
    Ok(
        match repository::revert_latest(pool, guild_id, user_id, now).await? {
            repository::RevertResult::Reverted {
                operation,
                session_id,
            } => RevertOutcome::Reverted {
                operation,
                session_id,
            },
            repository::RevertResult::NothingToRevert => RevertOutcome::NothingToRevert,
            repository::RevertResult::Conflict => RevertOutcome::Conflict,
        },
    )
}
