use std::env;

use anyhow::{Context as _, bail};
use chrono::NaiveTime;
use chrono_tz::Tz;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunMode {
    Standalone,
    Test,
    Release,
}

impl RunMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Standalone => "standalone",
            Self::Test => "test",
            Self::Release => "release",
        }
    }

    fn guild_id_env(self) -> &'static str {
        match self {
            Self::Standalone => "DISCORD_GUILD_ID",
            Self::Test => "DISCORD_TEST_GUILD_ID",
            Self::Release => "DISCORD_RELEASE_GUILD_ID",
        }
    }

    fn database_url_env(self) -> &'static str {
        match self {
            Self::Standalone => "DATABASE_URL",
            Self::Test => "DATABASE_URL_TEST",
            Self::Release => "DATABASE_URL_RELEASE",
        }
    }

    fn status_channel_id_env(self) -> &'static str {
        match self {
            Self::Standalone => "ATTENDANCE_STATUS_CHANNEL_ID",
            Self::Test => "ATTENDANCE_STATUS_CHANNEL_ID_TEST",
            Self::Release => "ATTENDANCE_STATUS_CHANNEL_ID_RELEASE",
        }
    }
}

pub fn parse_mode(args: &[String]) -> anyhow::Result<RunMode> {
    match args {
        [_] => Ok(RunMode::Standalone),
        [_, mode] if mode == "test" => Ok(RunMode::Test),
        [_, mode] if mode == "release" => Ok(RunMode::Release),
        [_, mode] => bail!("unknown run mode {mode:?}; expected `test`, `release`, or no argument"),
        _ => bail!("usage: discord-attendance-bot [test|release]"),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusMode {
    Disabled,
    Count,
    Names,
}

impl StatusMode {
    pub fn shows_names(self) -> bool {
        matches!(self, Self::Names)
    }

    pub fn is_enabled(self) -> bool {
        !matches!(self, Self::Disabled)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct StatusConfig {
    pub mode: StatusMode,
    pub channel_id: Option<u64>,
    pub refresh_interval_seconds: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatabaseSynchronous {
    Full,
    Normal,
}

pub struct AppConfig {
    pub mode: RunMode,
    pub token: String,
    pub guild_id: u64,
    pub database_url: String,
    pub status: StatusConfig,
    pub timezone: Tz,
    pub auto_end_time: Option<NaiveTime>,
    pub database_synchronous: DatabaseSynchronous,
}

impl AppConfig {
    pub fn from_env(mode: RunMode) -> anyhow::Result<Self> {
        let token = required_env("DISCORD_TOKEN")?;
        let guild_id_env = mode.guild_id_env();
        let guild_id = required_env(guild_id_env)?
            .parse::<u64>()
            .with_context(|| format!("{guild_id_env} must be a Discord snowflake"))?;
        let database_url_env = mode.database_url_env();
        let database_url = required_env(database_url_env)?;
        if mode != RunMode::Test && is_in_memory_database(&database_url) {
            bail!("{database_url_env} must use a persistent SQLite file outside test mode");
        }

        let status_channel_id_env = mode.status_channel_id_env();
        let status_channel_id = optional_env(status_channel_id_env)
            .map(|value| {
                value
                    .parse::<u64>()
                    .with_context(|| format!("{status_channel_id_env} must be a Discord snowflake"))
            })
            .transpose()?;
        let status_mode = parse_status_mode(
            optional_env("ATTENDANCE_STATUS_MODE").as_deref(),
            status_channel_id.is_some(),
        )?;
        if status_mode.is_enabled() && status_channel_id.is_none() {
            bail!("{status_channel_id_env} is required when ATTENDANCE_STATUS_MODE is enabled");
        }
        let refresh_interval_seconds = optional_env("ATTENDANCE_STATUS_REFRESH_SECONDS")
            .map(|value| {
                value
                    .parse::<u64>()
                    .context("ATTENDANCE_STATUS_REFRESH_SECONDS must be an integer")
            })
            .transpose()?
            .unwrap_or(600);
        if !(60..=86_400).contains(&refresh_interval_seconds) {
            bail!("ATTENDANCE_STATUS_REFRESH_SECONDS must be between 60 and 86400");
        }

        let timezone = optional_env("ATTENDANCE_TIMEZONE")
            .unwrap_or_else(|| "Asia/Tokyo".into())
            .parse::<Tz>()
            .context("ATTENDANCE_TIMEZONE must be an IANA timezone such as Asia/Tokyo")?;
        let auto_end_time = parse_auto_end_time(
            optional_env("ATTENDANCE_AUTO_END_TIME")
                .as_deref()
                .unwrap_or("21:00"),
        )?;
        let database_synchronous = parse_database_synchronous(
            optional_env("ATTENDANCE_SQLITE_SYNCHRONOUS")
                .as_deref()
                .unwrap_or("full"),
        )?;

        Ok(Self {
            mode,
            token,
            guild_id,
            database_url,
            status: StatusConfig {
                mode: status_mode,
                channel_id: status_channel_id,
                refresh_interval_seconds,
            },
            timezone,
            auto_end_time,
            database_synchronous,
        })
    }
}

fn required_env(name: &str) -> anyhow::Result<String> {
    optional_env(name).with_context(|| format!("{name} is not set"))
}

fn optional_env(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn is_in_memory_database(database_url: &str) -> bool {
    matches!(database_url, "sqlite::memory:" | "sqlite://:memory:")
        || database_url.contains("mode=memory")
}

fn parse_status_mode(value: Option<&str>, has_channel: bool) -> anyhow::Result<StatusMode> {
    match value.map(str::to_ascii_lowercase).as_deref() {
        None if has_channel => Ok(StatusMode::Count),
        None => Ok(StatusMode::Disabled),
        Some("disabled" | "off" | "none") => Ok(StatusMode::Disabled),
        Some("count") => Ok(StatusMode::Count),
        Some("names") => Ok(StatusMode::Names),
        Some(_) => bail!("ATTENDANCE_STATUS_MODE must be disabled, count, or names"),
    }
}

fn parse_auto_end_time(value: &str) -> anyhow::Result<Option<NaiveTime>> {
    match value.to_ascii_lowercase().as_str() {
        "disabled" | "off" | "none" => Ok(None),
        _ => NaiveTime::parse_from_str(value, "%H:%M")
            .map(Some)
            .context("ATTENDANCE_AUTO_END_TIME must be HH:MM or disabled"),
    }
}

fn parse_database_synchronous(value: &str) -> anyhow::Result<DatabaseSynchronous> {
    match value.to_ascii_lowercase().as_str() {
        "full" => Ok(DatabaseSynchronous::Full),
        "normal" => Ok(DatabaseSynchronous::Normal),
        _ => bail!("ATTENDANCE_SQLITE_SYNCHRONOUS must be full or normal"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_supported_modes_and_defaults_to_standalone() {
        assert_eq!(parse_mode(&["bot".into()]).unwrap(), RunMode::Standalone);
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
    fn rejects_unknown_or_extra_modes() {
        assert!(parse_mode(&["bot".into(), "staging".into()]).is_err());
        assert!(parse_mode(&["bot".into(), "test".into(), "extra".into()]).is_err());
    }

    #[test]
    fn status_mode_defaults_follow_channel_presence() {
        assert_eq!(
            parse_status_mode(None, false).unwrap(),
            StatusMode::Disabled
        );
        assert_eq!(parse_status_mode(None, true).unwrap(), StatusMode::Count);
        assert_eq!(
            parse_status_mode(Some("count"), true).unwrap(),
            StatusMode::Count
        );
        assert!(parse_status_mode(Some("public"), true).is_err());
    }

    #[test]
    fn parses_optional_auto_end_and_safe_synchronous_default() {
        assert_eq!(
            parse_auto_end_time("21:30").unwrap(),
            NaiveTime::from_hms_opt(21, 30, 0)
        );
        assert_eq!(parse_auto_end_time("disabled").unwrap(), None);
        assert!(parse_auto_end_time("25:00").is_err());
        assert_eq!(
            parse_database_synchronous("full").unwrap(),
            DatabaseSynchronous::Full
        );
    }

    #[test]
    fn detects_in_memory_database_urls() {
        assert!(is_in_memory_database("sqlite::memory:"));
        assert!(is_in_memory_database("sqlite://file?mode=memory"));
        assert!(!is_in_memory_database("sqlite://attendance.db"));
    }
}
