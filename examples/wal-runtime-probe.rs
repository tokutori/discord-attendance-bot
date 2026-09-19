//! Offline Docker regression probe. Uses only a disposable directory and dummy records.
//! Never starts a Discord client or loads environment/configuration files.
use std::{fs, path::Path, time::Duration};

use anyhow::{Context, ensure};
use discord_attendance_bot::{attendance, wal_anchor::WalAnchor};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};

async fn wait_for(path: &Path) -> anyhow::Result<()> {
    tokio::time::timeout(Duration::from_secs(120), async {
        while !path.exists() {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .context("probe synchronization timed out")?;
    Ok(())
}

async fn writer(dir: &Path, anchored: bool) -> anyhow::Result<()> {
    let options = SqliteConnectOptions::new()
        .filename(dir.join("dummy.db"))
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal);
    let anchor = if anchored {
        Some(WalAnchor::open(&options).await?)
    } else {
        None
    };
    let pool = SqlitePoolOptions::new()
        .max_connections(2)
        .min_connections(0)
        .idle_timeout(Duration::from_millis(50))
        .max_lifetime(Duration::from_millis(100))
        .connect_with(options)
        .await?;
    let version: String = sqlx::query_scalar("SELECT sqlite_version()")
        .fetch_one(&pool)
        .await?;
    ensure!(
        version == "3.51.3",
        "update probe evidence for SQLite {version}"
    );
    println!("SQLite {version}, SQLx 0.9, anchored={anchored}");
    sqlx::migrate!("./migrations").run(&pool).await?;
    for stage in 0..3 {
        attendance::start(&pool, 1, stage + 1, "dummy", 10, None, 10).await?;
        let checkpoint: (i64, i64, i64) = sqlx::query_as("PRAGMA wal_checkpoint(TRUNCATE)")
            .fetch_one(&pool)
            .await?;
        ensure!(
            checkpoint == (0, 0, 0),
            "anchor or reader blocks truncation"
        );
        tokio::time::timeout(Duration::from_secs(10), async {
            while pool.size() != 0
                || (!anchored
                    && (dir.join("dummy.db-wal").exists() || dir.join("dummy.db-shm").exists()))
            {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .context("physical pool connections were not reaped")?;
        ensure!(
            dir.join("dummy.db-wal").exists() == anchored,
            "unexpected WAL lifecycle"
        );
        ensure!(
            dir.join("dummy.db-shm").exists() == anchored,
            "unexpected SHM lifecycle"
        );
        // No writer access until the controller has tested the readonly mount.
        fs::write(dir.join(format!("ready-{stage}")), b"pool-size=0")?;
        println!("stage {stage}: pool-size=0; awaiting reader replacement");
        wait_for(&dir.join(format!("advance-{stage}"))).await?;
        if !anchored {
            break;
        }
    }
    pool.close().await;
    if let Some(anchor) = anchor {
        anchor.close().await?;
    }
    Ok(())
}

async fn reader(dir: &Path, hold: bool, expected: usize) -> anyhow::Result<()> {
    let reader = attendance_query::ReadDatabase::open_file(&dir.join("dummy.db")).await?;
    ensure!(
        attendance_query::active_sessions(&reader, 1).await?.len() == expected,
        "record count differs after view restart"
    );
    // Actual file opens, not SQLx permission checks. A mistakenly rw mount fails this probe.
    for name in ["dummy.db", "dummy.db-wal", "dummy.db-shm"] {
        let error = fs::OpenOptions::new()
            .write(true)
            .open(dir.join(name))
            .err()
            .context("OS allowed a write-capable file open")?;
        ensure!(
            matches!(error.raw_os_error(), Some(13 | 30)),
            "unexpected OS error: {error}"
        );
    }
    let error = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(dir.join("forbidden"))
        .err()
        .context("OS allowed file creation")?;
    ensure!(
        matches!(error.raw_os_error(), Some(13 | 30)),
        "unexpected OS error: {error}"
    );
    println!("READER_READY records={expected}; OS denied DB/WAL/SHM writes and file creation");
    if hold {
        tokio::time::sleep(Duration::from_secs(120)).await;
    }
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).context("expected probe mode")?;
    let dir = Path::new(args.get(2).context("expected disposable directory")?);
    match mode.as_str() {
        "writer" => writer(dir, true).await,
        "unanchored" => writer(dir, false).await,
        "reader" | "reader-hold" => {
            reader(
                dir,
                mode == "reader-hold",
                args.get(3).context("expected count")?.parse()?,
            )
            .await
        }
        "signal" => {
            fs::write(
                dir.join(args.get(3).context("expected marker name")?),
                b"go",
            )?;
            Ok(())
        }
        _ => anyhow::bail!("unknown probe mode"),
    }
}
