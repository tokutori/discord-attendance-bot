//! Real subprocess failures against a temporary DB; no Discord client or credentials.
use std::{
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use discord_attendance_bot::{attendance, repository};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};

#[tokio::test]
async fn readonly_child() {
    let Some(path) = std::env::var_os("ATTENDANCE_ISOLATION_TEST_DB") else {
        return;
    };
    let path = std::path::PathBuf::from(path);
    let reader = attendance_query::ReadDatabase::open_file(&path)
        .await
        .unwrap();
    assert_eq!(
        attendance_query::active_sessions(&reader, 1)
            .await
            .unwrap()
            .len(),
        1
    );
    std::fs::write(path.with_extension("ready"), b"ready").unwrap();
    if std::env::var_os("ATTENDANCE_ISOLATION_TEST_PANIC").is_some() {
        panic!("intentional presentation process failure");
    }
    tokio::time::sleep(Duration::from_secs(30)).await;
    panic!("parent did not stop the test view");
}

#[tokio::test]
async fn recording_continues_when_view_panics_or_is_killed() {
    for panic in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dummy.db");
        let pool = SqlitePoolOptions::new()
            .max_connections(2)
            .connect_with(
                SqliteConnectOptions::new()
                    .filename(&path)
                    .create_if_missing(true)
                    .journal_mode(SqliteJournalMode::Wal),
            )
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        attendance::start(&pool, 1, 2, "dummy", 10, None, 10)
            .await
            .unwrap();
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", "readonly_child", "--nocapture"])
            .env("ATTENDANCE_ISOLATION_TEST_DB", &path)
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if panic {
            command.env("ATTENDANCE_ISOLATION_TEST_PANIC", "1");
        }
        let mut child = command.spawn().unwrap();
        let deadline = Instant::now() + Duration::from_secs(15);
        while !path.with_extension("ready").exists() {
            if child.try_wait().unwrap().is_some() || Instant::now() > deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("test view did not read the live WAL database");
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        if !panic {
            // The writer commits while the separate view process is alive.
            attendance::start(&pool, 1, 3, "another dummy", 11, None, 11)
                .await
                .unwrap();
            child.kill().unwrap();
        }
        assert!(!child.wait().unwrap().success());
        assert!(matches!(
            attendance::end(&pool, 1, 2, 20, None, 20).await.unwrap(),
            attendance::EndOutcome::Ended(_)
        ));
        assert!(matches!(
            attendance::continue_activity(&pool, 1, 2, 21)
                .await
                .unwrap(),
            attendance::ContinueOutcome::Continued { .. }
        ));
        assert!(
            repository::open_session(&pool, 1, 2)
                .await
                .unwrap()
                .is_some()
        );
        let replacement = attendance_query::ReadDatabase::open_file(&path)
            .await
            .unwrap();
        assert!(
            attendance_query::open_session(&replacement, 1, 2)
                .await
                .unwrap()
                .is_some()
        );
        drop(replacement);
        pool.close().await;
    }
}
