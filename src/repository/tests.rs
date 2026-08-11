use chrono::TimeZone;
use std::time::Duration;

use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
};

use super::*;

async fn test_pool() -> SqlitePool {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();
    pool
}

async fn wal_test_pool() -> (tempfile::TempDir, SqlitePool) {
    let directory = tempfile::tempdir().unwrap();
    let options = SqliteConnectOptions::new()
        .filename(directory.path().join("attendance.db"))
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(Duration::from_secs(5))
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await
        .unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();
    (directory, pool)
}

async fn confirm_latest_revert(pool: &SqlitePool, now: i64) -> ConfirmationResult {
    let preview = match latest_revert_preview(pool, 1, 2).await.unwrap() {
        RevertPreviewResult::Available(preview) => preview,
        result => panic!("unexpected preview result: {result:?}"),
    };
    let request = create_confirmation(
        pool,
        1,
        2,
        ConfirmationInput {
            action: "revert",
            session_id: preview.session_id,
            change_id: Some(preview.change_id),
            expected: &preview.current,
            target_started_at: None,
            target_ended_at: None,
            target_note: None,
            requested_at: now,
        },
    )
    .await
    .unwrap();
    confirm_confirmation(pool, 1, 2, &request.code, now + 1)
        .await
        .unwrap()
}

#[tokio::test]
async fn confirmed_reverts_walk_all_changes_in_reverse_order() {
    let pool = test_pool().await;
    let id = insert_session(&pool, 1, 2, "Bem", 100, Some("start"), 100)
        .await
        .unwrap();
    close_session(&pool, id, 1, 2, 200, Some("end"), 200)
        .await
        .unwrap();
    reopen_session(&pool, id, 1, 2, 300).await.unwrap();
    let before_edit = SnapshotRow {
        started_at: 100,
        ended_at: None,
        open_since: Some(300),
        note: Some("end".into()),
        deleted_at: None,
    };
    let edit_request = create_confirmation(
        &pool,
        1,
        2,
        ConfirmationInput {
            action: "edit",
            session_id: id,
            change_id: None,
            expected: &before_edit,
            target_started_at: Some(150),
            target_ended_at: Some(250),
            target_note: Some("edit"),
            requested_at: 400,
        },
    )
    .await
    .unwrap();
    assert!(matches!(
        confirm_confirmation(&pool, 1, 2, &edit_request.code, 401)
            .await
            .unwrap(),
        ConfirmationResult::Confirmed { .. }
    ));

    let before_delete = SnapshotRow {
        started_at: 150,
        ended_at: Some(250),
        open_since: None,
        note: Some("edit".into()),
        deleted_at: None,
    };
    let delete_request = create_confirmation(
        &pool,
        1,
        2,
        ConfirmationInput {
            action: "delete",
            session_id: id,
            change_id: None,
            expected: &before_delete,
            target_started_at: None,
            target_ended_at: None,
            target_note: None,
            requested_at: 500,
        },
    )
    .await
    .unwrap();
    assert!(matches!(
        confirm_confirmation(&pool, 1, 2, &delete_request.code, 501)
            .await
            .unwrap(),
        ConfirmationResult::Confirmed { .. }
    ));

    for (now, operation) in [
        (600, "delete"),
        (700, "edit"),
        (800, "continue"),
        (900, "end"),
        (1_000, "start"),
    ] {
        assert_eq!(
            confirm_latest_revert(&pool, now).await,
            ConfirmationResult::Confirmed {
                action: "revert".into(),
                session_id: id,
                operation: Some(operation.into()),
            }
        );
    }

    assert!(get_owned(&pool, id, 1, 2).await.unwrap().is_none());
    assert_eq!(
        latest_revert_preview(&pool, 1, 2).await.unwrap(),
        RevertPreviewResult::NothingToRevert
    );
}

#[tokio::test]
async fn stale_revert_confirmation_cannot_skip_a_newer_change() {
    let pool = test_pool().await;
    let first = insert_session(&pool, 1, 2, "Bem", 100, None, 100)
        .await
        .unwrap();
    close_session(&pool, first, 1, 2, 200, None, 200)
        .await
        .unwrap();

    let preview = match latest_revert_preview(&pool, 1, 2).await.unwrap() {
        RevertPreviewResult::Available(preview) => preview,
        result => panic!("unexpected preview result: {result:?}"),
    };
    let stale_request = create_confirmation(
        &pool,
        1,
        2,
        ConfirmationInput {
            action: "revert",
            session_id: preview.session_id,
            change_id: Some(preview.change_id),
            expected: &preview.current,
            target_started_at: None,
            target_ended_at: None,
            target_note: None,
            requested_at: 210,
        },
    )
    .await
    .unwrap();

    let second = insert_session(&pool, 1, 2, "Bem", 300, None, 220)
        .await
        .unwrap();
    close_session(&pool, second, 1, 2, 400, None, 230)
        .await
        .unwrap();
    let second_snapshot = SnapshotRow {
        started_at: 300,
        ended_at: Some(400),
        open_since: None,
        note: None,
        deleted_at: None,
    };
    let delete_request = create_confirmation(
        &pool,
        1,
        2,
        ConfirmationInput {
            action: "delete",
            session_id: second,
            change_id: None,
            expected: &second_snapshot,
            target_started_at: None,
            target_ended_at: None,
            target_note: None,
            requested_at: 235,
        },
    )
    .await
    .unwrap();
    assert!(matches!(
        confirm_confirmation(&pool, 1, 2, &delete_request.code, 236)
            .await
            .unwrap(),
        ConfirmationResult::Confirmed { .. }
    ));

    assert_eq!(
        confirm_confirmation(&pool, 1, 2, &stale_request.code, 240)
            .await
            .unwrap(),
        ConfirmationResult::Conflict
    );
    assert_eq!(
        get_owned(&pool, first, 1, 2)
            .await
            .unwrap()
            .unwrap()
            .ended_at,
        Some(200)
    );
    assert_eq!(
        confirm_confirmation(&pool, 1, 2, &stale_request.code, 241)
            .await
            .unwrap(),
        ConfirmationResult::NotFound
    );
}

#[tokio::test]
async fn confirmation_is_preview_only_until_confirmed_and_single_use() {
    let pool = test_pool().await;
    let id = insert_session(&pool, 1, 2, "Bem", 100, Some("old"), 100)
        .await
        .unwrap();
    let expected = SnapshotRow {
        started_at: 100,
        ended_at: None,
        open_since: Some(100),
        note: Some("old".into()),
        deleted_at: None,
    };
    let request = create_confirmation(
        &pool,
        1,
        2,
        ConfirmationInput {
            action: "edit",
            session_id: id,
            change_id: None,
            expected: &expected,
            target_started_at: Some(150),
            target_ended_at: Some(250),
            target_note: Some("new"),
            requested_at: 200,
        },
    )
    .await
    .unwrap();
    assert_eq!(request.code.len(), 5);
    assert_eq!(
        get_owned(&pool, id, 1, 2)
            .await
            .unwrap()
            .unwrap()
            .started_at,
        100
    );

    assert_eq!(
        confirm_confirmation(&pool, 1, 2, &request.code, 201)
            .await
            .unwrap(),
        ConfirmationResult::Confirmed {
            action: "edit".into(),
            session_id: id,
            operation: None,
        }
    );
    assert_eq!(
        confirm_confirmation(&pool, 1, 2, &request.code, 202)
            .await
            .unwrap(),
        ConfirmationResult::NotFound
    );

    let expected = SnapshotRow {
        started_at: 150,
        ended_at: Some(250),
        open_since: None,
        note: Some("new".into()),
        deleted_at: None,
    };
    let delete_request = create_confirmation(
        &pool,
        1,
        2,
        ConfirmationInput {
            action: "delete",
            session_id: id,
            change_id: None,
            expected: &expected,
            target_started_at: None,
            target_ended_at: None,
            target_note: None,
            requested_at: 300,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        confirm_confirmation(&pool, 1, 2, &delete_request.code, 301)
            .await
            .unwrap(),
        ConfirmationResult::Confirmed {
            action: "delete".into(),
            session_id: id,
            operation: None,
        }
    );

    assert_eq!(
        confirm_latest_revert(&pool, 400).await,
        ConfirmationResult::Confirmed {
            action: "revert".into(),
            session_id: id,
            operation: Some("delete".into()),
        }
    );
    assert!(get_owned(&pool, id, 1, 2).await.unwrap().is_some());
}

#[tokio::test]
async fn auto_end_runs_after_midnight_and_user_end_corrects_it() {
    let pool = test_pool().await;
    let start = crate::time::DISPLAY_TIMEZONE
        .with_ymd_and_hms(2026, 8, 8, 18, 0, 0)
        .unwrap()
        .timestamp();
    let now = crate::time::DISPLAY_TIMEZONE
        .with_ymd_and_hms(2026, 8, 9, 0, 0, 1)
        .unwrap()
        .timestamp();
    let manual_end = crate::time::DISPLAY_TIMEZONE
        .with_ymd_and_hms(2026, 8, 8, 20, 45, 0)
        .unwrap()
        .timestamp();
    let id = insert_session(&pool, 1, 2, "Bem", start, None, start)
        .await
        .unwrap();

    let notices = apply_due_auto_ends(&pool, 1, now).await.unwrap();
    assert_eq!(notices.len(), 1);
    let session = get_owned(&pool, id, 1, 2).await.unwrap().unwrap();
    assert_eq!(
        session.ended_at,
        Some(
            crate::time::DISPLAY_TIMEZONE
                .with_ymd_and_hms(2026, 8, 8, 21, 0, 0)
                .unwrap()
                .timestamp()
        )
    );
    assert_eq!(apply_due_auto_ends(&pool, 1, now).await.unwrap().len(), 0);

    let notice = peek_auto_end_notice(&pool, 1, 2).await.unwrap().unwrap();
    assert_eq!(notice.session_id, id);
    assert!(
        acknowledge_auto_end_notice(&pool, notice.event_id, 1, 2, now)
            .await
            .unwrap()
    );
    assert!(peek_auto_end_notice(&pool, 1, 2).await.unwrap().is_none());

    let correction =
        correct_auto_ended_session(&pool, id, 1, 2, manual_end, Some("manual"), now + 1)
            .await
            .unwrap()
            .unwrap();
    assert_eq!(correction.session.ended_at, Some(manual_end));
    assert!(latest_auto_ended(&pool, 1, 2).await.unwrap().is_none());
}

#[tokio::test]
async fn same_session_can_be_auto_ended_again_after_continue() {
    let pool = test_pool().await;
    let start = crate::time::DISPLAY_TIMEZONE
        .with_ymd_and_hms(2026, 8, 8, 18, 0, 0)
        .unwrap()
        .timestamp();
    let first_midnight = crate::time::DISPLAY_TIMEZONE
        .with_ymd_and_hms(2026, 8, 9, 0, 0, 1)
        .unwrap()
        .timestamp();
    let continued_at = crate::time::DISPLAY_TIMEZONE
        .with_ymd_and_hms(2026, 8, 9, 1, 0, 0)
        .unwrap()
        .timestamp();
    let second_midnight = crate::time::DISPLAY_TIMEZONE
        .with_ymd_and_hms(2026, 8, 10, 0, 0, 1)
        .unwrap()
        .timestamp();
    let second_cutoff = crate::time::DISPLAY_TIMEZONE
        .with_ymd_and_hms(2026, 8, 9, 21, 0, 0)
        .unwrap()
        .timestamp();
    let id = insert_session(&pool, 1, 2, "Bem", start, None, start)
        .await
        .unwrap();

    assert_eq!(
        apply_due_auto_ends(&pool, 1, first_midnight)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        reopen_session(&pool, id, 1, 2, continued_at).await.unwrap(),
        1
    );
    assert!(latest_auto_ended(&pool, 1, 2).await.unwrap().is_none());
    assert_eq!(
        apply_due_auto_ends(&pool, 1, second_midnight)
            .await
            .unwrap()
            .len(),
        1
    );

    let session = get_owned(&pool, id, 1, 2).await.unwrap().unwrap();
    assert_eq!(session.ended_at, Some(second_cutoff));
    let event_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM attendance_auto_end_events WHERE session_id = ?")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(event_count, 2);
}

#[tokio::test]
async fn completed_manual_end_is_not_reclassified_as_old_auto_end() {
    let pool = test_pool().await;
    let start = crate::time::DISPLAY_TIMEZONE
        .with_ymd_and_hms(2026, 8, 8, 18, 0, 0)
        .unwrap()
        .timestamp();
    let midnight = crate::time::DISPLAY_TIMEZONE
        .with_ymd_and_hms(2026, 8, 9, 0, 0, 1)
        .unwrap()
        .timestamp();
    let continued_at = crate::time::DISPLAY_TIMEZONE
        .with_ymd_and_hms(2026, 8, 9, 10, 0, 0)
        .unwrap()
        .timestamp();
    let manual_end = crate::time::DISPLAY_TIMEZONE
        .with_ymd_and_hms(2026, 8, 9, 20, 0, 0)
        .unwrap()
        .timestamp();
    let id = insert_session(&pool, 1, 2, "Bem", start, None, start)
        .await
        .unwrap();
    apply_due_auto_ends(&pool, 1, midnight).await.unwrap();
    reopen_session(&pool, id, 1, 2, continued_at).await.unwrap();
    close_session(&pool, id, 1, 2, manual_end, None, manual_end)
        .await
        .unwrap();

    assert!(latest_auto_ended(&pool, 1, 2).await.unwrap().is_none());
    let outcome = crate::attendance::end(&pool, 1, 2, manual_end, None, manual_end)
        .await
        .unwrap();
    assert!(matches!(
        outcome,
        crate::attendance::EndOutcome::AlreadyInactive(_)
    ));
    assert_eq!(
        get_owned(&pool, id, 1, 2).await.unwrap().unwrap().ended_at,
        Some(manual_end)
    );
}

#[tokio::test]
async fn later_session_supersedes_an_old_automatic_end_correction() {
    let pool = test_pool().await;
    let first_start = 100;
    let automatic_end = 200;
    let first_id = insert_session(&pool, 1, 2, "Bem", first_start, None, first_start)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE attendance_sessions SET ended_at = ?, open_since = NULL, updated_at = ?
         WHERE id = ?",
    )
    .bind(automatic_end)
    .bind(automatic_end)
    .bind(first_id)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO attendance_auto_end_events
            (session_id, guild_id, user_id, automatic_ended_at, applied_at,
             change_id_at_application)
         VALUES (?, 1, 2, ?, ?, 1)",
    )
    .bind(first_id)
    .bind(automatic_end)
    .bind(automatic_end)
    .execute(&pool)
    .await
    .unwrap();

    // Change IDs, rather than second-resolution timestamps, define the strict ordering.
    let second_id = insert_session(&pool, 1, 2, "Bem", 300, None, automatic_end)
        .await
        .unwrap();
    close_session(&pool, second_id, 1, 2, 400, None, automatic_end)
        .await
        .unwrap();

    assert!(latest_auto_ended(&pool, 1, 2).await.unwrap().is_none());
    assert!(
        correct_auto_ended_session(&pool, first_id, 1, 2, 500, None, 500)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        get_owned(&pool, first_id, 1, 2)
            .await
            .unwrap()
            .unwrap()
            .ended_at,
        Some(automatic_end)
    );
}

#[tokio::test]
async fn confirm_open_transition_conflicts_with_newer_open_session() {
    let pool = test_pool().await;
    let first = insert_session(&pool, 1, 2, "Bem", 100, None, 100)
        .await
        .unwrap();
    close_session(&pool, first, 1, 2, 200, None, 200)
        .await
        .unwrap();
    let expected = SnapshotRow {
        started_at: 100,
        ended_at: Some(200),
        open_since: None,
        note: None,
        deleted_at: None,
    };
    let request = create_confirmation(
        &pool,
        1,
        2,
        ConfirmationInput {
            action: "edit",
            session_id: first,
            change_id: None,
            expected: &expected,
            target_started_at: Some(100),
            target_ended_at: None,
            target_note: None,
            requested_at: 300,
        },
    )
    .await
    .unwrap();
    insert_session(&pool, 1, 2, "Bem", 300, None, 300)
        .await
        .unwrap();

    assert_eq!(
        confirm_confirmation(&pool, 1, 2, &request.code, 301)
            .await
            .unwrap(),
        ConfirmationResult::Conflict
    );
    assert_eq!(
        confirm_confirmation(&pool, 1, 2, &request.code, 302)
            .await
            .unwrap(),
        ConfirmationResult::NotFound
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_confirm_is_single_use_on_wal_pool() {
    let (_directory, pool) = wal_test_pool().await;
    let id = insert_session(&pool, 1, 2, "Bem", 100, None, 100)
        .await
        .unwrap();
    let expected = SnapshotRow {
        started_at: 100,
        ended_at: None,
        open_since: Some(100),
        note: None,
        deleted_at: None,
    };
    let request = create_confirmation(
        &pool,
        1,
        2,
        ConfirmationInput {
            action: "edit",
            session_id: id,
            change_id: None,
            expected: &expected,
            target_started_at: Some(110),
            target_ended_at: Some(200),
            target_note: Some("confirmed once"),
            requested_at: 300,
        },
    )
    .await
    .unwrap();
    let first_pool = pool.clone();
    let second_pool = pool.clone();
    let first_code = request.code.clone();
    let second_code = request.code.clone();
    let (first, second) = tokio::join!(
        confirm_confirmation(&first_pool, 1, 2, &first_code, 301),
        confirm_confirmation(&second_pool, 1, 2, &second_code, 301),
    );
    let results = [first.unwrap(), second.unwrap()];
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, ConfirmationResult::Confirmed { .. }))
            .count(),
        1
    );
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, ConfirmationResult::NotFound))
            .count(),
        1
    );
}

#[tokio::test]
async fn composite_foreign_keys_reject_mismatched_owners() {
    let pool = test_pool().await;
    let id = insert_session(&pool, 1, 2, "Bem", 100, None, 100)
        .await
        .unwrap();
    let result = sqlx::query(
        "INSERT INTO attendance_changes (
            guild_id, user_id, session_id, kind,
            after_started_at, after_open_since, created_at
         ) VALUES (999, 888, ?, 'start', 100, 100, 100)",
    )
    .bind(id)
    .execute(&pool)
    .await;
    assert!(result.is_err());
    let foreign_key_errors: Vec<String> = sqlx::query_scalar("PRAGMA foreign_key_check")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert!(foreign_key_errors.is_empty());
}

#[tokio::test]
async fn overlapping_completed_edit_is_consumed_as_conflict() {
    let pool = test_pool().await;
    let first = insert_session(&pool, 1, 2, "Bem", 100, None, 100)
        .await
        .unwrap();
    close_session(&pool, first, 1, 2, 200, None, 200)
        .await
        .unwrap();
    let second = insert_session(&pool, 1, 2, "Bem", 300, None, 300)
        .await
        .unwrap();
    close_session(&pool, second, 1, 2, 400, None, 400)
        .await
        .unwrap();
    let expected = SnapshotRow {
        started_at: 300,
        ended_at: Some(400),
        open_since: None,
        note: None,
        deleted_at: None,
    };
    let request = create_confirmation(
        &pool,
        1,
        2,
        ConfirmationInput {
            action: "edit",
            session_id: second,
            change_id: None,
            expected: &expected,
            target_started_at: Some(150),
            target_ended_at: Some(250),
            target_note: None,
            requested_at: 500,
        },
    )
    .await
    .unwrap();

    assert_eq!(
        confirm_confirmation(&pool, 1, 2, &request.code, 501)
            .await
            .unwrap(),
        ConfirmationResult::Conflict
    );
    assert_eq!(
        confirm_confirmation(&pool, 1, 2, &request.code, 502)
            .await
            .unwrap(),
        ConfirmationResult::NotFound
    );
}

#[tokio::test]
async fn confirmation_is_owner_scoped_and_consumed_at_expiry_boundary() {
    let pool = test_pool().await;
    let id = insert_session(&pool, 1, 2, "Bem", 100, None, 100)
        .await
        .unwrap();
    let expected = SnapshotRow {
        started_at: 100,
        ended_at: None,
        open_since: Some(100),
        note: None,
        deleted_at: None,
    };
    let request = create_confirmation(
        &pool,
        1,
        2,
        ConfirmationInput {
            action: "delete",
            session_id: id,
            change_id: None,
            expected: &expected,
            target_started_at: None,
            target_ended_at: None,
            target_note: None,
            requested_at: 200,
        },
    )
    .await
    .unwrap();

    assert_eq!(
        confirm_confirmation(&pool, 1, 3, &request.code, 201)
            .await
            .unwrap(),
        ConfirmationResult::NotFound
    );
    assert_eq!(
        confirm_confirmation(&pool, 2, 2, &request.code, 201)
            .await
            .unwrap(),
        ConfirmationResult::NotFound
    );
    assert_eq!(
        confirm_confirmation(&pool, 1, 2, &request.code, request.expires_at)
            .await
            .unwrap(),
        ConfirmationResult::Expired
    );
    assert_eq!(
        confirm_confirmation(&pool, 1, 2, &request.code, request.expires_at + 1)
            .await
            .unwrap(),
        ConfirmationResult::NotFound
    );
}
