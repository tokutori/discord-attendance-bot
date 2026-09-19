use crate::repository;
use chrono::Utc;
use sqlx::SqlitePool;
use std::time::Duration;
const AUTO_END_RETRY_DELAY: Duration = Duration::from_secs(60);
const STARTUP_RETRY_INITIAL_DELAY: Duration = Duration::from_secs(1);
pub async fn apply_due_auto_ends(pool: &SqlitePool, guild_id: i64) -> anyhow::Result<usize> {
    let now = Utc::now().timestamp();
    let mut attempt = 0_u32;
    let notices = loop {
        match repository::apply_due_auto_ends(pool, guild_id, now).await {
            Ok(notices) => break notices,
            Err(error) if attempt < 3 && is_sqlite_busy(&error) => {
                attempt += 1;
                tracing::warn!(attempt, guild_id, "retrying busy automatic end transaction");
                tokio::time::sleep(Duration::from_millis(250 * u64::from(attempt))).await;
            }
            Err(error) => return Err(error.into()),
        }
    };
    for notice in &notices {
        tracing::info!(
            guild_id,
            session_id = notice.session_id,
            automatic_ended_at = notice.automatic_ended_at,
            "applied automatic attendance end"
        );
    }
    Ok(notices.len())
}

/// Applies every automatic end that is due at the current instant.
///
/// This is the startup recovery boundary: the caller must not expose the bot
/// as ready until this returns successfully. A database outage therefore
/// delays readiness instead of leaving active records unprocessed until the
/// next midnight. The delay is capped so a long outage does not create a hot
/// retry loop.
pub async fn apply_missed_auto_ends(pool: &SqlitePool, guild_id: i64) -> anyhow::Result<usize> {
    let mut delay = STARTUP_RETRY_INITIAL_DELAY;
    loop {
        match apply_due_auto_ends(pool, guild_id).await {
            Ok(count) => return Ok(count),
            Err(error) => {
                tracing::error!(
                    %error,
                    guild_id,
                    retry_seconds = delay.as_secs(),
                    "startup automatic-end recovery failed; retrying before readiness"
                );
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(AUTO_END_RETRY_DELAY);
            }
        }
    }
}

fn is_sqlite_busy(error: &sqlx::Error) -> bool {
    let sqlx::Error::Database(database_error) = error else {
        return false;
    };
    matches!(database_error.code().as_deref(), Some("5" | "6" | "517"))
}

pub fn spawn_auto_end_scheduler(pool: SqlitePool, guild_id: i64) {
    tokio::spawn(async move {
        loop {
            let now = Utc::now();
            let seconds_until_midnight = match crate::time::next_midnight_timestamp(now) {
                Ok(next_midnight) => (next_midnight - now.timestamp()).max(1) as u64,
                Err(error) => {
                    tracing::error!(%error, "failed to calculate next auto-end time");
                    60
                }
            };
            tokio::time::sleep(Duration::from_secs(seconds_until_midnight)).await;
            let count = loop {
                match apply_missed_auto_ends(&pool, guild_id).await {
                    Ok(count) => break count,
                    Err(error) => {
                        tracing::error!(%error, guild_id, "failed midnight automatic attendance end");
                        tracing::warn!(
                            guild_id,
                            delay_seconds = AUTO_END_RETRY_DELAY.as_secs(),
                            "retrying failed midnight automatic attendance end"
                        );
                        tokio::time::sleep(AUTO_END_RETRY_DELAY).await;
                    }
                }
            };
            tracing::info!(
                guild_id,
                count,
                "completed midnight automatic attendance end"
            );
        }
    });
}
