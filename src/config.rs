use std::env;

use anyhow::{Context as _, bail};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunMode {
    Test,
    Release,
}

impl RunMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Test => "test",
            Self::Release => "release",
        }
    }

    fn guild_id_env(self) -> &'static str {
        match self {
            Self::Test => "DISCORD_TEST_GUILD_ID",
            Self::Release => "DISCORD_RELEASE_GUILD_ID",
        }
    }

    fn database_url_env(self) -> &'static str {
        match self {
            Self::Test => "DATABASE_URL_TEST",
            Self::Release => "DATABASE_URL_RELEASE",
        }
    }

    fn status_channel_id_env(self) -> &'static str {
        match self {
            Self::Test => "ATTENDANCE_STATUS_CHANNEL_ID_TEST",
            Self::Release => "ATTENDANCE_STATUS_CHANNEL_ID_RELEASE",
        }
    }
}

pub fn parse_mode(args: &[String]) -> anyhow::Result<RunMode> {
    if args.len() != 2 {
        bail!("usage: discord-attendance-bot <test|release>");
    }

    match args[1].as_str() {
        "test" => Ok(RunMode::Test),
        "release" => Ok(RunMode::Release),
        _ => bail!(
            "unknown run mode {:?}; expected `test` or `release`",
            args[1]
        ),
    }
}

pub struct AppConfig {
    pub mode: RunMode,
    pub token: String,
    pub guild_id: u64,
    pub database_url: String,
    pub status_channel_id: u64,
}

impl AppConfig {
    pub fn from_env(mode: RunMode) -> anyhow::Result<Self> {
        let token = env::var("DISCORD_TOKEN").context("DISCORD_TOKEN is not set")?;
        let guild_id_env = mode.guild_id_env();
        let guild_id = env::var(guild_id_env)
            .with_context(|| format!("{guild_id_env} is not set"))?
            .parse::<u64>()
            .with_context(|| format!("{guild_id_env} must be a Discord snowflake"))?;
        let database_url_env = mode.database_url_env();
        let database_url =
            env::var(database_url_env).with_context(|| format!("{database_url_env} is not set"))?;
        let status_channel_id_env = mode.status_channel_id_env();
        let status_channel_id = env::var(status_channel_id_env)
            .with_context(|| format!("{status_channel_id_env} is not set"))?
            .parse::<u64>()
            .with_context(|| format!("{status_channel_id_env} must be a Discord snowflake"))?;

        Ok(Self {
            mode,
            token,
            guild_id,
            database_url,
            status_channel_id,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_supported_modes() {
        assert_eq!(
            parse_mode(&["bot".into(), "test".into()]).unwrap(),
            RunMode::Test
        );
        assert_eq!(
            parse_mode(&["bot".into(), "release".into()]).unwrap(),
            RunMode::Release
        );
    }

    #[test]
    fn rejects_missing_or_unknown_mode() {
        assert!(parse_mode(&["bot".into()]).is_err());
        assert!(parse_mode(&["bot".into(), "staging".into()]).is_err());
        assert!(parse_mode(&["bot".into(), "test".into(), "extra".into()]).is_err());
    }
}
