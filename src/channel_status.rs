use std::time::Duration;

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
