use chrono::{NaiveTime, TimeZone};
use discord_attendance_bot::{
    repository,
    time::{TimePolicy, install_time_policy},
};

#[tokio::test]
async fn repository_applies_missing_hour_cutoff_once() {
    let policy = TimePolicy {
        timezone: "America/New_York".parse().unwrap(),
        auto_end_time: NaiveTime::from_hms_opt(2, 30, 0),
    };
    let timezone = policy.timezone;
    install_time_policy(policy).unwrap();
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();
    let start = timezone
        .with_ymd_and_hms(2026, 3, 7, 23, 0, 0)
        .unwrap()
        .timestamp();
    let now = timezone
        .with_ymd_and_hms(2026, 3, 9, 0, 0, 0)
        .unwrap()
        .timestamp();
    let id = repository::insert_session(&pool, 1, 2, "dummy", start, None, start)
        .await
        .unwrap();
    assert_eq!(
        repository::apply_due_auto_ends(&pool, 1, now)
            .await
            .unwrap()
            .len(),
        1
    );
    let session = repository::get_owned(&pool, id, 1, 2)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        session.ended_at,
        Some(
            timezone
                .with_ymd_and_hms(2026, 3, 8, 3, 0, 0)
                .unwrap()
                .timestamp()
        )
    );
    assert!(
        repository::apply_due_auto_ends(&pool, 1, now)
            .await
            .unwrap()
            .is_empty()
    );
    pool.close().await;
}
