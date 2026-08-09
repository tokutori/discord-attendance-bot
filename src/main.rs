use std::{env, str::FromStr};

use anyhow::Context as _;
use discord_attendance_bot::{Data, channel_status, commands, config, framework_error};
use poise::serenity_prelude as serenity;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use tracing::info;

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
                    if let Err(error) = framework_error::handle(error).await {
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
                Ok(Data { database })
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
