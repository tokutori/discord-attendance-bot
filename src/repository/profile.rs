use sqlx::SqlitePool;

use super::{UserProfile, UserProfileUpdate};

pub async fn get_user_profile(
    pool: &SqlitePool,
    guild_id: i64,
    user_id: i64,
) -> Result<Option<UserProfile>, sqlx::Error> {
    sqlx::query_as(
        "SELECT guild_id, user_id, generation, real_name, role, updated_at
         FROM attendance_user_profiles
         WHERE guild_id = ? AND user_id = ?",
    )
    .bind(guild_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await
}

pub async fn upsert_user_profile(
    pool: &SqlitePool,
    guild_id: i64,
    user_id: i64,
    update: UserProfileUpdate<'_>,
    now: i64,
) -> Result<UserProfile, sqlx::Error> {
    sqlx::query(
        "INSERT INTO attendance_user_profiles (
            guild_id, user_id, generation, real_name, role, created_at, updated_at
         ) VALUES (?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT (guild_id, user_id) DO UPDATE SET
            generation = COALESCE(excluded.generation, attendance_user_profiles.generation),
            real_name = COALESCE(excluded.real_name, attendance_user_profiles.real_name),
            role = COALESCE(excluded.role, attendance_user_profiles.role),
            updated_at = excluded.updated_at",
    )
    .bind(guild_id)
    .bind(user_id)
    .bind(update.generation)
    .bind(update.real_name)
    .bind(update.role)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await?;

    get_user_profile(pool, guild_id, user_id)
        .await?
        .ok_or(sqlx::Error::RowNotFound)
}

pub async fn user_profiles_for_export(
    pool: &SqlitePool,
    guild_id: i64,
) -> Result<Vec<UserProfile>, sqlx::Error> {
    sqlx::query_as(
        "SELECT guild_id, user_id, generation, real_name, role, updated_at
         FROM attendance_user_profiles
         WHERE guild_id = ?
         ORDER BY generation, real_name, user_id",
    )
    .bind(guild_id)
    .fetch_all(pool)
    .await
}

#[cfg(test)]
mod tests {
    use sqlx::sqlite::SqlitePoolOptions;

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

    #[tokio::test]
    async fn profile_updates_only_specified_fields() {
        let pool = test_pool().await;
        let created = upsert_user_profile(
            &pool,
            1,
            2,
            UserProfileUpdate {
                generation: Some(5),
                real_name: Some("山田太郎"),
                role: Some("代表"),
            },
            100,
        )
        .await
        .unwrap();
        assert_eq!(created.generation, Some(5));
        assert_eq!(created.real_name.as_deref(), Some("山田太郎"));

        let updated = upsert_user_profile(
            &pool,
            1,
            2,
            UserProfileUpdate {
                generation: None,
                real_name: None,
                role: Some("設計班"),
            },
            200,
        )
        .await
        .unwrap();
        assert_eq!(updated.generation, Some(5));
        assert_eq!(updated.real_name.as_deref(), Some("山田太郎"));
        assert_eq!(updated.role.as_deref(), Some("設計班"));
    }
}
