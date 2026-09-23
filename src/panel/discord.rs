use poise::serenity_prelude as serenity;

use super::{
    manage::{Transport, TransportError},
    store::{Action, Location, Request},
};
use crate::Data;

const PANEL_TEXT: &str = "活動を開始するときは join、活動を終了するときは exit を押す。\n\n押した本人の活動を現在時刻で記録する。\n時刻指定や備考入力には /join・/exit を使用する。";

pub fn components(enabled: bool) -> Vec<serenity::CreateActionRow> {
    vec![serenity::CreateActionRow::Buttons(vec![
        serenity::CreateButton::new(Action::Join.custom_id())
            .label("join｜活動開始")
            .style(serenity::ButtonStyle::Success)
            .disabled(!enabled),
        serenity::CreateButton::new(Action::Exit.custom_id())
            .label("exit｜活動終了")
            .style(serenity::ButtonStyle::Primary)
            .disabled(!enabled),
    ])]
}

fn embed() -> serenity::CreateEmbed {
    serenity::CreateEmbed::new()
        .title("活動記録")
        .description(PANEL_TEXT)
}

fn transport_error(error: serenity::Error) -> TransportError {
    match error {
        serenity::Error::Http(serenity::HttpError::UnsuccessfulRequest(response))
            if response.error.code == 10008 =>
        {
            TransportError::MissingMessage
        }
        _ => TransportError::Failed,
    }
}

pub struct DiscordTransport<'a>(pub &'a serenity::Context);

impl Transport for DiscordTransport<'_> {
    async fn update(&mut self, location: Location, enabled: bool) -> Result<(), TransportError> {
        serenity::ChannelId::new(location.channel_id as u64)
            .edit_message(
                self.0,
                serenity::MessageId::new(location.message_id as u64),
                serenity::EditMessage::new()
                    .content("")
                    .embed(embed())
                    .components(components(enabled)),
            )
            .await
            .map_err(transport_error)?;
        Ok(())
    }

    async fn create_disabled(&mut self, channel_id: i64) -> Result<i64, TransportError> {
        let message = serenity::ChannelId::new(channel_id as u64)
            .send_message(
                self.0,
                serenity::CreateMessage::new()
                    .embed(embed())
                    .components(components(false)),
            )
            .await
            .map_err(transport_error)?;
        i64::try_from(message.id.get()).map_err(|_| TransportError::Failed)
    }

    async fn delete(&mut self, location: Location) -> Result<(), TransportError> {
        serenity::ChannelId::new(location.channel_id as u64)
            .delete_message(self.0, serenity::MessageId::new(location.message_id as u64))
            .await
            .map_err(transport_error)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum InputError {
    #[error("このサーバーからは記録できない")]
    Guild,
    #[error("このパネルの送信元を確認できない")]
    Source,
    #[error("未対応のボタンである。管理者にパネルの再設置を依頼してほしい")]
    Unsupported,
}

/// Validate the untrusted component envelope before constructing a recording request.
pub fn request(
    interaction: &serenity::ComponentInteraction,
    data: &Data,
    now: i64,
) -> Result<Request, InputError> {
    let guild = interaction
        .guild_id
        .filter(|id| id.get() == data.guild_id)
        .ok_or(InputError::Guild)?;
    if interaction
        .member
        .as_ref()
        .is_none_or(|member| member.user.id != interaction.user.id)
        || interaction.user.bot
        || interaction.application_id.get() != data.application_id
        || interaction.message.author.id.get() != data.application_id
        || interaction.message.channel_id != interaction.channel_id
        || interaction.message.guild_id.is_some_and(|id| id != guild)
    {
        return Err(InputError::Source);
    }
    if !matches!(
        interaction.data.kind,
        serenity::ComponentInteractionDataKind::Button
    ) {
        return Err(InputError::Unsupported);
    }
    let action = Action::parse(&interaction.data.custom_id).ok_or(InputError::Unsupported)?;
    let id = |value| i64::try_from(value).map_err(|_| InputError::Source);
    Ok(Request {
        location: Location {
            guild_id: id(guild.get())?,
            channel_id: id(interaction.channel_id.get())?,
            message_id: id(interaction.message.id.get())?,
            application_id: id(interaction.application_id.get())?,
        },
        interaction_id: id(interaction.id.get())?,
        user_id: id(interaction.user.id.get())?,
        display_name: interaction.user.display_name().to_owned(),
        action,
        created_at: interaction.id.created_at().unix_timestamp(),
        received_at: now,
    })
}

// Permissions are already computed by Discord for this interaction. Do not
// invoke Poise's REST-backed required_permissions before the initial response.
pub(super) fn check_permissions(
    user: Option<serenity::Permissions>,
    bot: Option<serenity::Permissions>,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        user.is_some_and(|p| p.contains(serenity::Permissions::MANAGE_GUILD)
            || p.contains(serenity::Permissions::ADMINISTRATOR)),
        "サーバー管理権限が必要である"
    );
    let required = serenity::Permissions::VIEW_CHANNEL
        | serenity::Permissions::SEND_MESSAGES
        | serenity::Permissions::EMBED_LINKS;
    anyhow::ensure!(
        bot.is_some_and(
            |p| p.contains(required) || p.contains(serenity::Permissions::ADMINISTRATOR)
        ),
        "Botのチャンネル権限が不足している、または確認できない"
    );
    Ok(())
}

pub(super) struct ResponseTransport<'a> {
    pub ctx: &'a serenity::Context,
    pub interaction: &'a serenity::ComponentInteraction,
}

impl super::Response for ResponseTransport<'_> {
    async fn acknowledge(&mut self) -> super::Acknowledgement {
        match self.interaction.defer_ephemeral(self.ctx).await {
            Ok(()) => super::Acknowledgement::Accepted,
            Err(serenity::Error::Http(serenity::HttpError::UnsuccessfulRequest(response)))
                if response.error.code == 40060 =>
            {
                super::Acknowledgement::AlreadyAcknowledged
            }
            Err(_) => super::Acknowledgement::Failed,
        }
    }
    async fn reply(&mut self, content: &str) -> bool {
        let sent = self
            .interaction
            .edit_response(
                self.ctx,
                serenity::EditInteractionResponse::new()
                    .embed(crate::presentation::response_embed("活動時間記録", content)),
            )
            .await
            .is_ok();
        if !sent {
            tracing::warn!(
                interaction_id = self.interaction.id.get(),
                "panel response failed; committed recording retained"
            );
        }
        sent
    }
}
