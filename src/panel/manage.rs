use std::future::Future;

use sqlx::SqlitePool;
use thiserror::Error;

use super::store::{self, Location};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum TransportError {
    #[error("panel message does not exist")]
    MissingMessage,
    #[error("panel request failed")]
    Failed,
}

/// A narrow HTTP boundary, replaceable in tests. Only Discord's Unknown Message
/// error permits recreation; permissions, missing channels and timeouts do not.
pub trait Transport {
    fn update(
        &mut self,
        location: Location,
        enabled: bool,
    ) -> impl Future<Output = Result<(), TransportError>> + Send;
    fn create_disabled(
        &mut self,
        channel_id: i64,
    ) -> impl Future<Output = Result<i64, TransportError>> + Send;
    fn delete(
        &mut self,
        location: Location,
    ) -> impl Future<Output = Result<(), TransportError>> + Send;
}

#[derive(Debug, Error)]
pub enum ManagementError {
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    Transport(#[from] TransportError),
}

#[derive(Debug)]
pub struct Managed {
    pub location: Location,
    pub enabled: bool,
    pub old_disabled: bool,
}

/// Caller serializes management requests with a management-only mutex.
/// No SQLite transaction spans a network request. Registration's commit is the
/// cutoff: clicks from the old location are rejected after that commit.
pub async fn install(
    pool: &SqlitePool,
    guild_id: i64,
    channel_id: i64,
    application_id: i64,
    transport: &mut impl Transport,
) -> Result<Managed, ManagementError> {
    let old = store::location(pool, guild_id).await?;
    if let Some(location) =
        old.filter(|p| p.channel_id == channel_id && p.application_id == application_id)
    {
        match transport.update(location, true).await {
            Ok(()) => {
                return Ok(Managed {
                    location,
                    enabled: true,
                    old_disabled: true,
                });
            }
            Err(TransportError::MissingMessage) => (),
            Err(error) => return Err(error.into()),
        }
    }
    let message_id = transport.create_disabled(channel_id).await?;
    let location = Location {
        guild_id,
        channel_id,
        message_id,
        application_id,
    };
    if let Err(error) = store::register(pool, location).await {
        // An orphan remains disabled and unregistered even if cleanup also fails.
        let _ = transport.delete(location).await;
        return Err(error.into());
    }
    let enabled = transport.update(location, true).await.is_ok();
    let old_disabled = match old.filter(|p| *p != location && p.application_id == application_id) {
        Some(old) => transport.update(old, false).await.is_ok(),
        None => true,
    };
    Ok(Managed {
        location,
        enabled,
        old_disabled,
    })
}
