use super::store::*;
use crate::{attendance, repository};
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
};

fn source() -> Location {
    Location {
        guild_id: 1,
        channel_id: 2,
        message_id: 3,
        application_id: 4,
    }
}

fn request(id: i64, user: i64, action: Action, now: i64) -> Request {
    Request {
        location: source(),
        interaction_id: id,
        user_id: user,
        display_name: "架空メンバー".into(),
        action,
        created_at: now,
        received_at: now,
    }
}

async fn memory() -> SqlitePool {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();
    register(&pool, source()).await.unwrap();
    pool
}

async fn count(pool: &SqlitePool) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM attendance_sessions")
        .fetch_one(pool)
        .await
        .unwrap()
}

/// The two adapters share state transitions, timestamps and optional-note semantics.
#[tokio::test]
async fn buttons_and_commands_produce_identical_results() {
    let button = memory().await;
    let command = memory().await;
    for (id, action, now) in [
        (1, Action::Join, 100),
        (2, Action::Join, 101),
        (3, Action::Exit, 200),
        (4, Action::Exit, 201),
    ] {
        let clicked = record(&button, &request(id, 10, action, now))
            .await
            .unwrap();
        let expected = match action {
            Action::Join => Outcome::Start(
                attendance::start(&command, 1, 10, "架空メンバー", now, None, now)
                    .await
                    .unwrap(),
            ),
            Action::Exit => Outcome::End(
                attendance::end(&command, 1, 10, now, None, now)
                    .await
                    .unwrap(),
            ),
        };
        assert_eq!(
            serde_json::to_value(clicked.outcome).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
    }
    assert_eq!(count(&button).await, 1);
}

/// An old join cannot reopen a session after a later exit, even after reconnection.
#[tokio::test]
async fn replay_survives_restart_and_preserves_the_original_result_time() {
    let dir = tempfile::tempdir().unwrap();
    let options = SqliteConnectOptions::new()
        .filename(dir.path().join("dummy.db"))
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal);
    let pool = SqlitePoolOptions::new()
        .connect_with(options.clone())
        .await
        .unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();
    register(&pool, source()).await.unwrap();
    let join = request(1, 10, Action::Join, 100);
    record(&pool, &join).await.unwrap();
    let active = request(2, 10, Action::Join, 150);
    let original = record(&pool, &active).await.unwrap();
    record(&pool, &request(3, 10, Action::Exit, 200))
        .await
        .unwrap();
    pool.close().await;
    let pool = SqlitePoolOptions::new()
        .connect_with(options)
        .await
        .unwrap();
    let replay = record(
        &pool,
        &Request {
            received_at: 250,
            ..join
        },
    )
    .await
    .unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.received_at, 100);
    let replay = record(
        &pool,
        &Request {
            received_at: 250,
            ..active
        },
    )
    .await
    .unwrap();
    assert_eq!(replay.received_at, original.received_at);
    assert_eq!(
        serde_json::to_value(replay.outcome).unwrap(),
        serde_json::to_value(original.outcome).unwrap()
    );
    assert!(
        repository::open_session(&pool, 1, 10)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(count(&pool).await, 1);
    pool.close().await;
}

/// Clicks and Slash Commands compete through the same SQLite write transaction.
#[tokio::test]
async fn concurrent_clicks_commands_and_users_preserve_single_open_session() {
    let dir = tempfile::tempdir().unwrap();
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(
            SqliteConnectOptions::new()
                .filename(dir.path().join("dummy.db"))
                .create_if_missing(true)
                .journal_mode(SqliteJournalMode::Wal)
                .busy_timeout(std::time::Duration::from_secs(5)),
        )
        .await
        .unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();
    register(&pool, source()).await.unwrap();
    let a = request(1, 10, Action::Join, 100);
    let b = request(2, 10, Action::Join, 100);
    let c = request(3, 20, Action::Join, 100);
    let (a, b, c, d, e) = tokio::join!(
        record(&pool, &a),
        record(&pool, &a),
        record(&pool, &b),
        record(&pool, &c),
        attendance::start(&pool, 1, 10, "架空メンバー", 100, None, 100)
    );
    assert!(a.is_ok() && b.is_ok() && c.is_ok() && d.is_ok() && e.is_ok());
    assert_eq!(count(&pool).await, 2);
    let a = request(4, 10, Action::Exit, 200);
    let b = request(5, 10, Action::Exit, 200);
    let (a, b, c) = tokio::join!(
        record(&pool, &a),
        record(&pool, &b),
        attendance::end(&pool, 1, 10, 200, None, 200)
    );
    assert!(a.is_ok() && b.is_ok() && c.is_ok());
    assert!(
        repository::open_session(&pool, 1, 10)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        repository::open_session(&pool, 1, 20)
            .await
            .unwrap()
            .is_some()
    );
    pool.close().await;
}

/// Every persisted source coordinate is required; replacing a panel revokes its buttons.
#[tokio::test]
async fn rejects_unknown_sources_and_mismatched_receipts() {
    let pool = memory().await;
    let locations = [
        Location {
            guild_id: 9,
            ..source()
        },
        Location {
            channel_id: 9,
            ..source()
        },
        Location {
            message_id: 9,
            ..source()
        },
        Location {
            application_id: 9,
            ..source()
        },
    ];
    for location in locations {
        let req = Request {
            location,
            ..request(1, 10, Action::Join, 100)
        };
        assert!(matches!(
            record(&pool, &req).await,
            Err(RecordingError::InvalidPanel)
        ));
    }
    let original = request(1, 10, Action::Join, 100);
    record(&pool, &original).await.unwrap();
    assert!(matches!(
        record(&pool, &request(1, 20, Action::Join, 100)).await,
        Err(RecordingError::ReceiptMismatch)
    ));
    assert!(matches!(
        record(&pool, &request(1, 10, Action::Exit, 100)).await,
        Err(RecordingError::ReceiptMismatch)
    ));
    register(
        &pool,
        Location {
            message_id: 5,
            ..source()
        },
    )
    .await
    .unwrap();
    assert!(matches!(
        record(&pool, &original).await,
        Err(RecordingError::InvalidPanel)
    ));
    assert_eq!(count(&pool).await, 1);
}

/// Failed receipt persistence rolls back the activity and its audit change together.
#[tokio::test]
async fn receipt_failure_rolls_back_recording_and_recovery_never_reapplies() {
    let pool = memory().await;
    sqlx::query("CREATE TRIGGER dummy_failure BEFORE INSERT ON attendance_panel_receipts BEGIN SELECT RAISE(ABORT,'dummy failure'); END")
        .execute(&pool).await.unwrap();
    let req = request(1, 10, Action::Join, 100);
    assert!(record(&pool, &req).await.is_err());
    assert_eq!(count(&pool).await, 0);
    let changes: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM attendance_changes")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(changes, 0);
    assert!(recover(&pool, &req).await.unwrap().is_none());
    sqlx::query("DROP TRIGGER dummy_failure")
        .execute(&pool)
        .await
        .unwrap();
    record(&pool, &req).await.unwrap();
    // Simulate losing the Discord response after commit: only consult the receipt.
    assert!(recover(&pool, &req).await.unwrap().unwrap().replayed);
    assert_eq!(count(&pool).await, 1);
}

/// A business rejection is fixed, even if later operations would make it valid.
#[tokio::test]
async fn rejected_operation_is_not_applied_after_state_changes() {
    let pool = memory().await;
    attendance::start(&pool, 1, 10, "dummy", 200, None, 200)
        .await
        .unwrap();
    let req = request(1, 10, Action::Exit, 100);
    assert!(matches!(
        record(&pool, &req).await.unwrap().outcome,
        Outcome::Rejected(Rejection::EndBeforeStart)
    ));
    attendance::end(&pool, 1, 10, 250, None, 250).await.unwrap();
    assert!(matches!(
        record(&pool, &req).await.unwrap().outcome,
        Outcome::Rejected(Rejection::EndBeforeStart)
    ));
}

/// User erasure removes receipt ownership/payload while retaining replay protection.
#[tokio::test]
async fn erasure_leaves_only_an_anonymous_receipt_tombstone() {
    let pool = memory().await;
    let req = request(1, 10, Action::Join, 100);
    record(&pool, &req).await.unwrap();
    record(&pool, &request(2, 20, Action::Join, 100))
        .await
        .unwrap();
    repository::purge_user_data(&pool, 1, 10).await.unwrap();
    assert!(matches!(
        record(&pool, &req).await.unwrap().outcome,
        Outcome::Erased
    ));
    let cleared: bool = sqlx::query_scalar("SELECT guild_id IS NULL AND user_id IS NULL AND channel_id IS NULL AND message_id IS NULL AND application_id IS NULL AND action IS NULL AND outcome_json IS NULL FROM attendance_panel_receipts WHERE interaction_id=1")
        .fetch_one(&pool).await.unwrap();
    assert!(cleared);
    assert_eq!(count(&pool).await, 1);
    assert!(
        repository::open_session(&pool, 1, 20)
            .await
            .unwrap()
            .is_some()
    );
}

/// Expiry is checked before cleanup; a forgotten receipt cannot make an old click new.
#[tokio::test]
async fn expired_interaction_remains_rejected_after_receipt_cleanup() {
    let pool = memory().await;
    let req = request(1, 10, Action::Join, 100);
    record(&pool, &req).await.unwrap();
    record(&pool, &request(2, 10, Action::Exit, 200))
        .await
        .unwrap();
    let now = 100 + RECEIPT_RETENTION + 1;
    record(&pool, &request(3, 20, Action::Join, now))
        .await
        .unwrap();
    assert!(recover(&pool, &req).await.unwrap().is_none());
    assert!(matches!(
        record(
            &pool,
            &Request {
                received_at: now,
                ..req
            }
        )
        .await,
        Err(RecordingError::Expired)
    ));
    assert_eq!(count(&pool).await, 2);
    assert_eq!(Action::parse("attendance:v2:join"), None);
    assert_eq!(Action::parse("attendance:v1:join:20"), None);
}

/// Buttons use the very same automatic-end correction rule as Slash Commands.
#[tokio::test]
async fn exit_corrects_automatic_end_and_repeated_exit_does_not_move_it() {
    use chrono::TimeZone;
    let pool = memory().await;
    let start = chrono::Utc
        .with_ymd_and_hms(2026, 8, 1, 1, 0, 0)
        .unwrap()
        .timestamp();
    record(&pool, &request(1, 10, Action::Join, start))
        .await
        .unwrap();
    let now = start + 15 * 3600;
    repository::apply_due_auto_ends(&pool, 1, now)
        .await
        .unwrap();
    let outcome = record(&pool, &request(2, 10, Action::Exit, now))
        .await
        .unwrap()
        .outcome;
    assert!(matches!(
        outcome,
        Outcome::End(attendance::EndOutcome::AutoEndedCorrected { .. })
    ));
    let next = record(&pool, &request(3, 10, Action::Exit, now + 60))
        .await
        .unwrap()
        .outcome;
    match next {
        Outcome::End(attendance::EndOutcome::AlreadyInactive(Some(session))) => {
            assert_eq!(session.ended_at, Some(now))
        }
        other => panic!("unexpected outcome: {other:?}"),
    }
}

#[derive(Default)]
struct FakeTransport {
    updates: std::collections::VecDeque<Result<(), super::manage::TransportError>>,
    calls: Vec<String>,
}
impl super::manage::Transport for FakeTransport {
    async fn update(
        &mut self,
        location: Location,
        enabled: bool,
    ) -> Result<(), super::manage::TransportError> {
        self.calls
            .push(format!("update:{}:{enabled}", location.message_id));
        self.updates.pop_front().unwrap_or(Ok(()))
    }
    async fn create_disabled(
        &mut self,
        channel: i64,
    ) -> Result<i64, super::manage::TransportError> {
        self.calls.push(format!("create_disabled:{channel}"));
        Ok(1000)
    }
    async fn delete(&mut self, location: Location) -> Result<(), super::manage::TransportError> {
        self.calls.push(format!("delete:{}", location.message_id));
        Ok(())
    }
}

#[tokio::test]
async fn management_updates_existing_and_recreates_only_missing_message() {
    use super::manage::{TransportError, install};
    let pool = memory().await;
    let mut http = FakeTransport::default();
    install(&pool, 1, 2, 4, &mut http).await.unwrap();
    assert_eq!(http.calls, ["update:3:true"]);
    http.calls.clear();
    http.updates.push_back(Err(TransportError::Failed));
    assert!(install(&pool, 1, 2, 4, &mut http).await.is_err());
    assert_eq!(http.calls, ["update:3:true"]);
    assert_eq!(location(&pool, 1).await.unwrap(), Some(source()));
    http.calls.clear();
    http.updates.push_back(Err(TransportError::MissingMessage));
    install(&pool, 1, 2, 4, &mut http).await.unwrap();
    assert_eq!(
        http.calls,
        [
            "update:3:true",
            "create_disabled:2",
            "update:1000:true",
            "update:3:false"
        ]
    );
    assert_eq!(location(&pool, 1).await.unwrap().unwrap().message_id, 1000);
}

#[tokio::test]
async fn failed_registration_cleans_disabled_orphan_and_preserves_old_panel() {
    let pool = memory().await;
    sqlx::query("CREATE TRIGGER dummy_failure BEFORE INSERT ON attendance_panels BEGIN SELECT RAISE(ABORT,'dummy'); END").execute(&pool).await.unwrap();
    let mut http = FakeTransport::default();
    assert!(
        super::manage::install(&pool, 1, 9, 4, &mut http)
            .await
            .is_err()
    );
    assert_eq!(http.calls, ["create_disabled:9", "delete:1000"]);
    assert_eq!(location(&pool, 1).await.unwrap(), Some(source()));
    record(&pool, &request(1, 10, Action::Join, 100))
        .await
        .unwrap();
}

#[tokio::test]
async fn replacement_revokes_old_even_when_http_updates_fail_and_retry_repairs_new() {
    use super::manage::{TransportError, install};
    let pool = memory().await;
    let mut http = FakeTransport::default();
    http.updates
        .extend([Err(TransportError::Failed), Err(TransportError::Failed)]);
    let result = install(&pool, 1, 9, 4, &mut http).await.unwrap();
    assert!(!result.enabled && !result.old_disabled);
    assert!(matches!(
        record(&pool, &request(1, 10, Action::Join, 100)).await,
        Err(RecordingError::InvalidPanel)
    ));
    http.calls.clear();
    assert!(install(&pool, 1, 9, 4, &mut http).await.unwrap().enabled);
    assert_eq!(http.calls, ["update:1000:true"]);
    // Registration is usable regardless of old-message cleanup.
    record(
        &pool,
        &Request {
            location: result.location,
            ..request(2, 10, Action::Join, 100)
        },
    )
    .await
    .unwrap();
}

fn interaction_json() -> serde_json::Value {
    serde_json::json!({
        "id":"1000000000000000000", "application_id":"4", "type":3,
        "data":{"custom_id":"attendance:v1:join","component_type":2,"values":[]},
        "guild_id":"1", "channel_id":"2",
        "member":{"user":{"id":"10","username":"dummy","discriminator":"0","avatar":null},
            "flags":0,"roles":[],"joined_at":"2026-01-01T00:00:00Z","deaf":false,"mute":false,"permissions":"0"},
        "token":"dummy-interaction-token", "version":1, "locale":"ja",
        "message":{"id":"3","channel_id":"2","guild_id":"1", "author":{"id":"4","username":"dummy-bot","discriminator":"0","avatar":null,"bot":true},
            "content":"", "timestamp":"2026-01-01T00:00:00Z","edited_timestamp":null,
            "tts":false,"mention_everyone":false,"mentions":[],"mention_roles":[],"attachments":[],"embeds":[],"pinned":false,"flags":0,"type":0},
        "entitlements":[],"attachment_size_limit":20971520
    })
}

#[tokio::test]
async fn discord_envelope_rejects_wrong_sources_and_component_types() {
    use super::discord::{InputError, request as parse};
    let data = crate::Data {
        database: memory().await,
        guild_id: 1,
        application_id: 4,
        panel_management: tokio::sync::Mutex::new(()),
    };
    let decode = |value| {
        serde_json::from_value::<poise::serenity_prelude::ComponentInteraction>(value).unwrap()
    };
    let valid = decode(interaction_json());
    let parsed = parse(&valid, &data, 123).unwrap();
    assert_eq!(parsed.user_id, 10);
    assert_eq!(parsed.received_at, 123);
    assert_eq!(parsed.location, source());
    for (pointer, replacement, expected) in [
        ("/guild_id", serde_json::json!("9"), InputError::Guild),
        (
            "/application_id",
            serde_json::json!("9"),
            InputError::Source,
        ),
        (
            "/message/author/id",
            serde_json::json!("9"),
            InputError::Source,
        ),
        (
            "/message/channel_id",
            serde_json::json!("9"),
            InputError::Source,
        ),
        (
            "/message/guild_id",
            serde_json::json!("9"),
            InputError::Source,
        ),
        (
            "/data/custom_id",
            serde_json::json!("attendance:v2:join"),
            InputError::Unsupported,
        ),
        (
            "/data/custom_id",
            serde_json::json!("attendance:v1:join:20"),
            InputError::Unsupported,
        ),
        (
            "/data/component_type",
            serde_json::json!(3),
            InputError::Unsupported,
        ),
    ] {
        let mut payload = interaction_json();
        *payload.pointer_mut(pointer).unwrap() = replacement;
        assert_eq!(parse(&decode(payload), &data, 123).unwrap_err(), expected);
    }
    let mut mismatch = valid.clone();
    mismatch.member.as_mut().unwrap().user.id = poise::serenity_prelude::UserId::new(99);
    assert_eq!(
        parse(&mismatch, &data, 123).unwrap_err(),
        InputError::Source
    );
    mismatch = valid.clone();
    mismatch.member = None;
    assert_eq!(
        parse(&mismatch, &data, 123).unwrap_err(),
        InputError::Source
    );
    mismatch = valid;
    mismatch.user.bot = true;
    assert_eq!(
        parse(&mismatch, &data, 123).unwrap_err(),
        InputError::Source
    );
}

#[test]
fn panel_permissions_and_two_explicit_buttons_are_fixed() {
    use poise::serenity_prelude::Permissions;
    let command = super::panel();
    assert!(command.guild_only);
    assert!(
        command
            .required_permissions
            .contains(Permissions::MANAGE_GUILD)
    );
    for enabled in [false, true] {
        let value = serde_json::to_value(super::discord::components(enabled)).unwrap();
        let buttons = value[0]["components"].as_array().unwrap();
        assert_eq!(buttons.len(), 2);
        assert_eq!(buttons[0]["custom_id"], "attendance:v1:join");
        assert_eq!(buttons[1]["custom_id"], "attendance:v1:exit");
        for button in buttons {
            assert_eq!(button["disabled"], !enabled);
        }
    }
}
