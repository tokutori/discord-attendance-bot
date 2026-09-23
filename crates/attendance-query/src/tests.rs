use super::*;
use sqlx::sqlite::SqliteJournalMode;

async fn fixture(path: &Path) -> SqlitePool {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            SqliteConnectOptions::new()
                .filename(path)
                .create_if_missing(true)
                .journal_mode(SqliteJournalMode::Wal),
        )
        .await
        .unwrap();
    sqlx::migrate!("../../migrations").run(&pool).await.unwrap();
    pool
}

#[tokio::test]
async fn every_reader_connection_rejects_writes_even_without_query_only() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dummy.db");
    let writer = fixture(&path).await;
    let reader = ReadDatabase::open_file(&path).await.unwrap();
    let mut first = reader.pool.acquire().await.unwrap();
    let mut second = reader.pool.acquire().await.unwrap();
    for connection in [&mut first, &mut second] {
        sqlx::query("PRAGMA query_only=OFF")
            .execute(&mut **connection)
            .await
            .unwrap();
        for statement in [
            "INSERT INTO attendance_user_profiles (guild_id,user_id,created_at,updated_at) VALUES (1,2,0,0)",
            "UPDATE attendance_sessions SET note='broken'",
            "DELETE FROM attendance_sessions",
            "DROP TABLE attendance_sessions",
            "PRAGMA user_version=99",
        ] {
            let error = sqlx::query(statement)
                .execute(&mut **connection)
                .await
                .unwrap_err();
            assert_eq!(
                error.as_database_error().unwrap().code().as_deref(),
                Some("8"),
                "{statement}: {error}"
            );
        }
    }
    drop((first, second));
    reader.pool.close().await;
    writer.close().await;
}

#[tokio::test]
async fn reader_observes_live_wal_and_does_not_acknowledge_notices() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dummy.db");
    let writer = fixture(&path).await;
    // URL flags must not enable immutable mode or writable access.
    let url = format!(
        "sqlite://{}?immutable=true&mode=rwc",
        path.to_string_lossy().replace('\\', "/")
    );
    let reader = ReadDatabase::open(&url).await.unwrap();
    assert!(active_sessions(&reader, 1).await.unwrap().is_empty());
    sqlx::query("INSERT INTO attendance_sessions (guild_id,user_id,display_name,started_at,open_since,created_at,updated_at) VALUES (1,2,'dummy',10,10,10,10)").execute(&writer).await.unwrap();
    let sessions = active_sessions(&reader, 1).await.unwrap();
    assert_eq!(sessions.len(), 1);
    assert!(active_sessions(&reader, 9).await.unwrap().is_empty());
    sqlx::query("INSERT INTO attendance_auto_end_events (session_id,guild_id,user_id,automatic_ended_at,applied_at,change_id_at_application) VALUES (?,1,2,20,20,0)").bind(sessions[0].id).execute(&writer).await.unwrap();
    let notice = peek_auto_end_notice(&reader, 1, 2).await.unwrap().unwrap();
    assert_eq!(
        peek_auto_end_notice(&reader, 1, 2)
            .await
            .unwrap()
            .unwrap()
            .event_id,
        notice.event_id
    );
    reader.pool.close().await;
    // Replacing only the view retains access to existing live records.
    let replacement = ReadDatabase::open_file(&path).await.unwrap();
    assert_eq!(history(&replacement, 1, 2, 5).await.unwrap().len(), 1);
    replacement.pool.close().await;
    writer.close().await;
}

#[tokio::test]
async fn missing_unmigrated_or_incompatible_database_is_rejected_without_migration() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dummy.db");
    assert!(ReadDatabase::open_file(&path).await.is_err());
    assert!(!path.exists());
    let writer = SqlitePoolOptions::new()
        .connect_with(
            SqliteConnectOptions::new()
                .filename(&path)
                .create_if_missing(true),
        )
        .await
        .unwrap();
    assert!(ReadDatabase::open_file(&path).await.is_err());
    let tables: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_master WHERE type='table'")
        .fetch_one(&writer)
        .await
        .unwrap();
    assert_eq!(tables, 0);
    sqlx::migrate!("../../migrations")
        .run(&writer)
        .await
        .unwrap();
    sqlx::query("UPDATE _sqlx_migrations SET version=99 WHERE version=1")
        .execute(&writer)
        .await
        .unwrap();
    assert!(
        ReadDatabase::open_file(&path)
            .await
            .err()
            .unwrap()
            .to_string()
            .contains("unsupported attendance schema")
    );
    writer.close().await;
}

#[tokio::test]
async fn initial_schema_requires_panel_tables_and_successful_ledger() {
    for statement in [
        "DROP TABLE attendance_panels",
        "DROP TABLE attendance_panel_receipts",
        "UPDATE _sqlx_migrations SET success=false WHERE version=1",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dummy.db");
        let writer = fixture(&path).await;
        sqlx::query(statement).execute(&writer).await.unwrap();
        assert!(ReadDatabase::open_file(&path).await.is_err(), "{statement}");
        writer.close().await;
    }
}
