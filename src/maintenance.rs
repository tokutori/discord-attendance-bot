use std::{path::Path, time::Duration};

use anyhow::Context as _;
use sqlx::{Connection, SqliteConnection, sqlite::SqliteConnectOptions};
use tempfile::TempPath;

use crate::database;

/// Unique, private staging file on the destination filesystem. It is empty so
/// SQLite VACUUM INTO may populate it, and is removed on all error paths.
fn prepare_destination(destination: &Path) -> anyhow::Result<TempPath> {
    anyhow::ensure!(
        !destination.exists(),
        "destination already exists: {}",
        destination.display()
    );
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    Ok(tempfile::Builder::new()
        .prefix(".attendance-")
        .tempfile_in(parent)?
        .into_temp_path())
}

fn finalize(partial: TempPath, destination: &Path) -> anyhow::Result<()> {
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&partial)?
        .sync_all()?;
    // Unlike rename, this fails if another process created the destination
    // after our initial check. Never fall back to an overwriting operation.
    partial.persist_noclobber(destination).with_context(|| {
        format!(
            "failed to finalize database without overwriting {}",
            destination.display()
        )
    })?;
    Ok(())
}

pub async fn backup_database(source: &Path, destination: &Path) -> anyhow::Result<()> {
    let partial = prepare_destination(destination)?;
    let options = SqliteConnectOptions::new()
        .filename(source)
        .create_if_missing(false)
        .busy_timeout(Duration::from_secs(30))
        .foreign_keys(true);
    let mut connection = SqliteConnection::connect_with(&options).await?;
    let version: String = sqlx::query_scalar("SELECT sqlite_version()")
        .fetch_one(&mut connection)
        .await?;
    database::ensure_safe_sqlite_version(&version)?;
    // VACUUM INTO requires an existing empty file or an absent path.
    let result = sqlx::query("VACUUM INTO ?")
        .bind(partial.to_str().context("backup path is not valid UTF-8")?)
        .execute(&mut connection)
        .await;
    connection.close().await?;
    result.context("SQLite VACUUM INTO backup failed")?;
    verify_database(&partial)
        .await
        .context("created backup did not pass verification")?;
    finalize(partial, destination)
}

const SCHEMA_SQL: &str = "SELECT type,name,tbl_name,sql FROM sqlite_schema WHERE name NOT GLOB 'sqlite_*' ORDER BY type,name";
type SchemaObject = (String, String, String, Option<String>);
static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

async fn verify_connection(connection: &mut SqliteConnection) -> anyhow::Result<()> {
    let version: String = sqlx::query_scalar("SELECT sqlite_version()")
        .fetch_one(&mut *connection)
        .await?;
    database::ensure_safe_sqlite_version(&version)?;
    // One consistent snapshot for metadata, structure and relationship checks.
    let mut tx = connection.begin().await?;
    let results = sqlx::query_scalar::<_, String>("PRAGMA integrity_check")
        .fetch_all(&mut *tx)
        .await?;
    anyhow::ensure!(
        results.as_slice() == ["ok"],
        "SQLite integrity check failed"
    );
    anyhow::ensure!(
        sqlx::query("PRAGMA foreign_key_check")
            .fetch_optional(&mut *tx)
            .await?
            .is_none(),
        "database contains foreign key violations"
    );
    let ledger: Vec<(i64, bool, Vec<u8>)> =
        sqlx::query_as("SELECT version,success,checksum FROM _sqlx_migrations ORDER BY version")
            .fetch_all(&mut *tx)
            .await?;
    let expected_ledger: Vec<_> = MIGRATOR
        .iter()
        .map(|m| (m.version, true, m.checksum.to_vec()))
        .collect();
    anyhow::ensure!(
        ledger == expected_ledger,
        "unsupported or incomplete migration metadata"
    );
    let actual: Vec<SchemaObject> = sqlx::query_as(SCHEMA_SQL).fetch_all(&mut *tx).await?;
    let mut expected = SqliteConnection::connect("sqlite::memory:").await?;
    MIGRATOR.run(&mut expected).await?;
    let expected_schema: Vec<SchemaObject> =
        sqlx::query_as(SCHEMA_SQL).fetch_all(&mut expected).await?;
    expected.close().await?;
    anyhow::ensure!(
        actual == expected_schema,
        "database schema differs from current migrations"
    );
    tx.commit().await?;
    Ok(())
}

pub async fn verify_database(path: &Path) -> anyhow::Result<()> {
    let options = SqliteConnectOptions::new()
        .filename(path)
        .read_only(true)
        .create_if_missing(false)
        .busy_timeout(Duration::from_secs(30))
        .foreign_keys(true);
    let mut connection = SqliteConnection::connect_with(&options).await?;
    let result = verify_connection(&mut connection).await;
    connection.close().await?;
    result
}

/// Both bots must be stopped before restoring. Never replaces an existing path.
pub async fn restore_database(backup: &Path, destination: &Path) -> anyhow::Result<()> {
    let partial = prepare_destination(destination)?;
    verify_database(backup).await?;
    std::fs::copy(backup, &partial)?;
    verify_database(&partial)
        .await
        .context("restored copy did not pass verification")?;
    finalize(partial, destination)
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
    #[test]
    fn competing_publish_never_overwrites_a_destination() {
        let dir = tempdir().unwrap();
        let destination = dir.path().join("result.db");
        let first = prepare_destination(&destination).unwrap();
        let second = prepare_destination(&destination).unwrap();
        std::fs::write(&first, b"first verified snapshot").unwrap();
        std::fs::write(&second, b"second verified snapshot").unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let tasks: Vec<_> = [first, second]
            .into_iter()
            .map(|partial| {
                let barrier = barrier.clone();
                let destination = destination.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    finalize(partial, &destination).is_ok()
                })
            })
            .collect();
        let wins = tasks
            .into_iter()
            .map(|task| usize::from(task.join().unwrap()))
            .sum::<usize>();
        assert_eq!(wins, 1);
        let bytes = std::fs::read(&destination).unwrap();
        assert!(bytes == b"first verified snapshot" || bytes == b"second verified snapshot");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn file_created_after_staging_is_preserved() {
        let dir = tempdir().unwrap();
        let destination = dir.path().join("result.db");
        let partial = prepare_destination(&destination).unwrap();
        std::fs::write(&partial, b"snapshot").unwrap();
        std::fs::write(&destination, b"competing file").unwrap();
        assert!(finalize(partial, &destination).is_err());
        assert_eq!(std::fs::read(&destination).unwrap(), b"competing file");
    }

    #[tokio::test]
    async fn rejects_invalid_schema_ledger_and_foreign_keys_without_publishing() {
        for damage in [
            "DROP TABLE attendance_panels",
            "DROP INDEX attendance_panel_receipts_owner",
            "CREATE TRIGGER sqliteX_extra AFTER INSERT ON attendance_sessions BEGIN SELECT 1; END",
            "UPDATE _sqlx_migrations SET success=0",
            "UPDATE _sqlx_migrations SET checksum=x'00'",
            "DELETE FROM _sqlx_migrations",
            "DROP TABLE _sqlx_migrations; CREATE TABLE _sqlx_migrations(version INTEGER)",
            "INSERT INTO attendance_auto_end_events(session_id,guild_id,user_id,automatic_ended_at,applied_at,change_id_at_application) VALUES(999,1,2,100,100,0)",
        ] {
            let dir = tempdir().unwrap();
            let source = dir.path().join("source.db");
            let options = SqliteConnectOptions::new()
                .filename(&source)
                .create_if_missing(true)
                .foreign_keys(false);
            let mut conn = SqliteConnection::connect_with(&options).await.unwrap();
            MIGRATOR.run(&mut conn).await.unwrap();
            sqlx::raw_sql(damage).execute(&mut conn).await.unwrap();
            conn.close().await.unwrap();
            assert!(verify_database(&source).await.is_err(), "{damage}");
            let destination = dir.path().join("output.db");
            assert!(
                backup_database(&source, &destination).await.is_err(),
                "{damage}"
            );
            assert!(!destination.exists());
            assert!(
                restore_database(&source, &destination).await.is_err(),
                "{damage}"
            );
            assert!(!destination.exists());
            assert!(!std::fs::read_dir(dir.path()).unwrap().any(|entry| {
                entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".attendance-")
            }));
        }
    }
}
