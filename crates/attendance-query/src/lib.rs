#![deny(unsafe_code)]

//! Read capability for presentation processes. Connections and executors never escape.
//!
//! ```compile_fail
//! # async fn cannot_write(db: attendance_query::ReadDatabase) {
//! sqlx::query("DELETE FROM attendance_sessions").execute(&db.pool).await.unwrap();
//! # }
//! ```
//!
//! ```compile_fail
//! use attendance_query::acknowledge_auto_end_notice;
//! ```

use std::{path::Path, str::FromStr, time::Duration};

use anyhow::{Context as _, ensure};
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};

pub use attendance_shared::records::*;
mod queries;
pub use queries::*;
/// SELECT-only primitives reused by the writer with its own pool.
pub mod sql;

#[derive(Clone)]
pub struct ReadDatabase {
    pool: SqlitePool,
}

impl ReadDatabase {
    pub async fn open(database_url: &str) -> anyhow::Result<Self> {
        let parsed = SqliteConnectOptions::from_str(database_url)?;
        Self::open_file(parsed.get_filename()).await
    }

    pub async fn open_file(path: &Path) -> anyhow::Result<Self> {
        ensure!(
            !path.as_os_str().is_empty() && path != Path::new(":memory:"),
            "attendance-view requires an existing SQLite file"
        );
        // Rebuild options from the filename: URL flags cannot override readonly,
        // enable immutable mode (which ignores live WAL changes), or select a VFS.
        let options = SqliteConnectOptions::new()
            .filename(path)
            .read_only(true)
            .create_if_missing(false)
            .pragma("query_only", "ON")
            .busy_timeout(Duration::from_secs(5));
        let pool = SqlitePoolOptions::new()
            .max_connections(2)
            .acquire_timeout(Duration::from_secs(5))
            .connect_with(options)
            .await
            .context("failed to open readonly attendance DB; start core and migrations first")?;
        attendance_shared::database::require_persistent_database(&pool).await?;
        attendance_shared::database::validate_runtime_sqlite(&pool).await?;
        validate_schema(&pool).await?;
        Ok(Self { pool })
    }
}

async fn validate_schema(pool: &SqlitePool) -> anyhow::Result<()> {
    let versions: Vec<(i64, bool)> =
        sqlx::query_as("SELECT version, success FROM _sqlx_migrations ORDER BY version")
            .fetch_all(pool)
            .await
            .context("attendance DB has not been migrated by core")?;
    ensure!(
        versions == [(1, true)],
        "unsupported attendance schema; core and view must use the current initial schema"
    );
    // Check the actual read contract as well as the migration ledger.
    sqlx::query("SELECT id, guild_id, user_id, display_name, started_at, ended_at, open_since, note, created_at, updated_at, deleted_at FROM attendance_sessions LIMIT 0").fetch_all(pool).await?;
    sqlx::query("SELECT guild_id, user_id, generation, real_name, role, name_reading, updated_at FROM attendance_user_profiles LIMIT 0").fetch_all(pool).await?;
    sqlx::query("SELECT id, session_id, guild_id, user_id, automatic_ended_at, applied_at, corrected_at, notified_at FROM attendance_auto_end_events LIMIT 0").fetch_all(pool).await?;
    // Prototypes consolidate the initial migration; an older schema with the
    // same ledger version is not accepted. No panel records are fetched here.
    sqlx::query(
        "SELECT guild_id, channel_id, message_id, application_id FROM attendance_panels LIMIT 0",
    )
    .fetch_all(pool)
    .await?;
    sqlx::query("SELECT interaction_id, guild_id, user_id, channel_id, message_id, application_id, action, received_at, outcome_json FROM attendance_panel_receipts LIMIT 0").fetch_all(pool).await?;
    Ok(())
}

#[cfg(test)]
mod tests;
