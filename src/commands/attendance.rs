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
        "revert",
        "confirm",
        "status",
        "history",
        "month",
        "edit",
        "delete",
        "help"
    ),
    subcommand_required
)]
pub async fn attendance(_: Context<'_>) -> Result<(), Error> {
    Ok(())
}

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

fn session_snapshot(session: &attendance::AttendanceSession) -> repository::SessionSnapshot {
    repository::SessionSnapshot {
        started_at: session.started_at,
        ended_at: session.ended_at,
        note: session.note.clone(),
        deleted_at: session.deleted_at,
    }
}

fn snapshot_text(started_at: Option<i64>, ended_at: Option<i64>, note: Option<&str>) -> String {
    let started = started_at
        .map(format_datetime)
        .unwrap_or_else(|| "記録なし".into());
    let ended = ended_at
        .map(format_datetime)
        .unwrap_or_else(|| "活動中".into());
    let note = note
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("なし");
    format!("開始: {started}\n終了: {ended}\n備考: {note}")
}

fn confirmation_notice(request: &repository::ConfirmationRequest, details: String) -> String {
    format!(
        "{details}\n\nこの時点では DB を変更していない。\n確認ID: `{}`\n有効期限: {}\n確定するには `/attendance confirm id:{}` を実行する。",
        request.code,
        format_datetime(request.expires_at),
        request.code
    )
}

async fn take_auto_end_notice(ctx: Context<'_>) -> Result<Option<String>, Error> {
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

async fn send_response(ctx: Context<'_>, content: impl Into<String>) -> Result<(), Error> {
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
    send_response(ctx, content).await?;
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
        EndOutcome::AutoEndedCorrected {
            session: s,
            automatic_end,
        } => format!(
            "活動を終了した。\n自動終了（{}）を取り消し、入力された終了時刻を正として扱った。\n開始時刻: {}\n終了時刻: {}\n活動時間: {}\n記録ID: #{}",
            format_datetime(automatic_end),
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
    send_response(ctx, content).await?;
    Ok(())
}

async fn undo_last_end(ctx: Context<'_>, reason: &'static str) -> Result<(), Error> {
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
    refresh_status_activity(ctx, reason).await;
    send_response(ctx, content).await?;
    Ok(())
}

#[poise::command(slash_command, guild_only, rename = "continue")]
pub async fn continue_activity(ctx: Context<'_>) -> Result<(), Error> {
    undo_last_end(ctx, "continue").await
}

#[poise::command(slash_command, guild_only)]
pub async fn revert(ctx: Context<'_>) -> Result<(), Error> {
    defer_ephemeral(ctx).await?;
    let (guild_id, user_id) = ids(ctx)?;
    let now = Utc::now().timestamp();
    let preview = match repository::latest_revert_preview(&ctx.data().database, guild_id, user_id)
        .await?
    {
        repository::RevertPreviewResult::Available(preview) => preview,
        repository::RevertPreviewResult::NothingToRevert => {
            return send_response(
                ctx,
                "取り消せる成功した変更操作がない。revert は実行されなかった。",
            )
            .await;
        }
        repository::RevertPreviewResult::Conflict => {
            return send_response(
                ctx,
                "最新の操作後の状態と現在の記録が一致しないため、安全のため revert 要求を作成しなかった。",
            )
            .await;
        }
    };
    let expected = preview.current.clone();
    let request = repository::create_confirmation(
        &ctx.data().database,
        guild_id,
        user_id,
        repository::ConfirmationInput {
            action: "revert",
            session_id: preview.session_id,
            operation_id: Some(preview.operation_id),
            expected: &expected,
            target_started_at: None,
            target_ended_at: None,
            target_note: None,
            requested_at: now,
        },
    )
    .await?;
    let before = snapshot_text(
        preview.before_started_at,
        preview.before_ended_at,
        preview.before_note.as_deref(),
    );
    let after = snapshot_text(
        Some(preview.current.started_at),
        preview.current.ended_at,
        preview.current.note.as_deref(),
    );
    let details = format!(
        "revert の確認要求を作成した。\n\n対象操作: {}\n対象記録: #{}\n\n取り消し前に戻る状態:\n{}\n\n現在の状態:\n{}",
        preview.operation, preview.session_id, before, after
    );
    let content = confirmation_notice(&request, details);
    send_response(ctx, content).await?;
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
    let mut embed = presentation::month_embed(ctx.author().display_name(), &monthly);
    if let Some(notice) = take_auto_end_notice(ctx).await? {
        embed = embed.field("自動終了のお知らせ", notice, false);
    }
    ctx.send(CreateReply::default().embed(embed).ephemeral(true))
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
        return send_response(
            ctx,
            "start、end、note の少なくとも1つを指定する必要がある。",
        )
        .await;
    }
    let (guild_id, user_id) = ids(ctx)?;
    let Some(existing) =
        repository::get_owned(&ctx.data().database, record, guild_id, user_id).await?
    else {
        return send_response(ctx, "指定した記録が存在しないか、自分の記録ではない。").await;
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
        return send_response(ctx, "終了時刻は開始時刻以降である必要がある。").await;
    }
    if ended_at.is_none()
        && let Some(open) =
            repository::open_session(&ctx.data().database, guild_id, user_id).await?
        && open.id != record
    {
        return send_response(
            ctx,
            format!(
                "別の活動記録 #{} が活動中であるため、この記録を活動中には戻せない。",
                open.id
            ),
        )
        .await;
    }
    let new_note = match note.as_deref() {
        Some("") => None,
        Some(v) => Some(v),
        None => existing.note.as_deref(),
    };
    let expected = session_snapshot(&existing);
    let request = repository::create_confirmation(
        &ctx.data().database,
        guild_id,
        user_id,
        repository::ConfirmationInput {
            action: "edit",
            session_id: record,
            operation_id: None,
            expected: &expected,
            target_started_at: Some(started_at),
            target_ended_at: ended_at,
            target_note: new_note,
            requested_at: Utc::now().timestamp(),
        },
    )
    .await?;
    let details = format!(
        "edit の確認要求を作成した。\n\n対象記録: #{}\n\n変更前:\n{}\n\n変更後:\n{}",
        record,
        snapshot_text(
            Some(existing.started_at),
            existing.ended_at,
            existing.note.as_deref(),
        ),
        snapshot_text(Some(started_at), ended_at, new_note),
    );
    send_response(ctx, confirmation_notice(&request, details)).await
}

#[poise::command(slash_command, guild_only)]
pub async fn delete(
    ctx: Context<'_>,
    #[description = "削除する記録ID"] record: i64,
) -> Result<(), Error> {
    defer_ephemeral(ctx).await?;
    let (guild_id, user_id) = ids(ctx)?;
    let Some(existing) =
        repository::get_owned(&ctx.data().database, record, guild_id, user_id).await?
    else {
        return send_response(ctx, "指定した記録が存在しないか、自分の記録ではない。").await;
    };
    let expected = session_snapshot(&existing);
    let request = repository::create_confirmation(
        &ctx.data().database,
        guild_id,
        user_id,
        repository::ConfirmationInput {
            action: "delete",
            session_id: record,
            operation_id: None,
            expected: &expected,
            target_started_at: None,
            target_ended_at: None,
            target_note: None,
            requested_at: Utc::now().timestamp(),
        },
    )
    .await?;
    let details = format!(
        "delete の確認要求を作成した。\n\n対象記録: #{}\n\n削除対象の内容:\n{}\n\n確定すると soft delete され、通常の履歴・集計から除外される。",
        record,
        snapshot_text(
            Some(existing.started_at),
            existing.ended_at,
            existing.note.as_deref(),
        )
    );
    send_response(ctx, confirmation_notice(&request, details)).await
}

#[poise::command(slash_command, guild_only)]
pub async fn confirm(
    ctx: Context<'_>,
    #[description = "確認ID（5文字）"] id: String,
) -> Result<(), Error> {
    defer_ephemeral(ctx).await?;
    let code = id.trim().to_ascii_uppercase();
    if code.len() != 5
        || !code
            .chars()
            .all(|character| character.is_ascii_alphanumeric())
    {
        return send_response(ctx, "確認IDは5文字の英数字で指定する必要がある。").await;
    }
    let (guild_id, user_id) = ids(ctx)?;
    let result = repository::confirm_confirmation(
        &ctx.data().database,
        guild_id,
        user_id,
        &code,
        Utc::now().timestamp(),
    )
    .await?;
    let changed = matches!(&result, repository::ConfirmationResult::Confirmed { .. });
    let content = match result {
        repository::ConfirmationResult::Confirmed {
            action,
            session_id,
            operation,
        } => match action.as_str() {
            "edit" => format!(
                "edit を確定した。\n\n記録 #{} をプレビューどおり修正した。",
                session_id
            ),
            "delete" => format!(
                "delete を確定した。\n\n記録 #{} を soft delete した。通常の履歴・集計から除外される。",
                session_id
            ),
            "revert" => format!(
                "revert を確定した。\n\n直前の {} 操作を取り消し、記録 #{} をプレビューどおり復元した。",
                operation.unwrap_or_else(|| "変更".into()),
                session_id
            ),
            _ => "確認した操作を確定した。".into(),
        },
        repository::ConfirmationResult::NotFound => {
            "確認IDが存在しないか、すでに使用済みである。プレビューからやり直してほしい。".into()
        }
        repository::ConfirmationResult::Expired => {
            "確認IDの有効期限が切れている。プレビューからやり直してほしい。".into()
        }
        repository::ConfirmationResult::Conflict => {
            "プレビュー後に記録の状態が変わったため、安全のため確定しなかった。最新状態を確認してからやり直してほしい。".into()
        }
    };
    if changed {
        refresh_status_activity(ctx, "confirm").await;
    }
    send_response(ctx, content).await
}
