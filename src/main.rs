#![deny(unsafe_code)]

use std::{env, str::FromStr, time::Duration};

use anyhow::Context as _;
use discord_attendance_bot::{Data, auto_end, commands, config, database, framework_error, time};
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
    let guild_database_id = i64::try_from(guild_id).context("guild ID exceeds SQLite range")?;
    let auto_end_enabled = app_config.auto_end_time.is_some();
    time::install_time_policy(time::TimePolicy {
        timezone: app_config.timezone,
        auto_end_time: app_config.auto_end_time,
    })?;

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "discord_attendance_bot=info,poise=info,serenity=info".into()),
        )
        .init();

    let synchronous = match app_config.database_synchronous {
        config::DatabaseSynchronous::Full => SqliteSynchronous::Full,
        config::DatabaseSynchronous::Normal => SqliteSynchronous::Normal,
    };
    let options = SqliteConnectOptions::from_str(&app_config.database_url)?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(synchronous)
        .busy_timeout(Duration::from_secs(5))
        .foreign_keys(true);
    // A live process is not enough: the pool may reap every physical connection.
    // Keep a separate autocommit connection open until the client stops so that
    // readonly view mounts can restart even after a long period without commands.
    let wal_anchor = discord_attendance_bot::wal_anchor::WalAnchor::open(&options)
        .await
        .context("failed to retain the recording WAL connection")?;
    let database = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await
        .context("failed to open SQLite database")?;
    if mode != config::RunMode::Test {
        database::require_persistent_database(&database).await?;
    }
    let sqlite_version = database::validate_runtime_sqlite(&database).await?;
    info!(%sqlite_version, "validated SQLite runtime version");
    sqlx::migrate!("./migrations").run(&database).await?;

    let framework = poise::Framework::builder()
        .options(poise::FrameworkOptions {
            commands: vec![
                commands::attendance(),
                commands::join(),
                commands::exit(),
                commands::attendanceexport(),
            ],
            command_check: Some(|ctx| Box::pin(async move {
                config::require_guild(ctx.data().guild_id, ctx.guild_id().map(|id| id.get()))?;
                Ok(true)
            })),
            skip_checks_for_owners: false,
            on_error: |error| {
                Box::pin(async move {
                    if let Err(error) = framework_error::handle(error).await {
                        tracing::error!(%error, "failed to send framework error response");
                    }
                })
            },
            event_handler: |ctx, event, _, data| Box::pin(async move {
                discord_attendance_bot::panel::handle(ctx, event, data).await;
                Ok(())
            }),
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
                if auto_end_enabled {
                    match auto_end::apply_missed_auto_ends(&database, guild_database_id).await {
                        Ok(count) if count > 0 => {
                            info!(guild_id, count, "applied missed automatic attendance ends");
                        }
                        Ok(_) => {}
                        Err(error) => {
                            tracing::error!(%error, guild_id, "failed to apply missed automatic attendance ends");
                        }
                    }
                }
                if auto_end_enabled {
                    auto_end::spawn_auto_end_scheduler(
                        database.clone(),
                        guild_database_id,
                    );
                }
                Ok(Data {
                    database, guild_id, application_id: ready.user.id.get(),
                    panel_management: tokio::sync::Mutex::new(()),
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
    wal_anchor.close().await?;
    Ok(())
}
