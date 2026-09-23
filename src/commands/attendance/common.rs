use chrono::Utc;
use poise::CreateReply;

use crate::{Context, Error, presentation, repository};

pub(super) fn ids(ctx: Context<'_>) -> Result<(i64, i64), Error> {
    let guild =
        crate::config::require_guild(ctx.data().guild_id, ctx.guild_id().map(|guild| guild.get()))?;
    Ok((guild, i64::try_from(ctx.author().id.get())?))
}

pub(super) struct PendingAutoEndNotice {
    pub event_id: i64,
    pub message: String,
}

pub(super) async fn peek_auto_end_notice(
    ctx: Context<'_>,
) -> Result<Option<PendingAutoEndNotice>, Error> {
    let (guild_id, user_id) = ids(ctx)?;
    let Some(notice) =
        repository::peek_auto_end_notice(&ctx.data().database, guild_id, user_id).await?
    else {
        return Ok(None);
    };
    let message = presentation::auto_end_notice_text(&notice);
    Ok(Some(PendingAutoEndNotice {
        event_id: notice.event_id,
        message,
    }))
}

pub(super) async fn acknowledge_auto_end_notice(
    ctx: Context<'_>,
    event_id: i64,
) -> Result<(), Error> {
    let (guild_id, user_id) = ids(ctx)?;
    repository::acknowledge_auto_end_notice(
        &ctx.data().database,
        event_id,
        guild_id,
        user_id,
        Utc::now().timestamp(),
    )
    .await?;
    Ok(())
}

pub(super) async fn send_response(
    ctx: Context<'_>,
    content: impl Into<String>,
) -> Result<(), Error> {
    let mut content = content.into();
    let notice = peek_auto_end_notice(ctx).await?;
    if let Some(notice) = &notice {
        content.push_str(&format!("\n\n【自動終了のお知らせ】\n{}", notice.message));
    }
    ctx.send(
        CreateReply::default()
            .embed(presentation::response_embed("活動時間記録", content))
            .ephemeral(true),
    )
    .await?;
    if let Some(notice) = notice
        && let Err(error) = acknowledge_auto_end_notice(ctx, notice.event_id).await
    {
        tracing::warn!(
            %error,
            event_id = notice.event_id,
            "response sent but failed to acknowledge automatic-end notice"
        );
    }
    Ok(())
}

pub(super) async fn defer_ephemeral(ctx: Context<'_>) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    Ok(())
}
