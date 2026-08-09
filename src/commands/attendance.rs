use chrono::{Datelike, Utc};
use poise::CreateReply;

use crate::{
    Context, Error,
    attendance::{self, ContinueOutcome, EndOutcome, StartOutcome},
    channel_status, presentation, repository,
    time::{self, DISPLAY_TIMEZONE, format_datetime, format_duration},
};

#[poise::command(
    slash_command,
    subcommands(
        "start",
        "end",
        "continue_activity",
        "status",
        "history",
        "month",
        "edit",
        "delete"
    ),
    subcommand_required
)]
pub async fn attendance(_: Context<'_>) -> Result<(), Error> {
    Ok(())
}

fn ids(ctx: Context<'_>) -> Result<(i64, i64), Error> {
    let guild = ctx
        .guild_id()
        .ok_or_else(|| anyhow::anyhow!("このコマンドはサーバー内でのみ使用できる"))?;
    Ok((
        i64::try_from(guild.get())?,
        i64::try_from(ctx.author().id.get())?,
    ))
}

fn now_and_optional_time(at: Option<&str>) -> Result<(i64, i64), Error> {
    let now = Utc::now();
    Ok((
        now.timestamp(),
        at.map(|x| time::parse_today_time(x, now))
            .transpose()?
            .unwrap_or(now.timestamp()),
    ))
}

async fn send_text(ctx: Context<'_>, content: impl Into<String>) -> Result<(), Error> {
    ctx.send(CreateReply::default().content(content).ephemeral(true))
        .await?;
    Ok(())
}

async fn defer_ephemeral(ctx: Context<'_>) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    Ok(())
}

async fn refresh_status_activity(ctx: Context<'_>, reason: &'static str) {
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

#[poise::command(slash_command, guild_only)]
pub async fn start(
    ctx: Context<'_>,
    #[description = "開始時刻（HH:MM）。省略時は現在時刻"] at: Option<String>,
    #[description = "活動内容の備考"]
    #[max_length = 500]
    note: Option<String>,
) -> Result<(), Error> {
    defer_ephemeral(ctx).await?;
    let (guild_id, user_id) = ids(ctx)?;
    let (now, started_at) = now_and_optional_time(at.as_deref())?;
    let outcome = attendance::start(
        &ctx.data().database,
        guild_id,
        user_id,
        ctx.author().display_name(),
        started_at,
        note.as_deref(),
        now,
    )
    .await?;
    let content = match outcome {
        StartOutcome::Started(s) => format!(
            "活動を開始した。\n開始時刻: {}\n記録ID: #{}",
            format_datetime(s.started_at),
            s.id
        ),
        StartOutcome::AlreadyActive(s) => {
            let mut text = format!(
                "すでに活動中である。\n\n開始時刻: {}\n経過時間: {}",
                format_datetime(s.started_at),
                format_duration(s.duration_seconds_at(now))
            );
            if let Some(note) = s.note.as_deref().filter(|x| !x.trim().is_empty()) {
                text.push_str(&format!("\n備考: {note}"));
            }
            text.push_str(&format!("\n記録ID: #{}", s.id));
            text
        }
    };
    refresh_status_activity(ctx, "start").await;
    send_text(ctx, content).await?;
    Ok(())
}

#[poise::command(slash_command, guild_only, rename = "end")]
pub async fn end(
    ctx: Context<'_>,
    #[description = "終了時刻（HH:MM）。省略時は現在時刻"] at: Option<String>,
    #[description = "終了時に設定する備考"]
    #[max_length = 500]
    note: Option<String>,
) -> Result<(), Error> {
    defer_ephemeral(ctx).await?;
    let (guild_id, user_id) = ids(ctx)?;
    let (now, ended_at) = now_and_optional_time(at.as_deref())?;
    let content = match attendance::end(
        &ctx.data().database,
        guild_id,
        user_id,
        ended_at,
        note.as_deref(),
        now,
    )
    .await?
    {
        EndOutcome::Ended(s) => format!(
            "活動を終了した。\n開始時刻: {}\n終了時刻: {}\n活動時間: {}\n記録ID: #{}",
            format_datetime(s.started_at),
            format_datetime(s.ended_at.unwrap()),
            format_duration(s.duration_seconds_at(now)),
            s.id
        ),
        EndOutcome::AlreadyInactive(Some(s)) => format!(
            "現在、活動中の記録はない。\n\n直近の活動:\n{} ～ {}\n活動時間: {}\n記録ID: #{}",
            format_datetime(s.started_at),
            format_datetime(s.ended_at.unwrap()),
            format_duration(s.duration_seconds_at(now)),
            s.id
        ),
        EndOutcome::AlreadyInactive(None) => {
            "現在、活動中の記録はない。\n過去の活動記録も存在しない。".into()
        }
    };
    refresh_status_activity(ctx, "end").await;
    send_text(ctx, content).await?;
    Ok(())
}

#[poise::command(slash_command, guild_only, rename = "continue")]
pub async fn continue_activity(ctx: Context<'_>) -> Result<(), Error> {
    defer_ephemeral(ctx).await?;
    let (guild_id, user_id) = ids(ctx)?;
    let now = Utc::now().timestamp();
    let content = match attendance::continue_activity(&ctx.data().database, guild_id, user_id, now)
        .await?
    {
        ContinueOutcome::Continued {
            session,
            removed_end,
        } => format!(
            "活動を継続状態に戻した。\n\n開始時刻: {}\n取り消した終了時刻: {}\n現在の経過時間: {}\n記録ID: #{}",
            format_datetime(session.started_at),
            format_datetime(removed_end),
            format_duration(session.duration_seconds_at(now)),
            session.id
        ),
        ContinueOutcome::AlreadyActive(s) => format!(
            "すでに活動中である。\n\n開始時刻: {}\n経過時間: {}\n記録ID: #{}",
            format_datetime(s.started_at),
            format_duration(s.duration_seconds_at(now)),
            s.id
        ),
        ContinueOutcome::NothingToContinue => "継続できる直近の活動記録がない。".into(),
    };
    refresh_status_activity(ctx, "continue").await;
    send_text(ctx, content).await?;
    Ok(())
}

#[poise::command(slash_command, guild_only)]
pub async fn status(ctx: Context<'_>) -> Result<(), Error> {
    defer_ephemeral(ctx).await?;
    let (guild_id, user_id) = ids(ctx)?;
    let now = Utc::now().timestamp();
    let content = match repository::open_session(&ctx.data().database, guild_id, user_id).await? {
        Some(s) => {
            let mut text = format!(
                "活動中\n\n開始時刻: {}\n経過時間: {}",
                format_datetime(s.started_at),
                format_duration(s.duration_seconds_at(now))
            );
            if let Some(note) = s.note.as_deref().filter(|x| !x.trim().is_empty()) {
                text.push_str(&format!("\n備考: {note}"));
            }
            text.push_str(&format!("\n記録ID: #{}", s.id));
            text
        }
        None => "現在、活動中の記録はない。".into(),
    };
    send_text(ctx, content).await
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
    ctx.send(
        CreateReply::default()
            .embed(presentation::history_embed(
                ctx.author().display_name(),
                &sessions,
                Utc::now().timestamp(),
            ))
            .ephemeral(true),
    )
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
    let ym = match target {
        Some(v) => time::parse_year_month(&v)?,
        None => {
            let now = Utc::now().with_timezone(&DISPLAY_TIMEZONE);
            attendance::YearMonth {
                year: now.year(),
                month: now.month(),
            }
        }
    };
    let (start, end) = time::month_bounds(ym)?;
    let sessions =
        repository::overlapping_completed(&ctx.data().database, guild_id, user_id, start, end)
            .await?;
    let monthly = attendance::aggregate_monthly(&sessions, ym)?;
    ctx.send(
        CreateReply::default()
            .embed(presentation::month_embed(
                ctx.author().display_name(),
                &monthly,
            ))
            .ephemeral(true),
    )
    .await?;
    Ok(())
}

#[poise::command(slash_command, guild_only)]
pub async fn edit(
    ctx: Context<'_>,
    #[description = "修正する記録ID"] record: i64,
    #[description = "修正後の開始日時（YYYY-MM-DD HH:MM）"] start: Option<String>,
    #[description = "修正後の終了日時（YYYY-MM-DD HH:MM）"] end: Option<String>,
    #[description = "修正後の備考。空文字で備考を削除"]
    #[max_length = 500]
    note: Option<String>,
) -> Result<(), Error> {
    defer_ephemeral(ctx).await?;
    if start.is_none() && end.is_none() && note.is_none() {
        return send_text(
            ctx,
            "start、end、note の少なくとも1つを指定する必要がある。",
        )
        .await;
    }
    let (guild_id, user_id) = ids(ctx)?;
    let Some(existing) =
        repository::get_owned(&ctx.data().database, record, guild_id, user_id).await?
    else {
        return send_text(ctx, "指定した記録が存在しないか、自分の記録ではない。").await;
    };
    let started_at = start
        .as_deref()
        .map(time::parse_full_datetime)
        .transpose()?
        .unwrap_or(existing.started_at);
    let ended_at = match end.as_deref() {
        Some(value) if value.trim().is_empty() => None,
        Some(value) => Some(time::parse_full_datetime(value)?),
        None => existing.ended_at,
    };
    if ended_at.is_some_and(|e| e < started_at) {
        return send_text(ctx, "終了時刻は開始時刻以降である必要がある。").await;
    }
    if ended_at.is_none() {
        if let Some(open) =
            repository::open_session(&ctx.data().database, guild_id, user_id).await?
        {
            if open.id != record {
                return send_text(
                    ctx,
                    format!(
                        "別の活動記録 #{} が活動中であるため、この記録を活動中には戻せない。",
                        open.id
                    ),
                )
                .await;
            }
        }
    }
    let new_note = match note.as_deref() {
        Some("") => None,
        Some(v) => Some(v),
        None => existing.note.as_deref(),
    };
    repository::update_owned(
        &ctx.data().database,
        record,
        guild_id,
        user_id,
        repository::AttendanceUpdate {
            started_at,
            ended_at,
            note: new_note,
        },
        Utc::now().timestamp(),
    )
    .await?;
    refresh_status_activity(ctx, "edit").await;
    send_text(ctx, format!("記録 #{} を修正した。", record)).await
}

#[poise::command(slash_command, guild_only)]
pub async fn delete(
    ctx: Context<'_>,
    #[description = "削除する記録ID"] record: i64,
    #[description = "削除確認"] confirm: bool,
) -> Result<(), Error> {
    defer_ephemeral(ctx).await?;
    if !confirm {
        return send_text(
            ctx,
            "削除は実行しなかった。実行する場合は confirm:true を指定する。",
        )
        .await;
    }
    let (guild_id, user_id) = ids(ctx)?;
    if repository::soft_delete_owned(
        &ctx.data().database,
        record,
        guild_id,
        user_id,
        Utc::now().timestamp(),
    )
    .await?
        == 0
    {
        send_text(ctx, "指定した記録が存在しないか、自分の記録ではない。").await
    } else {
        refresh_status_activity(ctx, "delete").await;
        send_text(ctx, format!("記録 #{} を削除した。", record)).await
    }
}
