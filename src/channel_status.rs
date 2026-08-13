use std::{sync::OnceLock, time::Duration};

use anyhow::Context as _;
use chrono::Utc;
use poise::serenity_prelude as serenity;
use sqlx::SqlitePool;

use crate::{
    attendance::AttendanceSession,
    presentation::{activity_status, status_topic},
    repository,
};

const TOPIC_REFRESH_INTERVAL: Duration = Duration::from_secs(600);
const AUTO_END_RETRY_DELAY: Duration = Duration::from_secs(60);
const STARTUP_RETRY_INITIAL_DELAY: Duration = Duration::from_secs(1);
static STATUS_REFRESH_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();

fn status_refresh_lock() -> &'static tokio::sync::Mutex<()> {
    STATUS_REFRESH_LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

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

async fn load_active_sessions(
    pool: &SqlitePool,
    guild_id: i64,
) -> anyhow::Result<Vec<AttendanceSession>> {
    tracing::info!(guild_id, "loading active attendance sessions");
    let sessions = tokio::time::timeout(
        Duration::from_secs(10),
        repository::active_sessions(pool, guild_id),
    )
    .await
    .context("timed out loading active attendance sessions")??;
    tracing::info!(
        guild_id,
        active_count = sessions.len(),
        "loaded active attendance sessions"
    );
    Ok(sessions)
}

pub async fn refresh_activity(
    ctx: &serenity::Context,
    pool: &SqlitePool,
    guild_id: i64,
) -> anyhow::Result<usize> {
    let _guard = status_refresh_lock().lock().await;
    let sessions = load_active_sessions(pool, guild_id).await?;
    let activity = activity_status(&sessions, Utc::now().timestamp());
    ctx.set_activity(Some(serenity::ActivityData::watching(activity)));
    Ok(sessions.len())
}

pub async fn refresh_status(
    ctx: &serenity::Context,
    pool: &SqlitePool,
    guild_id: i64,
    channel_id: u64,
) -> anyhow::Result<usize> {
    let _guard = status_refresh_lock().lock().await;
    let sessions = load_active_sessions(pool, guild_id).await?;
    let activity = activity_status(&sessions, Utc::now().timestamp());
    ctx.set_activity(Some(serenity::ActivityData::watching(activity)));

    let topic = status_topic(&sessions, Utc::now().timestamp());
    tracing::info!(
        guild_id,
        channel_id,
        "sending attendance status topic update"
    );
    tokio::time::timeout(
        Duration::from_secs(10),
        serenity::ChannelId::new(channel_id).edit(ctx, serenity::EditChannel::new().topic(topic)),
    )
    .await
    .context("timed out updating attendance status channel topic")??;
    tracing::info!(guild_id, channel_id, "sent attendance status topic update");
    Ok(sessions.len())
}

pub fn spawn_periodic_refresh(
    ctx: &serenity::Context,
    pool: SqlitePool,
    guild_id: i64,
    channel_id: u64,
) {
    let ctx = ctx.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(TOPIC_REFRESH_INTERVAL);
        loop {
            interval.tick().await;
            tracing::info!(
                guild_id,
                channel_id,
                "started periodic attendance status refresh"
            );
            match refresh_status(&ctx, &pool, guild_id, channel_id).await {
                Ok(active_count) => {
                    tracing::info!(
                        guild_id,
                        channel_id,
                        active_count,
                        "completed periodic attendance status refresh"
                    );
                }
                Err(error) => {
                    tracing::warn!(
                        %error,
                        guild_id,
                        channel_id,
                        "failed periodic attendance status refresh"
                    );
                }
            }
        }
    });
}

pub fn spawn_auto_end_scheduler(ctx: &serenity::Context, pool: SqlitePool, guild_id: i64) {
    let ctx = ctx.clone();
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
            // Refresh even when no row was closed: this also clears an old
            // Discord activity after an external/manual state correction.
            if let Err(error) = refresh_activity(&ctx, &pool, guild_id).await {
                tracing::warn!(%error, guild_id, "failed to refresh activity after automatic end");
            }
        }
    });
}
