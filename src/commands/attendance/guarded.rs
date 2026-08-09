use chrono::Utc;

use crate::{
    Context, Error, attendance, repository,
    time::{self, format_datetime},
};

use super::common::{defer_ephemeral, ids, refresh_status_activity, send_response};

fn session_snapshot(session: &attendance::AttendanceSession) -> repository::SessionSnapshot {
    repository::SessionSnapshot {
        started_at: session.started_at,
        ended_at: session.ended_at,
        open_since: session.open_since,
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

/// 直前の成功した変更を確認付きで取り消す。
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
            change_id: Some(preview.change_id),
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
    send_response(ctx, confirmation_notice(&request, details)).await
}

/// 活動記録を確認付きで修正する。
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
    let now = Utc::now().timestamp();
    if started_at > now || ended_at.is_some_and(|value| value > now) {
        return send_response(ctx, "未来の日時には修正できない。").await;
    }
    if ended_at.is_some_and(|value| value < started_at) {
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
        Some(value) => Some(value),
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
            change_id: None,
            expected: &expected,
            target_started_at: Some(started_at),
            target_ended_at: ended_at,
            target_note: new_note,
            requested_at: now,
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

/// 活動記録を確認付きで削除する。
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
            change_id: None,
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

/// 発行された確認IDで特殊操作を確定する。
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
