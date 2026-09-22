mod discord;
mod manage;
pub(crate) mod store;

use crate::{Context, Data, Error, presentation, repository};
use poise::{CreateReply, serenity_prelude as serenity};

/// 共用の活動記録パネルをこのチャンネルに設置・更新する。
#[poise::command(
    slash_command,
    guild_only,
    required_permissions = "MANAGE_GUILD",
    required_bot_permissions = "VIEW_CHANNEL | SEND_MESSAGES | EMBED_LINKS"
)]
pub async fn panel(ctx: Context<'_>) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    // Explicit boundary validation in addition to Poise's command metadata.
    let guild = ctx
        .guild_id()
        .filter(|g| g.get() == ctx.data().guild_id)
        .ok_or_else(|| anyhow::anyhow!("設定されたサーバーで実行してほしい"))?;
    let member = ctx
        .author_member()
        .await
        .ok_or_else(|| anyhow::anyhow!("管理権限を確認できない"))?;
    anyhow::ensure!(
        member
            .permissions
            .is_some_and(|p| p.contains(serenity::Permissions::MANAGE_GUILD)),
        "サーバー管理権限が必要である"
    );
    let channel = ctx
        .channel_id()
        .to_channel(ctx.serenity_context())
        .await?
        .guild()
        .ok_or_else(|| anyhow::anyhow!("サーバーのテキストチャンネルで実行してほしい"))?;
    anyhow::ensure!(
        channel.guild_id == guild
            && matches!(
                channel.kind,
                serenity::ChannelType::Text | serenity::ChannelType::News
            ),
        "サーバーのテキストチャンネルまたはアナウンスチャンネルで実行してほしい"
    );
    let _guard = ctx.data().panel_management.lock().await;
    let mut transport = discord::DiscordTransport(ctx.serenity_context());
    let result = manage::install(
        &ctx.data().database,
        i64::try_from(guild.get())?,
        i64::try_from(channel.id.get())?,
        i64::try_from(ctx.data().application_id)?,
        &mut transport,
    )
    .await?;
    let mut message = if result.enabled {
        format!(
            "活動記録パネルを設置・更新した。\nhttps://discord.com/channels/{}/{}/{}",
            result.location.guild_id, result.location.channel_id, result.location.message_id
        )
    } else {
        "パネルは登録されたが、ボタンの有効化に失敗した。同じチャンネルで /attendance panel を再実行してほしい。".into()
    };
    if !result.old_disabled {
        message.push_str("\n旧パネルの見た目を更新できなかった。旧パネルからの記録は拒否される。不要なメッセージは管理者が削除してほしい。");
    }
    ctx.send(
        CreateReply::default()
            .embed(presentation::response_embed("活動記録パネル", message))
            .ephemeral(true),
    )
    .await?;
    Ok(())
}

/// Runs for every component interaction, including messages posted before restart.
pub async fn handle(ctx: &serenity::Context, event: &serenity::FullEvent, data: &Data) {
    let serenity::FullEvent::InteractionCreate {
        interaction: serenity::Interaction::Component(interaction),
    } = event
    else {
        return;
    };
    if !interaction.data.custom_id.starts_with("attendance:") {
        return;
    }
    let now = chrono::Utc::now().timestamp();
    // Without an acknowledged private response, do not begin recording.
    if interaction.defer_ephemeral(ctx).await.is_err() {
        tracing::warn!(
            interaction_id = interaction.id.get(),
            "panel acknowledgement failed; recording not attempted"
        );
        return;
    }
    let request = match discord::request(interaction, data, now) {
        Ok(request) => request,
        Err(error) => {
            reply(ctx, interaction, &error.to_string()).await;
            return;
        }
    };
    let receipt = match store::record(&data.database, &request).await {
        Ok(receipt) => receipt,
        Err(
            error @ (store::RecordingError::InvalidPanel
            | store::RecordingError::Expired
            | store::RecordingError::ReceiptMismatch),
        ) => {
            reply(ctx, interaction, &error.to_string()).await;
            return;
        }
        Err(_) => match store::recover(&data.database, &request).await {
            Ok(Some(receipt)) => receipt,
            _ => {
                tracing::error!(
                    interaction_id = interaction.id.get(),
                    "panel recording result could not be confirmed"
                );
                reply(ctx, interaction, "記録結果を確認できなかった。履歴を確認し、同じ操作を繰り返す前に記録状態を確認してほしい。").await;
                return;
            }
        },
    };
    let mut content = match render(&receipt) {
        Ok(content) => content,
        Err(_) => "記録処理は完了したが結果を表示できなかった。履歴を確認してほしい。".into(),
    };
    let notice = match repository::peek_auto_end_notice(
        &data.database,
        request.location.guild_id,
        request.user_id,
    )
    .await
    {
        Ok(notice) => notice,
        Err(_) => {
            tracing::warn!(
                interaction_id = interaction.id.get(),
                "panel notice lookup failed"
            );
            None
        }
    };
    if let Some(notice) = &notice {
        content.push_str(&format!(
            "\n\n【自動終了のお知らせ】\n{}",
            presentation::auto_end_notice_text(notice)
        ));
    }
    if reply(ctx, interaction, &content).await
        && let Some(notice) = notice
        && repository::acknowledge_auto_end_notice(
            &data.database,
            notice.event_id,
            request.location.guild_id,
            request.user_id,
            now,
        )
        .await
        .is_err()
    {
        tracing::warn!(
            interaction_id = interaction.id.get(),
            "panel response sent but notice acknowledgement failed"
        );
    }
}

async fn reply(
    ctx: &serenity::Context,
    interaction: &serenity::ComponentInteraction,
    content: &str,
) -> bool {
    // Editing the deferred ephemeral response cannot mutate the shared panel.
    let sent = interaction
        .edit_response(
            ctx,
            serenity::EditInteractionResponse::new()
                .embed(presentation::response_embed("活動時間記録", content)),
        )
        .await
        .is_ok();
    if !sent {
        tracing::warn!(
            interaction_id = interaction.id.get(),
            "panel response failed; committed recording retained"
        );
    }
    sent
}

fn render(receipt: &store::Receipt) -> Result<String, Error> {
    use store::{Outcome, Rejection};
    let mut content = match &receipt.outcome {
        Outcome::Start(outcome) => presentation::start_outcome(outcome, receipt.received_at),
        Outcome::End(outcome) => presentation::end_outcome(outcome, receipt.received_at)?,
        Outcome::Rejected(reason) => match reason {
            Rejection::EndBeforeStart => "終了時刻は開始時刻以降である必要がある。",
            Rejection::Overlap => "活動記録の時間帯が別の記録と重複している。",
            Rejection::FutureTime => "未来の時刻は指定できない。",
            Rejection::OpenSessionConflict => {
                "活動中の記録を確認できなかった。履歴を確認してほしい。"
            }
        }
        .into(),
        Outcome::Erased => {
            "この操作は処理済みで、記録内容は完全消去されている。再適用は行わない。".into()
        }
    };
    if receipt.replayed {
        content.push_str("\n\n処理済み操作の結果を表示した。記録変更は再適用していない。");
    }
    Ok(content)
}

#[cfg(test)]
mod tests;
