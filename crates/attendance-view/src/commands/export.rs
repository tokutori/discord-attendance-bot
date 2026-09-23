mod delivery;

use std::sync::Arc;

use chrono::Utc;
use poise::{CreateReply, serenity_prelude as serenity};

use crate::{Context, Error, attendance_export, presentation, repository, time};
use delivery::{ExportFormat, Failure};

#[derive(Debug, Clone, Copy, poise::ChoiceParameter)]
enum ExportMode {
    #[name = "preview"]
    Preview,
    #[name = "publish"]
    Publish,
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

fn attachment_size_limit(ctx: Context<'_>) -> usize {
    match ctx {
        poise::Context::Application(context) => context.interaction.attachment_size_limit as usize,
        poise::Context::Prefix(_) => 20 * 1024 * 1024,
    }
}

/// 指定月の全メンバーの活動時間をCSV・PDFで出力する。
#[poise::command(slash_command, guild_only)]
pub async fn export(
    ctx: Context<'_>,
    #[description = "対象月（YYYY-MM）"] month: String,
    #[description = "送信範囲（既定値 preview）"] mode: Option<ExportMode>,
    #[description = "publishで本名を公開することを確認した場合のみ true"] confirm_public: Option<
        bool,
    >,
    #[description = "出力形式（all: CSVとPDF、csv: CSVのみ。既定値 all）"] format: Option<
        ExportFormat,
    >,
) -> Result<(), Error> {
    let publish = matches!(mode.unwrap_or(ExportMode::Preview), ExportMode::Publish);
    ctx.defer_ephemeral().await?;
    let poise::Context::Application(application) = ctx else {
        anyhow::bail!("Slash Commandで実行してほしい");
    };
    check_permissions(
        application
            .interaction
            .member
            .as_ref()
            .and_then(|m| m.permissions),
        application.interaction.app_permissions,
    )?;
    if publish && confirm_public != Some(true) {
        ctx.send(
            CreateReply::default()
                .embed(presentation::error_embed(
                    "公開エクスポートは実行されなかった",
                    "publishは全メンバーの本名と活動時間をチャンネルへ公開する。公開範囲と同意を確認したうえで、confirm_public に true を指定して再実行してほしい。previewには確認は不要。",
                ))
                .ephemeral(true),
        )
        .await?;
        return Ok(());
    }
    let year_month = time::parse_year_month(month.trim())?;
    let (guild_id, user_id) = ids(ctx)?;
    let now = Utc::now();
    let (range_start, range_end) = time::month_bounds(year_month)?;
    let sessions =
        repository::overlapping_for_export(&ctx.data().database, guild_id, range_start, range_end)
            .await?;
    let profiles = repository::user_profiles_for_export(&ctx.data().database, guild_id).await?;
    let export = Arc::new(attendance_export::build_monthly_export(
        year_month, &sessions, &profiles, now,
    )?);
    let notice = repository::peek_auto_end_notice(&ctx.data().database, guild_id, user_id).await?;
    let month_label = format!("{}-{:02}", year_month.year, year_month.month);
    if publish {
        tracing::info!(guild_id, user_id, month = %month_label,
            "publishing attendance export with personal data");
    }
    let results = delivery::deliver(
        format.unwrap_or(ExportFormat::All),
        attachment_size_limit(ctx),
        |kind| {
            let export = Arc::clone(&export);
            async move {
                if kind.is_pdf() {
                    // No database handle enters the renderer. A panic in this task
                    // becomes a per-file generation failure; already sent CSVs remain.
                    tokio::task::spawn_blocking(move || {
                        attendance_export::to_pdf(&export, kind.identity())
                    })
                    .await?
                } else {
                    Ok(attendance_export::to_csv(&export, kind.identity()))
                }
            }
        },
        |kind, bytes| {
            let filename = format!("attendance-{month_label}-{}", kind.suffix());
            // Keep provisional-month notices visible with public CSVs too.
            let caption: String = format!("{filename}\n{}", export.notices.join("\n"))
                .chars()
                .take(1800)
                .collect();
            async move {
                // Exactly one attachment with a bounded, non-user-controlled caption.
                // Never copy the private auto-end notice into a public message.
                let attachment = serenity::CreateAttachment::bytes(bytes, filename);
                if publish {
                    ctx.channel_id()
                        .send_message(
                            ctx.serenity_context(),
                            serenity::CreateMessage::new()
                                .content(caption)
                                .add_file(attachment),
                        )
                        .await?;
                } else {
                    ctx.send(
                        CreateReply::default()
                            .content(caption)
                            .attachment(attachment)
                            .ephemeral(true),
                    )
                    .await?;
                }
                Ok(())
            }
        },
    )
    .await;
    let mut description = format!(
        "対象月: {month_label}\nユーザー数: {}\n並び順: 役割 → 代 → 名前の読み\n送信範囲: {}\n\n",
        export.row_count(),
        if publish {
            "publish（チャンネルへ公開）"
        } else {
            "preview（本人のみ）"
        },
    );
    for (kind, result) in results {
        let status = match result {
            Ok(()) => "送信済み",
            Err(Failure::Generation) => "生成失敗（PDFの場合はフォント設定を確認）",
            Err(Failure::AttachmentLimit) => "未送信（この操作のファイル単体の上限を超過）",
            Err(Failure::RequestLimit) => "未送信（リクエスト容量確保のための24 MiB上限を超過）",
            Err(Failure::Delivery) => "送信を確認できなかった（権限・通信状況と送信先を確認）",
        };
        description.push_str(&format!("{}: {status}\n", kind.suffix()));
    }
    description.push_str(
        "\n再実行時は送信済みファイルの重複に注意。CSVのみ取得する場合は format:csv を指定する。",
    );
    if !export.notices.is_empty() {
        description.push_str("\n\n注記:\n");
        description.push_str(&export.notices.join("\n"));
    }
    let mut embed = presentation::response_embed("活動時間エクスポート結果", description);
    if let Some(notice) = &notice {
        embed = embed.field(
            "自動終了のお知らせ",
            presentation::auto_end_notice_text(notice),
            false,
        );
    }
    ctx.send(CreateReply::default().embed(embed).ephemeral(true))
        .await?;
    Ok(())
}

// Permissions are already computed by Discord for this interaction. Do not
// invoke Poise's REST-backed required_permissions before the initial response.
fn check_permissions(
    user: Option<serenity::Permissions>,
    bot: Option<serenity::Permissions>,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        user.is_some_and(|p| p.contains(serenity::Permissions::MANAGE_GUILD)
            || p.contains(serenity::Permissions::ADMINISTRATOR)),
        "サーバー管理権限が必要である"
    );
    let required = serenity::Permissions::SEND_MESSAGES
        | serenity::Permissions::EMBED_LINKS
        | serenity::Permissions::ATTACH_FILES;
    anyhow::ensure!(
        bot.is_some_and(
            |p| p.contains(required) || p.contains(serenity::Permissions::ADMINISTRATOR)
        ),
        "Botのチャンネル権限が不足している、または確認できない"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_payload_permissions_fail_closed() {
        use poise::serenity_prelude::Permissions as P;
        let bot = P::SEND_MESSAGES | P::EMBED_LINKS | P::ATTACH_FILES;
        assert!(check_permissions(Some(P::MANAGE_GUILD), Some(bot)).is_ok());
        assert!(check_permissions(Some(P::ADMINISTRATOR), Some(P::ADMINISTRATOR)).is_ok());
        for user in [None, Some(P::empty()), Some(P::SEND_MESSAGES)] {
            assert!(check_permissions(user, Some(bot)).is_err());
        }
        assert!(check_permissions(Some(P::MANAGE_GUILD), None).is_err());
        for bit in [P::SEND_MESSAGES, P::EMBED_LINKS, P::ATTACH_FILES] {
            assert!(check_permissions(Some(P::MANAGE_GUILD), Some(bot - bit)).is_err());
        }
    }

    #[test]
    fn export_metadata_does_not_require_rest_before_ack() {
        let command = crate::commands::attendanceview();
        assert!(
            command
                .description
                .as_deref()
                .is_some_and(|value| value != "A slash command")
        );
        let export = command
            .subcommands
            .iter()
            .find(|command| command.name == "export")
            .unwrap();
        assert!(export.required_permissions.is_empty());
        assert!(export.required_bot_permissions.is_empty());
        let format = export
            .parameters
            .iter()
            .find(|p| p.name == "format")
            .unwrap();
        assert!(!format.required);
        assert_eq!(
            format
                .choices
                .iter()
                .map(|c| c.name.as_str())
                .collect::<Vec<_>>(),
            ["all", "csv"]
        );
        for subcommand in command.subcommands {
            assert!(
                subcommand
                    .description
                    .as_deref()
                    .is_some_and(|value| value != "A slash command"),
                "missing description for {}",
                subcommand.name
            );
        }
    }
}
