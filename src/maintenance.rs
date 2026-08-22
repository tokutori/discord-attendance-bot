use std::{path::Path, time::Duration};

use anyhow::Context as _;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

use crate::database;

pub async fn backup_database(source: &Path, destination: &Path) -> anyhow::Result<()> {
    if destination.exists() {
        anyhow::bail!(
            "backup destination already exists: {}",
            destination.display()
        );
    }
    let parent = destination
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if !parent.is_dir() {
        anyhow::bail!(
            "backup destination directory does not exist: {}",
            parent.display()
        );
    }

    let file_name = destination
        .file_name()
        .context("backup destination must have a file name")?
        .to_string_lossy();
    let partial =
        destination.with_file_name(format!(".{file_name}.partial-{}", std::process::id()));
    if partial.exists() {
        anyhow::bail!(
            "temporary backup destination already exists: {}",
            partial.display()
        );
    }

    let source_options = SqliteConnectOptions::new()
        .filename(source)
        .create_if_missing(false)
        .busy_timeout(Duration::from_secs(30))
        .foreign_keys(true);
    let source_pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(source_options)
        .await
        .with_context(|| format!("failed to open source database: {}", source.display()))?;
    database::validate_runtime_sqlite(&source_pool).await?;

    let partial_text = partial
        .to_str()
        .context("backup destination path is not valid UTF-8")?;
    let backup_result = sqlx::query("VACUUM INTO ?")
        .bind(partial_text)
        .execute(&source_pool)
        .await;
    source_pool.close().await;
    if let Err(error) = backup_result {
        let _ = std::fs::remove_file(&partial);
        return Err(error).context("SQLite VACUUM INTO backup failed");
    }

    if let Err(error) = verify_database(&partial).await {
        let _ = std::fs::remove_file(&partial);
        return Err(error).context("created backup did not pass integrity verification");
    }
    std::fs::rename(&partial, destination).with_context(|| {
        format!(
            "failed to finalize backup {} -> {}",
            partial.display(),
            destination.display()
        )
    })?;
    Ok(())
}

pub async fn verify_database(path: &Path) -> anyhow::Result<()> {
    let options = SqliteConnectOptions::new()
        .filename(path)
        .read_only(true)
        .create_if_missing(false)
        .busy_timeout(Duration::from_secs(30))
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .with_context(|| {
            format!(
                "failed to open database for verification: {}",
                path.display()
            )
        })?;
    database::validate_runtime_sqlite(&pool).await?;
    let results = sqlx::query_scalar::<_, String>("PRAGMA integrity_check")
        .fetch_all(&pool)
        .await
        .context("PRAGMA integrity_check failed")?;
    let migration_table_exists: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master
         WHERE type = 'table' AND name = '_sqlx_migrations'",
    )
    .fetch_one(&pool)
    .await?;
    pool.close().await;
    if results.as_slice() != ["ok"] {
        anyhow::bail!("database integrity check failed: {}", results.join("; "));
    }
    if migration_table_exists != 1 {
        anyhow::bail!("database does not contain SQLx migration metadata");
    }
    Ok(())
}

/// Restores a verified backup to a path that does not yet exist.
///
/// The caller must stop the bot and move the current database together with
/// any `-wal` and `-shm` files out of the way first. Refusing to overwrite an
/// existing destination prevents accidental in-place replacement.
pub async fn restore_database(backup: &Path, destination: &Path) -> anyhow::Result<()> {
    if destination.exists() {
        anyhow::bail!(
            "restore destination already exists; stop the bot and move the current database aside first: {}",
            destination.display()
        );
    }
    verify_database(backup).await?;
    let parent = destination
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if !parent.is_dir() {
        anyhow::bail!(
            "restore destination directory does not exist: {}",
            parent.display()
        );
    }
    let file_name = destination
        .file_name()
        .context("restore destination must have a file name")?
        .to_string_lossy();
    let partial =
        destination.with_file_name(format!(".{file_name}.restore-{}", std::process::id()));
    if partial.exists() {
        anyhow::bail!(
            "temporary restore destination already exists: {}",
            partial.display()
        );
    }
    std::fs::copy(backup, &partial).with_context(|| {
        format!(
            "failed to copy backup {} to {}",
            backup.display(),
            partial.display()
        )
    })?;
    if let Err(error) = verify_database(&partial).await {
        let _ = std::fs::remove_file(&partial);
        return Err(error).context("restored copy did not pass integrity verification");
    }
    std::fs::rename(&partial, destination).with_context(|| {
        format!(
            "failed to finalize restore {} -> {}",
            partial.display(),
            destination.display()
        )
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use sqlx::sqlite::SqlitePoolOptions;
    use tempfile::tempdir;

    use super::*;

    #[tokio::test]
    async fn creates_and_verifies_live_wal_backup() {
        let directory = tempdir().unwrap();
        let source = directory.path().join("source.db");
        let backup = directory.path().join("backup.db");
        let pool = SqlitePoolOptions::new()
            .max_connections(2)
            .connect_with(
                SqliteConnectOptions::new()
                    .filename(&source)
                    .create_if_missing(true)
                    .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal),
            )
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO attendance_user_profiles (
                guild_id, user_id, real_name, created_at, updated_at
             ) VALUES (1, 2, 'テスト利用者', 100, 100)",
        )
        .execute(&pool)
        .await
        .unwrap();

        backup_database(&source, &backup).await.unwrap();
        verify_database(&backup).await.unwrap();
        let backup_pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                SqliteConnectOptions::new()
                    .filename(&backup)
                    .read_only(true),
            )
            .await
            .unwrap();
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM attendance_user_profiles")
            .fetch_one(&backup_pool)
            .await
            .unwrap();
        assert_eq!(count, 1);
        backup_pool.close().await;
        pool.close().await;
    }

    #[tokio::test]
    async fn refuses_to_overwrite_existing_backup() {
        let directory = tempdir().unwrap();
        let source = directory.path().join("source.db");
        let destination = directory.path().join("existing.db");
        std::fs::write(&source, b"not a database").unwrap();
        std::fs::write(&destination, b"keep me").unwrap();
        assert!(backup_database(&source, &destination).await.is_err());
        assert_eq!(std::fs::read(&destination).unwrap(), b"keep me");
    }

    #[tokio::test]
    async fn restore_refuses_overwrite_and_creates_verified_copy() {
        let directory = tempdir().unwrap();
        let source = directory.path().join("source.db");
        let backup = directory.path().join("backup.db");
        let restored = directory.path().join("restored.db");
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                SqliteConnectOptions::new()
                    .filename(&source)
                    .create_if_missing(true),
            )
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool.close().await;
        backup_database(&source, &backup).await.unwrap();

        restore_database(&backup, &restored).await.unwrap();
        verify_database(&restored).await.unwrap();
        assert!(restore_database(&backup, &restored).await.is_err());
    }
}
