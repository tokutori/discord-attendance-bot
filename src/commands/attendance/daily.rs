use chrono::Utc;

use crate::{
    Context, Error,
    attendance::{self, ContinueOutcome, EndOutcome, StartOutcome},
    time::{self, format_datetime, format_duration},
};

use super::common::{defer_ephemeral, ids, refresh_status_activity, send_response};

fn now_and_optional_time(at: Option<&str>) -> Result<(i64, i64), Error> {
    let now = Utc::now();
    Ok((
        now.timestamp(),
        at.map(|value| time::parse_today_time(value, now))
            .transpose()?
            .unwrap_or(now.timestamp()),
    ))
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
        StartOutcome::Started(session) => format!(
            "活動を開始した。\n開始時刻: {}\n記録ID: #{}",
            format_datetime(session.started_at),
            session.id
        ),
        StartOutcome::AlreadyActive(session) => {
            let mut text = format!(
                "すでに活動中である。\n\n開始時刻: {}\n経過時間: {}",
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
    };
    refresh_status_activity(ctx, "start").await;
    send_response(ctx, content).await
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
        EndOutcome::Ended(session) => format!(
            "活動を終了した。\n開始時刻: {}\n終了時刻: {}\n活動時間: {}\n記録ID: #{}",
            format_datetime(session.started_at),
            format_datetime(session.ended_at.unwrap()),
            format_duration(session.duration_seconds_at(now)),
            session.id
        ),
        EndOutcome::AutoEndedCorrected {
            session,
            automatic_end,
        } => format!(
            "活動を終了した。\n自動終了（{}）を取り消し、入力された終了時刻を正として扱った。\n開始時刻: {}\n終了時刻: {}\n活動時間: {}\n記録ID: #{}",
            format_datetime(automatic_end),
            format_datetime(session.started_at),
            format_datetime(session.ended_at.unwrap()),
            format_duration(session.duration_seconds_at(now)),
            session.id
        ),
        EndOutcome::AlreadyInactive(Some(session)) => format!(
            "現在、活動中の記録はない。\n\n直近の活動:\n{} ～ {}\n活動時間: {}\n記録ID: #{}",
            format_datetime(session.started_at),
            format_datetime(session.ended_at.unwrap()),
            format_duration(session.duration_seconds_at(now)),
            session.id
        ),
        EndOutcome::AlreadyInactive(None) => {
            "現在、活動中の記録はない。\n過去の活動記録も存在しない。".into()
        }
    };
    refresh_status_activity(ctx, "end").await;
    send_response(ctx, content).await
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
        ContinueOutcome::AlreadyActive(session) => format!(
            "すでに活動中である。\n\n開始時刻: {}\n経過時間: {}\n記録ID: #{}",
            format_datetime(session.started_at),
            format_duration(session.duration_seconds_at(now)),
            session.id
        ),
        ContinueOutcome::NothingToContinue => "継続できる直近の活動記録がない。".into(),
    };
    refresh_status_activity(ctx, "continue").await;
    send_response(ctx, content).await
}
