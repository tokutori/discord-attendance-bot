use crate::{Context, Error, presentation, repository};
use poise::CreateReply;

use super::common::{defer_ephemeral, ids, refresh_status_activity};

/// 自分の活動記録・変更履歴・ユーザー設定を完全に削除する。
#[poise::command(slash_command, guild_only)]
pub async fn erase(
    ctx: Context<'_>,
    #[description = "完全削除を実行する場合は DELETE と入力"] confirmation: String,
) -> Result<(), Error> {
    defer_ephemeral(ctx).await?;
    if confirmation.trim() != "DELETE" {
        ctx.send(
            CreateReply::default()
                .embed(presentation::error_embed(
                    "完全削除は実行されなかった",
                    "この操作は取り消せない。自分の全データを完全削除する場合だけ confirmation に `DELETE` と入力してほしい。",
                ))
                .ephemeral(true),
        )
        .await?;
        return Ok(());
    }

    let (guild_id, user_id) = ids(ctx)?;
    let result = repository::purge_user_data(&ctx.data().database, guild_id, user_id).await?;
    refresh_status_activity(ctx, "user data erased").await;
    ctx.send(
        CreateReply::default()
            .embed(presentation::response_embed(
                "個人データを完全削除した",
                format!(
                    "活動記録 {} 件とユーザー設定 {} 件を、このBotの稼働中データベースから完全に削除した。変更履歴・確認要求・自動終了イベントも同時に削除した。\n\n既存のバックアップや、過去にDiscordへ公開したCSV・PDFはこの操作の対象外であるため、必要な場合はサーバー管理者へ削除を依頼してほしい。",
                    result.sessions, result.profile
                ),
            ))
            .ephemeral(true),
    )
    .await?;
    Ok(())
}
