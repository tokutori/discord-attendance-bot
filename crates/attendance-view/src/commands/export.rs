use chrono::Utc;
use poise::{CreateReply, serenity_prelude as serenity};

use crate::{Context, Error, attendance_export, presentation, repository, time};

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
        poise::Context::Prefix(_) => 10 * 1024 * 1024,
    }
}

/// 指定月の全メンバーの活動時間をCSV・PDFで出力する。
#[poise::command(
    slash_command,
    guild_only,
    required_permissions = "MANAGE_GUILD",
    required_bot_permissions = "SEND_MESSAGES | EMBED_LINKS | ATTACH_FILES"
)]
pub async fn export(
    ctx: Context<'_>,
    #[description = "対象月（YYYY-MM）"] month: String,
    #[description = "送信範囲（既定値 preview）"] mode: Option<ExportMode>,
    #[description = "publishで本名を公開することを確認した場合のみ true"] confirm_public: Option<
        bool,
    >,
) -> Result<(), Error> {
    let publish = matches!(mode.unwrap_or(ExportMode::Preview), ExportMode::Publish);
    ctx.defer_ephemeral().await?;
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
    let export = attendance_export::build_monthly_export(year_month, &sessions, &profiles, now)?;
    let csv_with_discord =
        attendance_export::to_csv(&export, attendance_export::IdentityMode::WithDiscordName);
    let csv_real_name_only =
        attendance_export::to_csv(&export, attendance_export::IdentityMode::RealNameOnly);
    let pdf_with_discord =
        attendance_export::to_pdf(&export, attendance_export::IdentityMode::WithDiscordName)?;
    let pdf_real_name_only =
        attendance_export::to_pdf(&export, attendance_export::IdentityMode::RealNameOnly)?;
    let size_limit = attachment_size_limit(ctx);
    let generated_sizes = [
        ("Discord表示名ありCSV", csv_with_discord.len()),
        ("本名のみCSV", csv_real_name_only.len()),
        ("Discord表示名ありPDF", pdf_with_discord.len()),
        ("本名のみPDF", pdf_real_name_only.len()),
    ];
    if generated_sizes.iter().any(|(_, size)| *size > size_limit) {
        let sizes = generated_sizes
            .iter()
            .map(|(label, size)| format!("{label}: {:.2} MiB", *size as f64 / 1_048_576.0))
            .collect::<Vec<_>>()
            .join("\n");
        let description = format!(
            "生成したファイルが、この操作で許可された添付上限を超えている。\n\n上限: {:.2} MiB\n{sizes}\n\nPDF用フォントや対象人数を確認してほしい。",
            size_limit as f64 / 1_048_576.0,
        );
        ctx.send(
            CreateReply::default()
                .embed(presentation::error_embed(
                    "添付ファイルが大きすぎる",
                    description,
                ))
                .ephemeral(true),
        )
        .await?;
        return Ok(());
    }
    let notice = repository::peek_auto_end_notice(&ctx.data().database, guild_id, user_id).await?;
    let month_label = format!("{}-{:02}", year_month.year, year_month.month);
    let mut description = format!(
        "対象月: {month_label}\nユーザー数: {}\nDiscord表示名ありCSV: {}行 × {}列\n本名のみCSV: {}行 × {}列\nPDF: 各{}行 × {}列\nユーザー設定: 代・本名・役割・名前の読みを反映\n並び順: 役割 → 代 → 名前の読み\n送信範囲: {}",
        export.row_count(),
        export.row_count() + 1,
        export.csv_column_count(attendance_export::IdentityMode::WithDiscordName),
        export.row_count() + 1,
        export.csv_column_count(attendance_export::IdentityMode::RealNameOnly),
        export.row_count() + 1,
        export.pdf_column_count(),
        if publish {
            "publish（全員に公開）"
        } else {
            "preview（本人のみ）"
        },
    );
    if !export.notices.is_empty() {
        description.push_str("\n\n注記:\n");
        description.push_str(
            &export
                .notices
                .iter()
                .map(|notice| format!("・{notice}"))
                .collect::<Vec<_>>()
                .join("\n"),
        );
    }
    let embed = serenity::CreateEmbed::new()
        .title("活動時間エクスポート")
        .description(description)
        .color(0x2f80ed);
    let csv_with_discord_name = format!("attendance-{month_label}-with-discord.csv");
    let csv_real_name_only_name = format!("attendance-{month_label}-real-name-only.csv");
    let pdf_with_discord_name = format!("attendance-{month_label}-with-discord.pdf");
    let pdf_real_name_only_name = format!("attendance-{month_label}-real-name-only.pdf");
    if publish {
        tracing::info!(
            guild_id,
            user_id,
            month = %month_label,
            "publishing attendance export with personal data"
        );
        ctx.channel_id()
            .send_message(
                ctx.serenity_context(),
                serenity::CreateMessage::new().embed(embed).add_files([
                    serenity::CreateAttachment::bytes(csv_with_discord, csv_with_discord_name),
                    serenity::CreateAttachment::bytes(csv_real_name_only, csv_real_name_only_name),
                    serenity::CreateAttachment::bytes(pdf_with_discord, pdf_with_discord_name),
                    serenity::CreateAttachment::bytes(pdf_real_name_only, pdf_real_name_only_name),
                ]),
            )
            .await?;
        let mut private_description =
            format!("{month_label} のCSV・PDFをこのチャンネルへ公開した。");
        if let Some(notice) = &notice {
            private_description.push_str("\n\n【自動終了のお知らせ】\n");
            private_description.push_str(&presentation::auto_end_notice_text(notice));
        }
        ctx.send(
            CreateReply::default()
                .embed(presentation::response_embed(
                    "活動時間エクスポート",
                    private_description,
                ))
                .ephemeral(true),
        )
        .await?;
    } else {
        let mut preview_embed = embed;
        if let Some(notice) = &notice {
            preview_embed = preview_embed.field(
                "自動終了のお知らせ",
                presentation::auto_end_notice_text(notice),
                false,
            );
        }
        ctx.send(
            CreateReply::default()
                .embed(preview_embed)
                .attachment(serenity::CreateAttachment::bytes(
                    csv_with_discord,
                    csv_with_discord_name,
                ))
                .attachment(serenity::CreateAttachment::bytes(
                    csv_real_name_only,
                    csv_real_name_only_name,
                ))
                .attachment(serenity::CreateAttachment::bytes(
                    pdf_with_discord,
                    pdf_with_discord_name,
                ))
                .attachment(serenity::CreateAttachment::bytes(
                    pdf_real_name_only,
                    pdf_real_name_only_name,
                ))
                .ephemeral(true),
        )
        .await?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_metadata_requires_manager_and_attachment_permissions() {
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
        assert!(
            export
                .required_permissions
                .contains(serenity::Permissions::MANAGE_GUILD)
        );
        assert!(
            export
                .required_bot_permissions
                .contains(serenity::Permissions::ATTACH_FILES)
        );
        assert!(
            export
                .required_bot_permissions
                .contains(serenity::Permissions::EMBED_LINKS)
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
