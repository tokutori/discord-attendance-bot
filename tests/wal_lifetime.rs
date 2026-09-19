use std::time::Duration;

use discord_attendance_bot::wal_anchor::WalAnchor;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};

#[tokio::test]
async fn anchor_survives_pool_reaping_without_pinning_a_read_transaction() {
    exercise_reaping(true).await;
    exercise_reaping(false).await;
}

async fn exercise_reaping(idle: bool) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dummy.db");
    let options = SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal);
    let anchor = WalAnchor::open(&options).await.unwrap();
    let pool = SqlitePoolOptions::new()
        .max_connections(2)
        .min_connections(0)
        .idle_timeout(idle.then_some(Duration::from_millis(50)))
        .max_lifetime((!idle).then_some(Duration::from_millis(50)))
        .connect_with(options)
        .await
        .unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();
    for user in 1..=3 {
        discord_attendance_bot::attendance::start(&pool, 1, user, "dummy", 10, None, 10)
            .await
            .unwrap();
        // Anchor must not pin a snapshot or prevent WAL truncation.
        let checkpoint: (i64, i64, i64) = sqlx::query_as("PRAGMA wal_checkpoint(TRUNCATE)")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(checkpoint, (0, 0, 0));
        tokio::time::timeout(Duration::from_secs(10), async {
            while pool.size() != 0 {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        assert!(path.with_extension("db-wal").is_file());
        assert!(path.with_extension("db-shm").is_file());
        // No writer access between observed pool size=0 and reader startup.
        let reader = attendance_query::ReadDatabase::open_file(&path)
            .await
            .unwrap();
        assert_eq!(
            attendance_query::active_sessions(&reader, 1)
                .await
                .unwrap()
                .len(),
            user as usize
        );
        drop(reader);
    }
    pool.close().await;
    anchor.close().await.unwrap();
}
