use sqlx::SqlitePool;

use super::transaction::begin_immediate;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PurgeUserResult {
    pub sessions: u64,
    pub profile: u64,
}

/// Permanently removes all data owned by one user in one guild.
///
/// Foreign-key cascades remove change history, pending confirmations, and
/// automatic-end events belonging to the deleted sessions.
pub async fn purge_user_data(
    pool: &SqlitePool,
    guild_id: i64,
    user_id: i64,
) -> Result<PurgeUserResult, sqlx::Error> {
    let mut tx = begin_immediate(pool).await?;
    let sessions = sqlx::query(
        "DELETE FROM attendance_sessions
         WHERE guild_id = ? AND user_id = ?",
    )
    .bind(guild_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    let profile = sqlx::query(
        "DELETE FROM attendance_user_profiles
         WHERE guild_id = ? AND user_id = ?",
    )
    .bind(guild_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    tx.commit().await?;
    Ok(PurgeUserResult { sessions, profile })
}

#[cfg(test)]
mod tests {
    use sqlx::sqlite::SqlitePoolOptions;

    use super::*;

    #[tokio::test]
    async fn purge_removes_sessions_cascades_and_profile_for_only_one_owner() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();

        for user_id in [10_i64, 20] {
            let session_id = sqlx::query(
                "INSERT INTO attendance_sessions (
                    guild_id, user_id, display_name, started_at, ended_at, open_since,
                    created_at, updated_at
                 ) VALUES (1, ?, 'user', 100, 200, NULL, 100, 100)",
            )
            .bind(user_id)
            .execute(&pool)
            .await
            .unwrap()
            .last_insert_rowid();
            sqlx::query(
                "INSERT INTO attendance_changes (
                    guild_id, user_id, session_id, kind,
                    before_started_at, before_ended_at, before_open_since,
                    after_started_at, after_ended_at, after_open_since,
                    created_at
                 ) VALUES (1, ?, ?, 'end', 100, NULL, 100, 100, 200, NULL, 200)",
            )
            .bind(user_id)
            .bind(session_id)
            .execute(&pool)
            .await
            .unwrap();
            sqlx::query(
                "INSERT INTO attendance_user_profiles (
                    guild_id, user_id, real_name, created_at, updated_at
                 ) VALUES (1, ?, 'real name', 100, 100)",
            )
            .bind(user_id)
            .execute(&pool)
            .await
            .unwrap();
        }

        let result = purge_user_data(&pool, 1, 10).await.unwrap();
        assert_eq!(
            result,
            PurgeUserResult {
                sessions: 1,
                profile: 1
            }
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM attendance_changes WHERE user_id = 10"
            )
            .fetch_one(&pool)
            .await
            .unwrap(),
            0
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM attendance_sessions WHERE user_id = 20"
            )
            .fetch_one(&pool)
            .await
            .unwrap(),
            1
        );
    }
}
