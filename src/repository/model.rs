use sqlx::FromRow;
use std::fmt;
use thiserror::Error;

use crate::attendance::AttendanceSession;

#[derive(Debug, Clone)]
pub struct ConfirmationInput<'a> {
    pub action: ConfirmationAction,
    pub session_id: i64,
    pub change_id: Option<i64>,
    pub expected: &'a SessionSnapshot,
    pub target_started_at: Option<i64>,
    pub target_ended_at: Option<i64>,
    pub target_note: Option<&'a str>,
    pub requested_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmationRequest {
    pub code: String,
    pub action: ConfirmationAction,
    pub session_id: i64,
    pub change_id: Option<i64>,
    pub target_started_at: Option<i64>,
    pub target_ended_at: Option<i64>,
    pub target_note: Option<String>,
    pub expires_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevertPreview {
    pub change_id: i64,
    pub operation: String,
    pub session_id: i64,
    pub current: SessionSnapshot,
    pub before_started_at: Option<i64>,
    pub before_ended_at: Option<i64>,
    pub before_open_since: Option<i64>,
    pub before_note: Option<String>,
    pub before_deleted_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfirmationResult {
    Confirmed {
        action: ConfirmationAction,
        session_id: i64,
        operation: Option<String>,
    },
    NotFound,
    Expired,
    Conflict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// The confirmation-protected operation requested by a user.
pub enum ConfirmationAction {
    Edit,
    Delete,
    Revert,
}

impl ConfirmationAction {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Edit => "edit",
            Self::Delete => "delete",
            Self::Revert => "revert",
        }
    }
}

impl fmt::Display for ConfirmationAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<String> for ConfirmationAction {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        match value.as_str() {
            "edit" => Ok(Self::Edit),
            "delete" => Ok(Self::Delete),
            "revert" => Ok(Self::Revert),
            _ => Err(value),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// A persisted activity change operation.
pub enum ChangeOperation {
    Start,
    End,
    Continue,
    Edit,
    Delete,
}

impl ChangeOperation {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::End => "end",
            Self::Continue => "continue",
            Self::Edit => "edit",
            Self::Delete => "delete",
        }
    }
}

impl fmt::Display for ChangeOperation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<String> for ChangeOperation {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        match value.as_str() {
            "start" => Ok(Self::Start),
            "end" => Ok(Self::End),
            "continue" => Ok(Self::Continue),
            "edit" => Ok(Self::Edit),
            "delete" => Ok(Self::Delete),
            _ => Err(value),
        }
    }
}

#[derive(Debug, Error)]
/// Typed failures raised while preserving activity-state invariants.
pub enum SessionMutationError {
    #[error("活動記録の時間帯が別の記録と重複している")]
    Overlapping,
    #[error("終了時刻は開始時刻以降である必要がある")]
    EndBeforeStart,
    #[error("活動記録の状態がDBの不変条件を満たさない: {0}")]
    Invariant(&'static str),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RevertPreviewResult {
    Available(Box<RevertPreview>),
    NothingToRevert,
    Conflict,
}

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct SnapshotRow {
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub open_since: Option<i64>,
    pub note: Option<String>,
    pub deleted_at: Option<i64>,
}

pub type SessionSnapshot = SnapshotRow;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoEndNotice {
    pub event_id: i64,
    pub session_id: i64,
    pub automatic_ended_at: i64,
    pub applied_at: i64,
    pub corrected_at: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct AutoEndCorrection {
    pub session: AttendanceSession,
    pub automatic_ended_at: i64,
}

#[derive(Debug, Clone)]
pub enum EndSessionResult {
    Ended(AttendanceSession),
    AutoEndedCorrected {
        session: AttendanceSession,
        automatic_ended_at: i64,
    },
    AlreadyInactive(Option<AttendanceSession>),
    EndBeforeStart,
}

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct UserProfile {
    pub guild_id: i64,
    pub user_id: i64,
    pub generation: Option<i64>,
    pub real_name: Option<String>,
    pub role: Option<String>,
    pub name_reading: Option<String>,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Copy)]
pub struct UserProfileUpdate<'a> {
    pub generation: Option<i64>,
    pub real_name: Option<&'a str>,
    pub role: Option<&'a str>,
    pub name_reading: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct ActiveAttendanceMember {
    pub user_id: i64,
    pub display_name: String,
    pub generation: Option<i64>,
    pub real_name: Option<String>,
    pub role: Option<String>,
    pub name_reading: Option<String>,
}
