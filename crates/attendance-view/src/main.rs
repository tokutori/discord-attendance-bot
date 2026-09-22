#![deny(unsafe_code)]

use anyhow::Context as _;
use attendance_view::{Data, channel_status, commands, config, framework_error, time};
use poise::serenity_prelude as serenity;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // A separate config file/environment must contain only the view token.
    dotenvy::from_path(".env.view").ok();
    let args: Vec<String> = std::env::args().collect();
    let mode = config::parse_mode(&args)?;
    let config = config::AppConfig::view_from_env(mode)?;
    let guild_id = config.guild_id;
    let guild_database_id = i64::try_from(guild_id)?;
    let core_application_id = config
        .core_application_id
        .context("missing core application ID")?;
    let status = config.status;
    time::install_time_policy(time::TimePolicy {
        timezone: config.timezone,
        auto_end_time: config.auto_end_time,
    })?;
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "attendance_view=info,poise=info,serenity=info".into()),
        )
        .init();
    let database = attendance_query::ReadDatabase::open(&config.database_url).await?;
    let framework = poise::Framework::builder()
        .options(poise::FrameworkOptions {
            commands: vec![commands::attendanceview()],
            on_error: |error| {
                Box::pin(async move {
                    if let Err(error) = framework_error::handle(error).await {
                        tracing::error!(%error, "failed to send view error response");
                    }
                })
            },
            ..Default::default()
        })
        .setup(move |ctx, ready, framework| {
            Box::pin(async move {
                ensure_distinct_application(ready.user.id.get(), core_application_id)?;
                poise::builtins::register_in_guild(
                    ctx,
                    &framework.options().commands,
                    serenity::GuildId::new(guild_id),
                )
                .await?;
                if status.mode.is_enabled() {
                    channel_status::spawn_periodic_refresh(
                        ctx,
                        database.clone(),
                        guild_database_id,
                        status,
                    );
                }
                Ok(Data { database })
            })
        })
        .build();
    let mut client = serenity::ClientBuilder::new(config.token, serenity::GatewayIntents::empty())
        .framework(framework)
        .await
        .context("failed to create view client")?;
    client.start().await.context("attendance-view stopped")
}

fn ensure_distinct_application(view_id: u64, core_id: u64) -> anyhow::Result<()> {
    anyhow::ensure!(
        core_id != 0 && view_id != core_id,
        "attendance-view must use a different Discord Application from core; refusing command registration"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn refuses_core_identity_before_registration() {
        assert!(super::ensure_distinct_application(12, 12).is_err());
        assert!(super::ensure_distinct_application(12, 0).is_err());
        assert!(super::ensure_distinct_application(13, 12).is_ok());
    }
}
