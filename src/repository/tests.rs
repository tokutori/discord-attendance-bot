use chrono::TimeZone;
use sqlx::{SqlitePool, sqlite::SqlitePoolOptions};

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
    close_session(&pool, id, 200, Some("end"), 200)
        .await
        .unwrap();
    reopen_session(&pool, id, 300).await.unwrap();
    let before_edit = SnapshotRow {
        started_at: 100,
        ended_at: None,
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
async fn confirmation_is_preview_only_until_confirmed_and_single_use() {
    let pool = test_pool().await;
    let id = insert_session(&pool, 1, 2, "Bem", 100, Some("old"), 100)
        .await
        .unwrap();
    let expected = SnapshotRow {
        started_at: 100,
        ended_at: None,
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

    let notice = take_auto_end_notice(&pool, 1, 2, now).await.unwrap();
    assert_eq!(notice.unwrap().session_id, id);
    assert!(
        take_auto_end_notice(&pool, 1, 2, now)
            .await
            .unwrap()
            .is_none()
    );

    let correction =
        correct_auto_ended_session(&pool, id, 1, 2, manual_end, Some("manual"), now + 1)
            .await
            .unwrap()
            .unwrap();
    assert_eq!(correction.session.ended_at, Some(manual_end));
    assert!(latest_auto_ended(&pool, 1, 2).await.unwrap().is_none());
}
