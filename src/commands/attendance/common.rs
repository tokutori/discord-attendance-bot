use chrono::Utc;
use poise::CreateReply;

use crate::{Context, Error, channel_status, presentation, repository, time::format_datetime};

pub(super) fn ids(ctx: Context<'_>) -> Result<(i64, i64), Error> {
    let guild = ctx
        .guild_id()
        .ok_or_else(|| anyhow::anyhow!("このコマンドはサーバー内でのみ使用できる"))?;
    Ok((
        i64::try_from(guild.get())?,
        i64::try_from(ctx.author().id.get())?,
    ))
}

pub(super) async fn take_auto_end_notice(ctx: Context<'_>) -> Result<Option<String>, Error> {
    let (guild_id, user_id) = ids(ctx)?;
    let Some(notice) = repository::take_auto_end_notice(
        &ctx.data().database,
        guild_id,
        user_id,
        Utc::now().timestamp(),
    )
    .await?
    else {
        return Ok(None);
    };
    let message = if notice.corrected_at.is_some() {
        format!(
            "前回の終了忘れにより、記録 #{} は {} に自動終了として扱われていた。今回の入力で自動終了を取り消し、ユーザー入力を正として扱った。",
            notice.session_id,
            format_datetime(notice.automatic_ended_at)
        )
    } else {
        format!(
            "前回の終了忘れにより、記録 #{} は {} に自動終了として扱った。実際の終了時刻が異なる場合は `/attendance edit record:{}` で修正してほしい。",
            notice.session_id,
            format_datetime(notice.automatic_ended_at),
            notice.session_id
        )
    };
    Ok(Some(message))
}

pub(super) async fn send_response(
    ctx: Context<'_>,
    content: impl Into<String>,
) -> Result<(), Error> {
    let mut content = content.into();
    if let Some(notice) = take_auto_end_notice(ctx).await? {
        content.push_str(&format!("\n\n【自動終了のお知らせ】\n{notice}"));
    }
    ctx.send(
        CreateReply::default()
            .embed(presentation::response_embed("活動時間記録", content))
            .ephemeral(true),
    )
    .await?;
    Ok(())
}

pub(super) async fn defer_ephemeral(ctx: Context<'_>) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    Ok(())
}

pub(super) async fn refresh_status_activity(ctx: Context<'_>, reason: &'static str) {
    let Some(guild_id) = ctx.guild_id() else {
        return;
    };
    match channel_status::refresh_activity(
        ctx.serenity_context(),
        &ctx.data().database,
        guild_id.get() as i64,
    )
    .await
    {
        Ok(active_count) => {
            tracing::info!(
                reason,
                guild_id = guild_id.get(),
                active_count,
                "updated attendance activity"
            );
        }
        Err(error) => {
            tracing::warn!(%error, reason, guild_id = guild_id.get(), "failed to update attendance activity");
        }
    }
}
