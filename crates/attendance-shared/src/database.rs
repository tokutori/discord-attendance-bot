use anyhow::Context as _;
use sqlx::SqlitePool;

pub async fn validate_runtime_sqlite(pool: &SqlitePool) -> anyhow::Result<String> {
    let version: String = sqlx::query_scalar("SELECT sqlite_version()")
        .fetch_one(pool)
        .await
        .context("failed to read SQLite version")?;
    ensure_safe_sqlite_version(&version)?;
    Ok(version)
}

pub fn ensure_safe_sqlite_version(version: &str) -> anyhow::Result<()> {
    let mut parts = version.split('.');
    let major = parts
        .next()
        .context("SQLite version is missing major component")?
        .parse::<u32>()?;
    let minor = parts
        .next()
        .context("SQLite version is missing minor component")?
        .parse::<u32>()?;
    let patch = parts
        .next()
        .context("SQLite version is missing patch component")?
        .parse::<u32>()?;
    let safe = major > 3
        || (major == 3
            && (minor > 51
                || (minor == 51 && patch >= 3)
                || (minor == 50 && patch >= 7)
                || (minor == 44 && patch >= 6)));
    if !safe {
        anyhow::bail!(
            "SQLite {version} is affected by the WAL-reset corruption bug; use 3.51.3 or a supported fixed backport"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_vulnerable_sqlite_and_accepts_fixed_releases() {
        assert!(ensure_safe_sqlite_version("3.46.0").is_err());
        assert!(ensure_safe_sqlite_version("3.51.2").is_err());
        assert!(ensure_safe_sqlite_version("3.44.6").is_ok());
        assert!(ensure_safe_sqlite_version("3.50.7").is_ok());
        assert!(ensure_safe_sqlite_version("3.51.3").is_ok());
        assert!(ensure_safe_sqlite_version("3.53.2").is_ok());
    }
}
