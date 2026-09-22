//! Writer-owned connection keeping live WAL sidecars available to readonly mounts.
use sqlx::{Connection, SqliteConnection, sqlite::SqliteConnectOptions};

/// Keep this owner alive until the recording process stops.
///
/// Unlike a pooled connection, this connection has no idle/lifetime reaper. It
/// performs one autocommit read to open the WAL, then holds no transaction and
/// no read snapshot. It is never lent to commands or presentation code.
pub struct WalAnchor {
    connection: SqliteConnection,
}

impl WalAnchor {
    pub async fn open(options: &SqliteConnectOptions) -> Result<Self, sqlx::Error> {
        let mut connection = SqliteConnection::connect_with(options).await?;
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sqlite_schema")
            .fetch_one(&mut connection)
            .await?;
        Ok(Self { connection })
    }

    pub async fn close(self) -> Result<(), sqlx::Error> {
        self.connection.close().await
    }
}
