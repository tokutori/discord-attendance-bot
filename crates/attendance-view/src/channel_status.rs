use std::{future::Future, sync::OnceLock, time::Duration};

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
    // Always use HTTP: a cached channel cannot establish the current destination.
    refresh_after_channel_check(
        guild_id,
        channel_id,
        async {
            Ok(ctx
                .http
                .get_channel(serenity::ChannelId::new(channel_id))
                .await?)
        },
        || async {
            let sessions = load_active_sessions(pool, guild_id).await?;
            let activity =
                activity_status(&sessions, Utc::now().timestamp(), status.mode.shows_names());
            ctx.set_activity(Some(serenity::ActivityData::watching(activity)));

            let topic = status_topic(&sessions, Utc::now().timestamp(), status.mode.shows_names());
            tracing::info!(
                guild_id,
                channel_id,
                "sending attendance status topic update"
            );
            tokio::time::timeout(
                Duration::from_secs(10),
                serenity::ChannelId::new(channel_id)
                    .edit(ctx, serenity::EditChannel::new().topic(topic)),
            )
            .await
            .context("timed out updating attendance status channel topic")??;
            tracing::info!(guild_id, channel_id, "sent attendance status topic update");
            Ok(sessions.len())
        },
    )
    .await
}

// Keep every DB read and publication inside the checked continuation. This also
// lets tests prove that failed destination lookup never reaches those effects.
async fn refresh_after_channel_check<L, R, RF>(
    guild_id: i64,
    channel_id: u64,
    lookup: L,
    refresh: R,
) -> anyhow::Result<usize>
where
    L: Future<Output = anyhow::Result<serenity::Channel>>,
    R: FnOnce() -> RF,
    RF: Future<Output = anyhow::Result<usize>>,
{
    let channel = tokio::time::timeout(Duration::from_secs(10), lookup)
        .await
        .context("timed out verifying attendance status channel")??;
    let serenity::Channel::Guild(channel) = channel else {
        anyhow::bail!("attendance status channel must be a Guild channel");
    };
    anyhow::ensure!(
        guild_id > 0 && channel.guild_id.get() == guild_id as u64 && channel.id.get() == channel_id,
        "attendance status channel does not belong to the configured Guild"
    );
    anyhow::ensure!(
        matches!(
            channel.kind,
            serenity::ChannelType::Text
                | serenity::ChannelType::News
                | serenity::ChannelType::Forum
        ),
        "attendance status channel must support a topic (text, announcement or forum)"
    );
    refresh().await
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn channel(guild: u64, id: u64, kind: serenity::ChannelType) -> serenity::Channel {
        let mut channel = serenity::GuildChannel::default();
        channel.guild_id = serenity::GuildId::new(guild);
        channel.id = serenity::ChannelId::new(id);
        channel.kind = kind;
        serenity::Channel::Guild(channel)
    }

    #[tokio::test]
    async fn verifies_each_refresh_before_any_data_access_or_publication() {
        let events = Mutex::new(Vec::new());
        for kind in [
            serenity::ChannelType::Text,
            serenity::ChannelType::News,
            serenity::ChannelType::Forum,
        ] {
            let count = refresh_after_channel_check(
                10,
                20,
                async {
                    events.lock().unwrap().push("lookup");
                    Ok(channel(10, 20, kind))
                },
                || async {
                    events.lock().unwrap().extend(["load", "activity", "topic"]);
                    Ok(3)
                },
            )
            .await
            .unwrap();
            assert_eq!(count, 3);
        }
        assert_eq!(
            *events.lock().unwrap(),
            ["lookup", "load", "activity", "topic"].repeat(3)
        );
        // A later lookup returning another Guild must not reuse earlier approval.
        let before = events.lock().unwrap().len();
        assert!(
            refresh_after_channel_check(
                10,
                20,
                async { Ok(channel(11, 20, serenity::ChannelType::Text)) },
                || async {
                    events.lock().unwrap().push("unexpected effect");
                    Ok(0)
                }
            )
            .await
            .is_err()
        );
        assert_eq!(events.lock().unwrap().len(), before);
    }

    #[tokio::test]
    async fn rejects_wrong_guild_id_kind_private_channel_and_lookup_errors() {
        let mut invalid = vec![
            channel(11, 20, serenity::ChannelType::Text),
            channel(10, 21, serenity::ChannelType::Text),
            serenity::Channel::Private(serenity::PrivateChannel::default()),
        ];
        for kind in [
            serenity::ChannelType::Voice,
            serenity::ChannelType::Category,
            serenity::ChannelType::Stage,
            serenity::ChannelType::PublicThread,
            serenity::ChannelType::PrivateThread,
            serenity::ChannelType::NewsThread,
            serenity::ChannelType::Directory,
            serenity::ChannelType::Unknown(99),
        ] {
            invalid.push(channel(10, 20, kind));
        }
        for value in invalid {
            assert!(
                refresh_after_channel_check(10, 20, async { Ok(value) }, || async {
                    panic!("must not read DB or publish activity/topic")
                })
                .await
                .is_err()
            );
        }
        for reason in ["not found", "forbidden", "transport failure"] {
            assert!(
                refresh_after_channel_check(10, 20, async { anyhow::bail!(reason) }, || async {
                    panic!("must not read DB or publish activity/topic")
                })
                .await
                .is_err()
            );
        }
    }

    #[tokio::test]
    async fn lookup_timeout_prevents_all_refresh_effects() {
        let error = refresh_after_channel_check(10, 20, std::future::pending(), || async {
            panic!("must not read DB or publish activity/topic")
        })
        .await
        .unwrap_err();
        assert!(error.to_string().contains("timed out verifying"));
    }
}
