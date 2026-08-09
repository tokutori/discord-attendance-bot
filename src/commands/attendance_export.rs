use chrono::Utc;
use poise::{CreateReply, serenity_prelude as serenity};

use crate::{Context, Error, attendance_export, presentation, repository, time};

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

fn export_help_embed() -> serenity::CreateEmbed {
    serenity::CreateEmbed::new()
        .title("活動時間エクスポート ヘルプ")
        .description("月単位の活動時間を CSV と PDF で出力する。month は必須。通常は preview で本人だけに送信する。")
        .field(
            "使い方",
            "`/attendanceexport export month:YYYY-MM [mode:preview|publish]`\n月次帳票を出力する。\n\n`/attendanceexport userconfig [generation] [real_name] [role]`\n代・本名・役割を設定する。全項目を省略すると現在値を表示する。\n\n`/attendanceexport help`\nこのヘルプを表示する。",
            false,
        )
        .field(
            "出力内容",
            "ユーザー設定の代・本名・役割をCSVの列とPDFのユーザー情報へ反映する。未設定の本名はDiscord表示名を使用する。活動時間があるセルは `時間:分` 形式。PDFの0時間セルは空欄、CSVの0時間セルは `0:00` と表示する。",
            false,
        )
        .field(
            "preview / publish",
            "`preview`（既定）: 実行者だけに表示する。\n`publish`: サーバー全員が見られるメッセージとして送信する。",
            false,
        )
        .field(
            "集計上の注記",
            "対象月が終了していない場合は暫定集計と明記する。翌月1日に出力した場合も、修正の可能性があるため確定版ではないと明記する。",
            false,
        )
        .footer(serenity::CreateEmbedFooter::new(
            "PDFは日本語フォントの設定が必要な場合がある",
        ))
        .color(0x2f80ed)
}

fn profile_embed(
    profile: Option<&repository::UserProfile>,
    updated: bool,
) -> serenity::CreateEmbed {
    let title = if updated {
        "ユーザー設定を更新した"
    } else {
        "現在のユーザー設定"
    };
    let generation = profile
        .and_then(|value| value.generation)
        .map(|value| format!("{value}代"))
        .unwrap_or_else(|| "未設定".into());
    let real_name = profile
        .and_then(|value| value.real_name.as_deref())
        .unwrap_or("未設定");
    let role = profile
        .and_then(|value| value.role.as_deref())
        .unwrap_or("未設定");
    serenity::CreateEmbed::new()
        .title(title)
        .description("この設定は月次CSV・PDFのユーザー情報に使用する。")
        .field("代", generation, true)
        .field("本名", real_name, true)
        .field("役割", role, true)
        .footer(serenity::CreateEmbedFooter::new(
            "未指定の項目は既存値を維持する",
        ))
        .color(0x2f80ed)
}

#[poise::command(slash_command, guild_only)]
pub async fn help(ctx: Context<'_>) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    ctx.send(
        CreateReply::default()
            .embed(export_help_embed())
            .ephemeral(true),
    )
    .await?;
    Ok(())
}

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
    let mut embed = profile_embed(profile.as_ref(), has_update);
    if let Some(notice) = repository::take_auto_end_notice(
        &ctx.data().database,
        guild_id,
        user_id,
        Utc::now().timestamp(),
    )
    .await?
    {
        embed = embed.field(
            "自動終了のお知らせ",
            format!(
                "記録 #{} は {} に自動終了として扱った。必要なら `/attendance edit` で修正してほしい。",
                notice.session_id,
                time::format_datetime(notice.automatic_ended_at)
            ),
            false,
        );
    }
    ctx.send(CreateReply::default().embed(embed).ephemeral(true))
        .await?;
    Ok(())
}

#[poise::command(slash_command, guild_only)]
pub async fn export(
    ctx: Context<'_>,
    #[description = "対象月（YYYY-MM）"] month: String,
    #[description = "送信範囲（preview または publish。既定値 preview）"] mode: Option<String>,
) -> Result<(), Error> {
    let publish = match mode
        .as_deref()
        .unwrap_or("preview")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "preview" => false,
        "publish" => true,
        _ => {
            ctx.defer_ephemeral().await?;
            ctx.send(
                CreateReply::default()
                    .embed(presentation::response_embed(
                        "活動時間エクスポート",
                        "mode は `preview` または `publish` で指定する。",
                    ))
                    .ephemeral(true),
            )
            .await?;
            return Ok(());
        }
    };
    if publish {
        ctx.defer().await?;
    } else {
        ctx.defer_ephemeral().await?;
    }
    let year_month = time::parse_year_month(month.trim())?;
    let (guild_id, user_id) = ids(ctx)?;
    let now = Utc::now();
    let (range_start, range_end) = time::month_bounds(year_month)?;
    let sessions =
        repository::overlapping_for_export(&ctx.data().database, guild_id, range_start, range_end)
            .await?;
    let profiles = repository::user_profiles_for_export(&ctx.data().database, guild_id).await?;
    let mut export =
        attendance_export::build_monthly_export(year_month, &sessions, &profiles, now)?;
    if let Some(notice) =
        repository::take_auto_end_notice(&ctx.data().database, guild_id, user_id, now.timestamp())
            .await?
    {
        export.notices.push(if notice.corrected_at.is_some() {
            format!(
                "前回の終了忘れによる記録 #{} の自動終了（{}）は、ユーザー入力を正として扱った。",
                notice.session_id,
                time::format_datetime(notice.automatic_ended_at)
            )
        } else {
            format!(
                "前回の終了忘れにより、記録 #{} は {} に自動終了として扱った。実際の終了時刻が異なる場合は `/attendance edit record:{}` で修正してほしい。",
                notice.session_id,
                time::format_datetime(notice.automatic_ended_at),
                notice.session_id
            )
        });
    }
    let csv = attendance_export::to_csv(&export);
    let pdf = attendance_export::to_pdf(&export)?;
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
    ctx.send(
        CreateReply::default()
            .embed(embed)
            .attachment(serenity::CreateAttachment::bytes(
                csv,
                format!("attendance-{month_label}.csv"),
            ))
            .attachment(serenity::CreateAttachment::bytes(
                pdf,
                format!("attendance-{month_label}.pdf"),
            ))
            .ephemeral(!publish),
    )
    .await?;
    Ok(())
}
