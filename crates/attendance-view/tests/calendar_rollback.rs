use std::collections::BTreeMap;

use attendance_query::{ReadDatabase, overlapping_completed, overlapping_for_export};
use attendance_view::{
    attendance::{YearMonth, aggregate_monthly},
    attendance_export::{IdentityMode, build_monthly_export, to_csv},
    time::{TimePolicy, install_time_policy, month_bounds},
};
use chrono::{DateTime, Datelike, Utc};
use sqlx::{SqlitePool, sqlite::SqliteConnectOptions};

fn utc(clock: &str) -> i64 {
    DateTime::parse_from_rfc3339(&format!("2009-11-01T{clock}:00Z"))
        .unwrap()
        .timestamp()
}

async fn insert(
    pool: &SqlitePool,
    guild: i64,
    user: i64,
    start: i64,
    end: Option<i64>,
    deleted: bool,
) {
    sqlx::query(
        "INSERT INTO attendance_sessions
         (guild_id, user_id, display_name, started_at, ended_at, open_since,
          created_at, updated_at, deleted_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(guild)
    .bind(user)
    .bind(format!("dummy-{user}"))
    .bind(start)
    .bind(end)
    .bind(end.is_none().then_some(start))
    .bind(start)
    .bind(end.unwrap_or(start))
    .bind(deleted.then_some(start))
    .execute(pool)
    .await
    .unwrap();
}

/// At 03:01 UTC Goose Bay's date returns from November 1 to October 31.
/// Exercise the actual readonly SQL boundary, not merely hand-fed DTOs.
#[tokio::test]
async fn readonly_queries_preserve_rollback_months_and_access_scopes() {
    install_time_policy(TimePolicy {
        timezone: "America/Goose_Bay".parse().unwrap(),
        auto_end_time: None,
    })
    .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("dummy.sqlite3");
    let writer = SqlitePool::connect_with(
        SqliteConnectOptions::new()
            .filename(&path)
            .create_if_missing(true),
    )
    .await
    .unwrap();
    sqlx::migrate!("../../migrations")
        .run(&writer)
        .await
        .unwrap();

    for (user, start, end) in [
        (1, "01:00", "05:00"),
        (2, "03:10", "03:20"),
        (3, "03:00", "03:01"),
        (4, "04:00", "04:10"),
    ] {
        insert(&writer, 1, user, utc(start), Some(utc(end)), false).await;
    }
    insert(&writer, 1, 5, utc("03:10"), None, false).await;
    insert(&writer, 1, 6, utc("05:10"), None, false).await;
    insert(&writer, 1, 7, utc("01:00"), Some(utc("05:00")), true).await;
    insert(&writer, 2, 8, utc("01:00"), Some(utc("05:00")), false).await;
    let reader = ReadDatabase::open_file(&path).await.unwrap();
    let now = DateTime::<Utc>::from_timestamp(utc("05:00"), 0).unwrap();

    for (month, expected) in [(10, [10740, 600, 0, 0]), (11, [3660, 0, 60, 600])] {
        let ym = YearMonth { year: 2009, month };
        let (start, end) = month_bounds(ym).unwrap();
        for (user, seconds) in (1..=4).zip(expected) {
            let sessions = overlapping_completed(&reader, 1, user, start, end)
                .await
                .unwrap();
            assert!(
                sessions
                    .iter()
                    .all(|s| s.guild_id == 1 && s.user_id == user)
            );
            let monthly = aggregate_monthly(&sessions, ym, now).unwrap();
            assert_eq!(monthly.total_seconds, seconds, "month {month}, user {user}");
            assert_eq!(monthly.session_count, usize::from(seconds > 0));
            assert_eq!(
                monthly
                    .daily_totals
                    .iter()
                    .map(|d| d.total_seconds)
                    .sum::<i64>(),
                seconds
            );
            assert!(monthly.daily_totals.iter().all(|d| d.date.month() == month));
        }
        for user in [5, 6, 7, 8] {
            assert!(
                overlapping_completed(&reader, 1, user, start, end)
                    .await
                    .unwrap()
                    .is_empty()
            );
        }

        let sessions = overlapping_for_export(&reader, 1, start, end)
            .await
            .unwrap();
        assert!(
            sessions
                .iter()
                .all(|s| s.guild_id == 1 && s.deleted_at.is_none())
        );
        let export = build_monthly_export(ym, &sessions, &[], now).unwrap();
        let actual = export
            .rows
            .iter()
            .map(|r| (r.user_id, r.total_seconds))
            .collect::<BTreeMap<_, _>>();
        let mut expected_rows = (1..=4)
            .zip(expected)
            .filter(|(_, seconds)| *seconds > 0)
            .collect::<BTreeMap<_, _>>();
        expected_rows.insert(5, if month == 10 { 3000 } else { 3600 });
        assert_eq!(actual, expected_rows);
        for row in &export.rows {
            assert_eq!(row.daily_seconds.iter().sum::<i64>(), row.total_seconds);
            let day = if month == 10 { 30 } else { 0 };
            assert_eq!(row.daily_seconds[day], row.total_seconds);
        }
        let csv = String::from_utf8(to_csv(&export, IdentityMode::WithDiscordName)).unwrap();
        assert_eq!(csv.lines().count(), export.rows.len() + 1);
        assert!(!csv.contains("dummy-6") && !csv.contains("dummy-7") && !csv.contains("dummy-8"));

        // Open sessions are clipped to now even while local date has rolled back.
        let live = build_monthly_export(
            ym,
            &sessions
                .iter()
                .filter(|s| s.user_id == 5)
                .cloned()
                .collect::<Vec<_>>(),
            &[],
            DateTime::<Utc>::from_timestamp(utc("03:20"), 0).unwrap(),
        )
        .unwrap();
        if month == 10 {
            assert_eq!(live.rows[0].total_seconds, 600);
        } else {
            assert!(live.rows.is_empty());
        }
    }
    drop(reader);
    writer.close().await;
}
