use std::{sync::OnceLock, time::Duration};

use anyhow::Context as _;
use attendance_query::ReadDatabase;
use chrono::Utc;
use poise::serenity_prelude as serenity;

use crate::{
    attendance::AttendanceSession,
    config::{StatusConfig, StatusMode},
    presentation::{activity_status, status_topic},
    repository,
};

static STATUS_REFRESH_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();

fn status_refresh_lock() -> &'static tokio::sync::Mutex<()> {
    STATUS_REFRESH_LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

async fn load_active_sessions(
    pool: &ReadDatabase,
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
    pool: &ReadDatabase,
    guild_id: i64,
    mode: StatusMode,
) -> anyhow::Result<usize> {
    if !mode.is_enabled() {
        ctx.set_activity(None);
        return Ok(0);
    }
    let _guard = status_refresh_lock().lock().await;
    let sessions = load_active_sessions(pool, guild_id).await?;
    let activity = activity_status(&sessions, Utc::now().timestamp(), mode.shows_names());
    ctx.set_activity(Some(serenity::ActivityData::watching(activity)));
    Ok(sessions.len())
}

pub async fn refresh_status(
    ctx: &serenity::Context,
    pool: &ReadDatabase,
    guild_id: i64,
    status: StatusConfig,
) -> anyhow::Result<usize> {
    if !status.mode.is_enabled() {
        ctx.set_activity(None);
        return Ok(0);
    }
    let channel_id = status
        .channel_id
        .context("status channel is not configured")?;
    let _guard = status_refresh_lock().lock().await;
    let sessions = load_active_sessions(pool, guild_id).await?;
    let activity = activity_status(&sessions, Utc::now().timestamp(), status.mode.shows_names());
    ctx.set_activity(Some(serenity::ActivityData::watching(activity)));

    let topic = status_topic(&sessions, Utc::now().timestamp(), status.mode.shows_names());
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
    pool: ReadDatabase,
    guild_id: i64,
    status: StatusConfig,
) {
    let ctx = ctx.clone();
    tokio::spawn(async move {
        let mut interval =
            tokio::time::interval(Duration::from_secs(status.refresh_interval_seconds));
        loop {
            interval.tick().await;
            let channel_id = status.channel_id.unwrap_or_default();
            tracing::info!(
                guild_id,
                channel_id,
                "started periodic attendance status refresh"
            );
            match refresh_status(&ctx, &pool, guild_id, status).await {
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
