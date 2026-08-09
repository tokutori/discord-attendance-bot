use chrono::{Datelike, Utc};
use poise::CreateReply;

use crate::{
    Context, Error, attendance, presentation, repository,
    time::{self, DISPLAY_TIMEZONE, format_datetime, format_duration},
};

use super::common::{defer_ephemeral, ids, send_response, take_auto_end_notice};

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

#[poise::command(slash_command, guild_only)]
pub async fn status(ctx: Context<'_>) -> Result<(), Error> {
    defer_ephemeral(ctx).await?;
    let (guild_id, user_id) = ids(ctx)?;
    let now = Utc::now().timestamp();
    let content = match repository::open_session(&ctx.data().database, guild_id, user_id).await? {
        Some(session) => {
            let mut text = format!(
                "活動中\n\n開始時刻: {}\n経過時間: {}",
                format_datetime(session.started_at),
                format_duration(session.duration_seconds_at(now))
            );
            if let Some(note) = session
                .note
                .as_deref()
                .filter(|value| !value.trim().is_empty())
            {
                text.push_str(&format!("\n備考: {note}"));
            }
            text.push_str(&format!("\n記録ID: #{}", session.id));
            text
        }
        None => "現在、活動中の記録はない。".into(),
    };
    send_response(ctx, content).await
}

#[poise::command(slash_command, guild_only)]
pub async fn history(
    ctx: Context<'_>,
    #[description = "表示件数（1〜20、既定値5）"]
    #[min = 1]
    #[max = 20]
    limit: Option<i64>,
) -> Result<(), Error> {
    defer_ephemeral(ctx).await?;
    let (guild_id, user_id) = ids(ctx)?;
    let sessions =
        repository::history(&ctx.data().database, guild_id, user_id, limit.unwrap_or(5)).await?;
    let mut embed = presentation::history_embed(
        ctx.author().display_name(),
        &sessions,
        Utc::now().timestamp(),
    );
    if let Some(notice) = take_auto_end_notice(ctx).await? {
        embed = embed.field("自動終了のお知らせ", notice, false);
    }
    ctx.send(CreateReply::default().embed(embed).ephemeral(true))
        .await?;
    Ok(())
}

#[poise::command(slash_command, guild_only)]
pub async fn month(
    ctx: Context<'_>,
    #[description = "対象月（YYYY-MM）。省略時は当月"] target: Option<String>,
) -> Result<(), Error> {
    defer_ephemeral(ctx).await?;
    let (guild_id, user_id) = ids(ctx)?;
    let now = Utc::now();
    let year_month = match target {
        Some(value) => time::parse_year_month(&value)?,
        None => {
            let local_now = now.with_timezone(&DISPLAY_TIMEZONE);
            attendance::YearMonth {
                year: local_now.year(),
                month: local_now.month(),
            }
        }
    };
    let (start, end) = time::month_bounds(year_month)?;
    let sessions =
        repository::overlapping_completed(&ctx.data().database, guild_id, user_id, start, end)
            .await?;
    let monthly = attendance::aggregate_monthly(&sessions, year_month, now)?;
    let mut embed = presentation::month_embed(ctx.author().display_name(), &monthly);
    if let Some(notice) = take_auto_end_notice(ctx).await? {
        embed = embed.field("自動終了のお知らせ", notice, false);
    }
    ctx.send(CreateReply::default().embed(embed).ephemeral(true))
        .await?;
    Ok(())
}
