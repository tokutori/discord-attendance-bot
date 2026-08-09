use std::{env, str::FromStr};

use anyhow::Context as _;
use discord_attendance_bot::{Data, channel_status, commands, config, presentation};
use poise::serenity_prelude as serenity;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use tracing::info;

fn explain_command_error(error: &anyhow::Error) -> (&'static str, String) {
    let message = error.to_string();
    let lowercase = message.to_ascii_lowercase();
    if message.contains("年月は") || message.contains("月は1から12") {
        return (
            "入力形式エラー",
            format!("原因: {message}\n対象月は `YYYY-MM` 形式で指定してほしい。例: `2026-08`。"),
        );
    }
    if message.contains("時刻は") || message.contains("日時は") {
        return (
            "入力形式エラー",
            format!("原因: {message}\nコマンドのヘルプに記載された形式で入力してほしい。"),
        );
    }
    if lowercase.contains("request entity too large") || lowercase.contains("payload too large") {
        return (
            "添付ファイルが大きすぎる",
            "原因: Discord が CSV または PDF の添付をサイズ超過として受け付けなかった。`preview` と `publish` では添付サイズは変わらない。\n対処: 管理者は `ATTENDANCE_PDF_FONT_PATH` に軽量な日本語 TTF またはサブセット済みフォントを指定してほしい。改善しない場合は対象月の人数・記録数による制限の可能性がある。".into(),
        );
    }
    if message.contains("PDF") || message.contains("フォント") {
        return (
            "PDF生成設定エラー",
            format!(
                "原因: {message}\n日本語 TTF フォントを `ATTENDANCE_PDF_FONT_PATH` に指定して再試行してほしい。"
            ),
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

async fn handle_error(
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
                        "`/attendance help` で利用可能なコマンドを確認できる。",
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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let args: Vec<String> = env::args().collect();
    let mode = config::parse_mode(&args)?;
    let app_config = config::AppConfig::from_env(mode)?;
    let guild_id = app_config.guild_id;
    let status_channel_id = app_config.status_channel_id;

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "discord_attendance_bot=info,poise=info,serenity=info".into()),
        )
        .init();

    let options = SqliteConnectOptions::from_str(&app_config.database_url)?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .foreign_keys(true);
    let database = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await
        .context("failed to open SQLite database")?;
    sqlx::migrate!("./migrations").run(&database).await?;

    let framework = poise::Framework::builder()
        .options(poise::FrameworkOptions {
            commands: vec![commands::attendance(), commands::attendanceexport()],
            on_error: |error| {
                Box::pin(async move {
                    if let Err(error) = handle_error(error).await {
                        tracing::error!(%error, "failed to send framework error response");
                    }
                })
            },
            ..Default::default()
        })
        .setup(move |ctx, ready, framework| {
            let database = database.clone();
            Box::pin(async move {
                info!(
                    bot = %ready.user.name,
                    id = %ready.user.id,
                    mode = mode.as_str(),
                    guild_id,
                    "connected to Discord"
                );
                poise::builtins::register_in_guild(
                    ctx,
                    &framework.options().commands,
                    serenity::GuildId::new(guild_id),
                )
                .await?;
                info!(mode = mode.as_str(), guild_id, "registered guild commands");
                match channel_status::apply_due_auto_ends(&database, guild_id as i64).await {
                    Ok(count) if count > 0 => {
                        info!(guild_id, count, "applied missed automatic attendance ends");
                    }
                    Ok(_) => {}
                    Err(error) => {
                        tracing::error!(%error, guild_id, "failed to apply missed automatic attendance ends");
                    }
                }
                channel_status::spawn_auto_end_scheduler(
                    ctx,
                    database.clone(),
                    guild_id as i64,
                );
                channel_status::spawn_periodic_refresh(
                    ctx,
                    database.clone(),
                    guild_id as i64,
                    status_channel_id,
                );
                Ok(Data {
                    database,
                    status_channel_id,
                })
            })
        })
        .build();

    let intents = serenity::GatewayIntents::empty();
    let mut client = serenity::ClientBuilder::new(app_config.token, intents)
        .framework(framework)
        .await
        .context("failed to create Discord client")?;
    client.start().await.context("Discord client stopped")?;
    Ok(())
}
