use chrono::Utc;
use poise::CreateReply;

use crate::{Context, Error, presentation, repository, text};

/// 帳票に使う本人の情報を設定する。
#[poise::command(
    slash_command,
    rename = "attendanceexport",
    subcommands("userconfig", "clearuserconfig", "help"),
    subcommand_required
)]
pub async fn attendanceexport(_: Context<'_>) -> Result<(), Error> {
    Ok(())
}

fn ids(ctx: Context<'_>) -> Result<(i64, i64), Error> {
    let guild =
        crate::config::require_guild(ctx.data().guild_id, ctx.guild_id().map(|guild| guild.get()))?;
    Ok((guild, i64::try_from(ctx.author().id.get())?))
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

/// CSV・PDFと活動中名簿に使用する本人の代・本名・役割・読み仮名を設定する。
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
    #[description = "名簿を五十音順に並べるための本名の読み仮名"]
    #[max_length = 100]
    name_reading: Option<String>,
) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    let (guild_id, user_id) = ids(ctx)?;
    let real_name = real_name.as_deref().map(str::trim);
    let role = role.as_deref().map(str::trim);
    let name_reading = name_reading.as_deref().map(str::trim);
    if real_name.is_some_and(str::is_empty)
        || role.is_some_and(str::is_empty)
        || name_reading.is_some_and(str::is_empty)
    {
        ctx.send(
            CreateReply::default()
                .embed(presentation::error_embed(
                    "ユーザー設定を確認してください",
                    "本名・役割・名前の読みには空白だけの値を指定できない。",
                ))
                .ephemeral(true),
        )
        .await?;
        return Ok(());
    }
    let normalized_name_reading = match name_reading.map(text::normalize_name_reading) {
        Some(Ok(value)) => Some(value),
        Some(Err(error)) => {
            ctx.send(
                CreateReply::default()
                    .embed(presentation::error_embed(
                        "名前の読みを確認してください",
                        error.to_string(),
                    ))
                    .ephemeral(true),
            )
            .await?;
            return Ok(());
        }
        None => None,
    };

    let has_update = generation.is_some()
        || real_name.is_some()
        || role.is_some()
        || normalized_name_reading.is_some();
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
                    name_reading: normalized_name_reading.as_deref(),
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
    if let Some(notice) = &notice
        && let Err(error) = acknowledge_notice(ctx, notice).await
    {
        tracing::warn!(
            %error,
            event_id = notice.event_id,
            "response sent but failed to acknowledge automatic-end notice"
        );
    }
    Ok(())
}

/// 本人の代・本名・役割・読み仮名の設定をすべて解除する。
#[poise::command(slash_command, guild_only)]
pub async fn clearuserconfig(ctx: Context<'_>) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    let (guild_id, user_id) = ids(ctx)?;
    let deleted = repository::delete_user_profile(&ctx.data().database, guild_id, user_id).await?;
    let description = if deleted {
        "代・本名・役割・名前の読みをすべて削除した。活動記録は変更していない。"
    } else {
        "削除するユーザー設定はなかった。活動記録は変更していない。"
    };
    ctx.send(
        CreateReply::default()
            .embed(presentation::response_embed(
                "ユーザー設定を解除した",
                description,
            ))
            .ephemeral(true),
    )
    .await?;
    Ok(())
}
