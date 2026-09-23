use discord_attendance_bot::database::require_persistent_database;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use std::str::FromStr;

#[tokio::test]
async fn backing_file_check_rejects_memory_and_accepts_durable_records() {
    for url in [
        "sqlite::memory:?cache=shared",
        "sqlite://dummy?mode=mem%6fry",
        "sqlite://%3Amemory%3A",
        "sqlite:///dummy?vfs=memdb",
    ] {
        let options = SqliteConnectOptions::from_str(url)
            .unwrap()
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .unwrap();
        assert!(require_persistent_database(&pool).await.is_err(), "{url}");
        pool.close().await;
    }
    let dir = tempfile::tempdir().unwrap();
    let options = SqliteConnectOptions::new()
        .filename(dir.path().join("dummy.db"))
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options.clone())
        .await
        .unwrap();
    require_persistent_database(&pool).await.unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();
    discord_attendance_bot::attendance::start(&pool, 1, 2, "dummy", 100, None, 100)
        .await
        .unwrap();
    pool.close().await;
    let reopened = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap();
    require_persistent_database(&reopened).await.unwrap();
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM attendance_sessions")
        .fetch_one(&reopened)
        .await
        .unwrap();
    assert_eq!(count, 1);
    reopened.close().await;
}
