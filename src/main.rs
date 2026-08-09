use std::{env, str::FromStr};

use anyhow::Context as _;
use discord_attendance_bot::{Data, commands};
use poise::serenity_prelude as serenity;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use tracing::info;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "discord_attendance_bot=info,poise=info,serenity=info".into()),
        )
        .init();

    let token = env::var("DISCORD_TOKEN").context("DISCORD_TOKEN is not set")?;
    let database_url = env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite://attendance.db".into());
    let test_guild_id = env::var("DISCORD_TEST_GUILD_ID")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.parse::<u64>())
        .transpose()
        .context("DISCORD_TEST_GUILD_ID must be a Discord snowflake")?;

    let options = SqliteConnectOptions::from_str(&database_url)?
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
            commands: vec![commands::attendance()],
            on_error: |error| {
                Box::pin(async move {
                    if let Err(error) = poise::builtins::on_error(error).await {
                        tracing::error!(%error, "failed to send command error response");
                    }
                })
            },
            ..Default::default()
        })
        .setup(move |ctx, ready, framework| {
            let database = database.clone();
            Box::pin(async move {
                info!(bot = %ready.user.name, id = %ready.user.id, "connected to Discord");
                if let Some(guild_id) = test_guild_id {
                    poise::builtins::register_in_guild(
                        ctx,
                        &framework.options().commands,
                        serenity::GuildId::new(guild_id),
                    )
                    .await?;
                    info!(guild_id, "registered guild commands");
                } else {
                    poise::builtins::register_globally(ctx, &framework.options().commands).await?;
                    info!("registered global commands");
                }
                Ok(Data { database })
            })
        })
        .build();

    let intents = serenity::GatewayIntents::empty();
    let mut client = serenity::ClientBuilder::new(token, intents)
        .framework(framework)
        .await
        .context("failed to create Discord client")?;
    client.start().await.context("Discord client stopped")?;
    Ok(())
}
