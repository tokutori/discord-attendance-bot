use poise::serenity_prelude as serenity;
use sqlx::Error as SqlxError;

use crate::{
    Data,
    attendance::ServiceError,
    presentation,
    text::NameReadingError,
    time::{ParseTimeError, ParseYearMonthError},
};

fn explain_command_error(error: &anyhow::Error) -> (&'static str, String) {
    if let Some(error) = error.downcast_ref::<ServiceError>() {
        return match error {
            ServiceError::EndBeforeStart | ServiceError::FutureTime => (
                "入力形式エラー",
                format!("原因: {error}\n時刻を確認して再試行してほしい。"),
            ),
            ServiceError::OpenSessionConflict | ServiceError::OverlappingSession => (
                "活動記録の競合",
                format!(
                    "原因: {error}\n既存の活動記録を確認し、必要なら `edit` または `revert` で時刻を修正してほしい。"
                ),
            ),
            ServiceError::Invariant(_) => (
                "内部状態エラー",
                "活動記録の内部状態を確認できなかった。管理者に連絡してほしい。".into(),
            ),
            ServiceError::Database(_) => (
                "データベースエラー",
                "原因: 活動記録データベースの読み書きに失敗した。時間を置いて再試行してほしい。繰り返す場合は管理者に連絡してほしい。".into(),
            ),
        };
    }
    if let Some(error) = error.downcast_ref::<ParseTimeError>() {
        return (
            "入力形式エラー",
            format!("原因: {error}\nコマンドのヘルプに記載された形式で入力してほしい。"),
        );
    }
    if let Some(error) = error.downcast_ref::<ParseYearMonthError>() {
        return (
            "入力形式エラー",
            format!("原因: {error}\n対象月は `YYYY-MM` 形式で指定してほしい。例: `2026-08`。"),
        );
    }
    if let Some(error) = error.downcast_ref::<NameReadingError>() {
        return (
            "入力形式エラー",
            format!("原因: {error}\n名前の読みを確認して再試行してほしい。"),
        );
    }
    if error.downcast_ref::<SqlxError>().is_some() {
        return (
            "データベースエラー",
            "原因: 活動記録データベースの読み書きに失敗した。時間を置いて再試行してほしい。繰り返す場合は管理者に連絡してほしい。".into(),
        );
    }

    // Discord/HTTP libraries expose transport failures as external error
    // types. Their stable typed variants differ by library version, so retain
    // a narrow fallback for those boundary messages only.
    let message = error.to_string();
    let lowercase = message.to_ascii_lowercase();
    if lowercase.contains("request entity too large") || lowercase.contains("payload too large") {
        return (
            "添付ファイルが大きすぎる",
            "原因: Discord が CSV または PDF の添付をサイズ超過として受け付けなかった。`preview` と `publish` では添付サイズは変わらない。\n対処: 管理者は `ATTENDANCE_PDF_FONT_PATH` に軽量な日本語 TTF またはサブセット済みフォントを指定してほしい。改善しない場合は対象月の人数・記録数による制限の可能性がある。".into(),
        );
    }
    if message.contains("PDF") || message.contains("フォント") {
        return (
            "PDF生成設定エラー",
            "原因: PDF生成に必要な日本語フォントを読み込めなかった。\n日本語 TTF フォントを `ATTENDANCE_PDF_FONT_PATH` に指定して再試行してほしい。".into(),
        );
    }
    if lowercase.contains("database") || lowercase.contains("sqlite") || lowercase.contains("sqlx")
    {
        return (
            "データベースエラー",
            "原因: 活動記録データベースの読み書きに失敗した。時間を置いて再試行してほしい。繰り返す場合は管理者に連絡してほしい。".into(),
        );
    }
    if lowercase.contains("discord")
        || lowercase.contains("http")
        || lowercase.contains("connection")
    {
        return (
            "Discord通信エラー",
            "原因: Discord との通信に失敗した。時間を置いて再試行してほしい。".into(),
        );
    }
    (
        "処理に失敗した",
        "原因: 内部処理で想定外のエラーが発生した。時間を置いて再試行してほしい。繰り返す場合は管理者に連絡してほしい。".into(),
    )
}

pub async fn handle(
    error: poise::FrameworkError<'_, Data, anyhow::Error>,
) -> Result<(), serenity::Error> {
    use poise::FrameworkError;

    match error {
        FrameworkError::Command { ctx, error, .. } => {
            tracing::error!(%error, "attendance command failed");
            let (title, description) = explain_command_error(&error);
            ctx.send(
                poise::CreateReply::default()
                    .embed(presentation::error_embed(title, description))
                    .ephemeral(true),
            )
            .await?;
        }
        FrameworkError::ArgumentParse { ctx, error, .. } => {
            ctx.send(
                poise::CreateReply::default()
                    .embed(presentation::error_embed(
                        "入力を確認してください",
                        error.to_string(),
                    ))
                    .ephemeral(true),
            )
            .await?;
        }
        FrameworkError::SubcommandRequired { ctx } => {
            ctx.send(
                poise::CreateReply::default()
                    .embed(presentation::response_embed(
                        "サブコマンドが必要",
                        "`/attendance help` または `/attendanceexport help` で利用可能なコマンドを確認できる。",
                    ))
                    .ephemeral(true),
            )
            .await?;
        }
        FrameworkError::CommandPanic { ctx, .. } => {
            tracing::error!("attendance command panicked");
            ctx.send(
                poise::CreateReply::default()
                    .embed(presentation::error_embed(
                        "内部エラー",
                        "予期しないエラーが発生した。時間を置いて再試行してほしい。",
                    ))
                    .ephemeral(true),
            )
            .await?;
        }
        FrameworkError::CooldownHit {
            remaining_cooldown,
            ctx,
            ..
        } => {
            ctx.send(
                poise::CreateReply::default()
                    .embed(presentation::response_embed(
                        "少し待ってください",
                        format!(
                            "{}秒後にもう一度実行してほしい。",
                            remaining_cooldown.as_secs()
                        ),
                    ))
                    .ephemeral(true),
            )
            .await?;
        }
        FrameworkError::MissingBotPermissions {
            missing_permissions,
            ctx,
            ..
        } => {
            ctx.send(
                poise::CreateReply::default()
                    .embed(presentation::error_embed(
                        "Bot の権限が不足",
                        format!("必要な権限: {missing_permissions}"),
                    ))
                    .ephemeral(true),
            )
            .await?;
        }
        FrameworkError::MissingUserPermissions {
            missing_permissions,
            ctx,
            ..
        } => {
            let description = missing_permissions
                .map(|permissions| format!("必要な権限: {permissions}"))
                .unwrap_or_else(|| "必要な権限を確認できなかった。".into());
            ctx.send(
                poise::CreateReply::default()
                    .embed(presentation::error_embed("ユーザー権限が不足", description))
                    .ephemeral(true),
            )
            .await?;
        }
        FrameworkError::GuildOnly { ctx, .. } => {
            ctx.send(
                poise::CreateReply::default()
                    .embed(presentation::error_embed(
                        "サーバー内で実行してください",
                        "このコマンドは Discord サーバー内でのみ使用できる。",
                    ))
                    .ephemeral(true),
            )
            .await?;
        }
        FrameworkError::DmOnly { ctx, .. } => {
            ctx.send(
                poise::CreateReply::default()
                    .embed(presentation::error_embed(
                        "ダイレクトメッセージで実行してください",
                        "このコマンドは DM でのみ使用できる。",
                    ))
                    .ephemeral(true),
            )
            .await?;
        }
        FrameworkError::NsfwOnly { ctx, .. } => {
            ctx.send(
                poise::CreateReply::default()
                    .embed(presentation::error_embed(
                        "NSFW チャンネルで実行してください",
                        "このコマンドは NSFW チャンネルでのみ使用できる。",
                    ))
                    .ephemeral(true),
            )
            .await?;
        }
        other => {
            if let Err(error) = poise::builtins::on_error(other).await {
                tracing::error!(%error, "failed to send framework error response");
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_expected_command_errors() {
        assert_eq!(
            explain_command_error(&anyhow::Error::new(ParseYearMonthError::InvalidFormat)).0,
            "入力形式エラー"
        );
        assert_eq!(
            explain_command_error(&anyhow::anyhow!("Request entity too large")).0,
            "添付ファイルが大きすぎる"
        );
        assert_eq!(
            explain_command_error(&anyhow::Error::new(ServiceError::OverlappingSession)).0,
            "活動記録の競合"
        );
    }
}
