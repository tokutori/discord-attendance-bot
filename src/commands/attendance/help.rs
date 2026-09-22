use super::common::defer_ephemeral;
use crate::{Context, Error, presentation};
use poise::CreateReply;
/// 記録コマンドの使い方を表示する。
#[poise::command(slash_command, guild_only)]
pub async fn help(ctx: Context<'_>) -> Result<(), Error> {
    defer_ephemeral(ctx).await?;
    ctx.send(
        CreateReply::default()
            .embed(presentation::help_embed(ctx.author().display_name()))
            .ephemeral(true),
    )
    .await?;
    Ok(())
}
