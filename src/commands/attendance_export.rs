use chrono::Utc;
use poise::{CreateReply, serenity_prelude as serenity};

use crate::{Context, Error, attendance_export, presentation, repository, time};

#[poise::command(
    slash_command,
    rename = "attendanceexport",
    subcommands("export", "help"),
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
            "`/attendanceexport export month:YYYY-MM [mode:preview|publish]`\n\n`/attendanceexport help`\nこのヘルプを表示する。",
            false,
        )
        .field(
            "出力内容",
            "縦方向はユーザー、横方向はその月の日付と合計。活動時間があるセルは `時間:分` 形式。PDFの0時間セルは空欄、CSVの0時間セルは `0:00` と表示する。",
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
    let mut export = attendance_export::build_monthly_export(year_month, &sessions, now)?;
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
        "対象月: {month_label}\nユーザー数: {}\n表のサイズ: {}行 × {}列\n送信範囲: {}",
        export.row_count(),
        export.row_count() + 1,
        export.column_count(),
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
