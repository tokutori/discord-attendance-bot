use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};
use thiserror::Error;

use crate::{attendance, repository};

pub const INTERACTION_MAX_AGE: i64 = 15 * 60;
pub const RECEIPT_RETENTION: i64 = 30 * 24 * 60 * 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Action {
    Join,
    Exit,
}

impl Action {
    pub fn parse(custom_id: &str) -> Option<Self> {
        match custom_id {
            "attendance:v1:join" => Some(Self::Join),
            "attendance:v1:exit" => Some(Self::Exit),
            _ => None,
        }
    }

    pub fn custom_id(self) -> &'static str {
        match self {
            Self::Join => "attendance:v1:join",
            Self::Exit => "attendance:v1:exit",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, FromRow)]
pub struct Location {
    pub guild_id: i64,
    pub channel_id: i64,
    pub message_id: i64,
    pub application_id: i64,
}

#[derive(Debug)]
pub struct Request {
    pub location: Location,
    pub interaction_id: i64,
    pub user_id: i64,
    pub display_name: String,
    pub action: Action,
    pub created_at: i64,
    pub received_at: i64,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Rejection {
    EndBeforeStart,
    Overlap,
    FutureTime,
    OpenSessionConflict,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum Outcome {
    Start(attendance::StartOutcome),
    End(attendance::EndOutcome),
    Rejected(Rejection),
    Erased,
}

#[derive(Debug)]
pub struct Receipt {
    pub outcome: Outcome,
    pub received_at: i64,
    pub replayed: bool,
}

#[derive(Debug, Error)]
pub enum RecordingError {
    #[error("このパネルは有効ではない。管理者に再設置を依頼してほしい")]
    InvalidPanel,
    #[error("この操作は受付期限を過ぎている。履歴を確認してから操作してほしい")]
    Expired,
    #[error("処理済み操作の情報が一致しない")]
    ReceiptMismatch,
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    Service(#[from] attendance::ServiceError),
    #[error("保存済みの処理結果を確認できなかった")]
    InvalidReceipt(#[from] serde_json::Error),
}

pub async fn location(pool: &SqlitePool, guild_id: i64) -> Result<Option<Location>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM attendance_panels WHERE guild_id=?")
        .bind(guild_id)
        .fetch_optional(pool)
        .await
}

/// Called only after Discord has created a disabled message. No HTTP in this tx.
pub async fn register(pool: &SqlitePool, location: Location) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO attendance_panels (guild_id,channel_id,message_id,application_id) VALUES (?,?,?,?) ON CONFLICT(guild_id) DO UPDATE SET channel_id=excluded.channel_id,message_id=excluded.message_id,application_id=excluded.application_id")
        .bind(location.guild_id).bind(location.channel_id).bind(location.message_id)
        .bind(location.application_id).execute(pool).await?;
    Ok(())
}

#[derive(FromRow)]
struct StoredReceipt {
    guild_id: Option<i64>,
    user_id: Option<i64>,
    channel_id: Option<i64>,
    message_id: Option<i64>,
    application_id: Option<i64>,
    action: Option<String>,
    received_at: i64,
    outcome_json: Option<String>,
}

impl StoredReceipt {
    fn decode(self, request: &Request) -> Result<Receipt, RecordingError> {
        if self.guild_id.is_none() && self.user_id.is_none() && self.outcome_json.is_none() {
            return Ok(Receipt {
                outcome: Outcome::Erased,
                received_at: self.received_at,
                replayed: true,
            });
        }
        let source = request.location;
        if self.guild_id != Some(source.guild_id)
            || self.user_id != Some(request.user_id)
            || self.channel_id != Some(source.channel_id)
            || self.message_id != Some(source.message_id)
            || self.application_id != Some(source.application_id)
            || self.action.as_deref() != Some(request.action.custom_id())
        {
            return Err(RecordingError::ReceiptMismatch);
        }
        let outcome = serde_json::from_str(self.outcome_json.as_deref().unwrap_or(""))?;
        Ok(Receipt {
            outcome,
            received_at: self.received_at,
            replayed: true,
        })
    }
}

/// Recover a persisted result after an uncertain DB error; never repeat a mutation.
pub async fn recover(
    pool: &SqlitePool,
    request: &Request,
) -> Result<Option<Receipt>, RecordingError> {
    sqlx::query_as::<_, StoredReceipt>(
        "SELECT * FROM attendance_panel_receipts WHERE interaction_id=?",
    )
    .bind(request.interaction_id)
    .fetch_optional(pool)
    .await?
    .map(|stored| stored.decode(request))
    .transpose()
}

pub async fn record(pool: &SqlitePool, request: &Request) -> Result<Receipt, RecordingError> {
    let age = request.received_at.saturating_sub(request.created_at);
    if !(-60..=INTERACTION_MAX_AGE).contains(&age) {
        return Err(RecordingError::Expired);
    }
    let source = request.location;
    let mut tx = repository::begin_immediate(pool).await?;
    let active: Option<Location> =
        sqlx::query_as("SELECT * FROM attendance_panels WHERE guild_id=?")
            .bind(source.guild_id)
            .fetch_optional(&mut *tx)
            .await?;
    if active != Some(source) {
        return Err(RecordingError::InvalidPanel);
    }
    sqlx::query("DELETE FROM attendance_panel_receipts WHERE received_at < ?")
        .bind(request.received_at.saturating_sub(RECEIPT_RETENTION))
        .execute(&mut *tx)
        .await?;
    if let Some(stored) = sqlx::query_as::<_, StoredReceipt>(
        "SELECT * FROM attendance_panel_receipts WHERE interaction_id=?",
    )
    .bind(request.interaction_id)
    .fetch_optional(&mut *tx)
    .await?
    {
        let result = stored.decode(request)?;
        tx.commit().await?;
        return Ok(result);
    }
    // Business rejections are receipts too. A savepoint ensures a rejected
    // attempt cannot retain partial writes if a service adds validation later.
    sqlx::query("SAVEPOINT panel_recording")
        .execute(&mut *tx)
        .await?;
    let result = match request.action {
        Action::Join => attendance::start_in_tx(
            &mut tx,
            source.guild_id,
            request.user_id,
            &request.display_name,
            request.received_at,
            None,
            request.received_at,
        )
        .await
        .map(Outcome::Start),
        Action::Exit => attendance::end_in_tx(
            &mut tx,
            source.guild_id,
            request.user_id,
            request.received_at,
            None,
            request.received_at,
        )
        .await
        .map(Outcome::End),
    };
    let outcome = match result {
        Ok(outcome) => outcome,
        Err(error) => {
            let reason = match error {
                attendance::ServiceError::EndBeforeStart => Rejection::EndBeforeStart,
                attendance::ServiceError::OverlappingSession => Rejection::Overlap,
                attendance::ServiceError::FutureTime => Rejection::FutureTime,
                attendance::ServiceError::OpenSessionConflict => Rejection::OpenSessionConflict,
                error => return Err(error.into()),
            };
            sqlx::query("ROLLBACK TO panel_recording")
                .execute(&mut *tx)
                .await?;
            Outcome::Rejected(reason)
        }
    };
    sqlx::query("RELEASE panel_recording")
        .execute(&mut *tx)
        .await?;
    let serialized = serde_json::to_string(&outcome)?;
    sqlx::query("INSERT INTO attendance_panel_receipts (interaction_id,guild_id,user_id,channel_id,message_id,application_id,action,received_at,outcome_json) VALUES (?,?,?,?,?,?,?,?,?)")
        .bind(request.interaction_id).bind(source.guild_id).bind(request.user_id)
        .bind(source.channel_id).bind(source.message_id).bind(source.application_id)
        .bind(request.action.custom_id()).bind(request.received_at).bind(serialized)
        .execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(Receipt {
        outcome,
        received_at: request.received_at,
        replayed: false,
    })
}
