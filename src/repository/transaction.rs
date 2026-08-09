use sqlx::{Sqlite, SqlitePool, Transaction};

pub(super) async fn begin_immediate(
    pool: &SqlitePool,
) -> Result<Transaction<'static, Sqlite>, sqlx::Error> {
    pool.begin_with("BEGIN IMMEDIATE").await
}
