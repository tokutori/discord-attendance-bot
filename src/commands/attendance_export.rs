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

/// 月次活動時間の設定とCSV・PDF出力を行う。
#[poise::command(
    slash_command,
    rename = "attendanceexport",
    subcommands("export", "userconfig", "help"),
    subcommand_required
)]
pub async fn attendanceexport(_: Context<'_>) -> Result<(), Error> {
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

fn attachment_size_limit(ctx: Context<'_>) -> usize {
    match ctx {
        poise::Context::Application(context) => context.interaction.attachment_size_limit as usize,
        poise::Context::Prefix(_) => 10 * 1024 * 1024,
    }
}

async fn acknowledge_notice(
    ctx: Context<'_>,
    notice: &repository::AutoEndNotice,
) -> Result<(), Error> {
    let (guild_id, user_id) = ids(ctx)?;
    repository::acknowledge_auto_end_notice(
        &ctx.data().database,
        notice.event_id,
        guild_id,
        user_id,
        Utc::now().timestamp(),
    )
    .await?;
    Ok(())
}

/// エクスポートコマンドの使い方を表示する。
#[poise::command(slash_command, guild_only)]
pub async fn help(ctx: Context<'_>) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    ctx.send(
        CreateReply::default()
            .embed(presentation::export_help_embed())
            .ephemeral(true),
    )
    .await?;
    Ok(())
}

/// CSV・PDFに使用する本人の代・本名・役割を設定する。
#[poise::command(slash_command, guild_only)]
pub async fn userconfig(
    ctx: Context<'_>,
    #[description = "所属する代（整数）"] generation: Option<i64>,
    #[description = "CSV・PDFに表示する本名"]
    #[max_length = 100]
    real_name: Option<String>,
    #[description = "代表・新入生・班名などの役割"]
    #[max_length = 100]
    role: Option<String>,
) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    let (guild_id, user_id) = ids(ctx)?;
    let real_name = real_name.as_deref().map(str::trim);
    let role = role.as_deref().map(str::trim);
    if real_name.is_some_and(str::is_empty) || role.is_some_and(str::is_empty) {
        ctx.send(
            CreateReply::default()
                .embed(presentation::error_embed(
                    "ユーザー設定を確認してください",
                    "本名と役割には空白だけの値を指定できない。",
                ))
                .ephemeral(true),
        )
        .await?;
        return Ok(());
    }

    let has_update = generation.is_some() || real_name.is_some() || role.is_some();
    let profile = if has_update {
        Some(
            repository::upsert_user_profile(
                &ctx.data().database,
                guild_id,
                user_id,
                repository::UserProfileUpdate {
                    generation,
                    real_name,
                    role,
                },
                Utc::now().timestamp(),
            )
            .await?,
        )
    } else {
        repository::get_user_profile(&ctx.data().database, guild_id, user_id).await?
    };
    let mut embed = presentation::user_profile_embed(profile.as_ref(), has_update);
    let notice = repository::peek_auto_end_notice(&ctx.data().database, guild_id, user_id).await?;
    if let Some(notice) = &notice {
        embed = embed.field(
            "自動終了のお知らせ",
            presentation::auto_end_notice_text(notice),
            false,
        );
    }
    ctx.send(CreateReply::default().embed(embed).ephemeral(true))
        .await?;
    if let Some(notice) = &notice {
        acknowledge_notice(ctx, notice).await?;
    }
    Ok(())
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
) -> Result<(), Error> {
    let publish = matches!(mode.unwrap_or(ExportMode::Preview), ExportMode::Publish);
    ctx.defer_ephemeral().await?;
    let year_month = time::parse_year_month(month.trim())?;
    let (guild_id, user_id) = ids(ctx)?;
    let now = Utc::now();
    let (range_start, range_end) = time::month_bounds(year_month)?;
    let sessions =
        repository::overlapping_for_export(&ctx.data().database, guild_id, range_start, range_end)
            .await?;
    let profiles = repository::user_profiles_for_export(&ctx.data().database, guild_id).await?;
    let export = attendance_export::build_monthly_export(year_month, &sessions, &profiles, now)?;
    let csv = attendance_export::to_csv(&export);
    let pdf = attendance_export::to_pdf(&export)?;
    let size_limit = attachment_size_limit(ctx);
    if csv.len() > size_limit || pdf.len() > size_limit {
        let description = format!(
            "生成したファイルが、この操作で許可された添付上限を超えている。\n\n上限: {:.2} MiB\nCSV: {:.2} MiB\nPDF: {:.2} MiB\n\nPDF用フォントや対象人数を確認してほしい。",
            size_limit as f64 / 1_048_576.0,
            csv.len() as f64 / 1_048_576.0,
            pdf.len() as f64 / 1_048_576.0,
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
        "対象月: {month_label}\nユーザー数: {}\nCSV: {}行 × {}列\nPDF: {}行 × {}列\nユーザー設定: 代・本名・役割を反映\n送信範囲: {}",
        export.row_count(),
        export.row_count() + 1,
        export.csv_column_count(),
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
    let csv_name = format!("attendance-{month_label}.csv");
    let pdf_name = format!("attendance-{month_label}.pdf");
    if publish {
        ctx.channel_id()
            .send_message(
                ctx.serenity_context(),
                serenity::CreateMessage::new().embed(embed).add_files([
                    serenity::CreateAttachment::bytes(csv, csv_name),
                    serenity::CreateAttachment::bytes(pdf, pdf_name),
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
                .attachment(serenity::CreateAttachment::bytes(csv, csv_name))
                .attachment(serenity::CreateAttachment::bytes(pdf, pdf_name))
                .ephemeral(true),
        )
        .await?;
    }
    if let Some(notice) = &notice {
        acknowledge_notice(ctx, notice).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_metadata_requires_manager_and_attachment_permissions() {
        let command = attendanceexport();
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
