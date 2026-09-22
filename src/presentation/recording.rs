use crate::{
    Error,
    attendance::{EndOutcome, StartOutcome},
    time::{format_datetime, format_duration},
};

/// Render the frozen recording result for either input adapter.
pub fn start_outcome(outcome: &StartOutcome, now: i64) -> String {
    match outcome {
        StartOutcome::Started(session) => format!(
            "活動を開始した。\n開始時刻: {}\n記録ID: #{}",
            format_datetime(session.started_at),
            session.id
        ),
        StartOutcome::AlreadyActive(session) => {
            let mut text = format!(
                "すでに活動中である。\n\n開始時刻: {}\n経過時間: {}",
                format_datetime(session.started_at),
                format_duration(session.duration_seconds_at(now))
            );
            if let Some(note) = session
                .note
                .as_deref()
                .filter(|value| !value.trim().is_empty())
            {
                text.push_str(&format!("\n備考: {note}"));
            }
            text.push_str(&format!("\n記録ID: #{}", session.id));
            text
        }
    }
}

pub fn end_outcome(outcome: &EndOutcome, now: i64) -> Result<String, Error> {
    Ok(match outcome {
        EndOutcome::Ended(session) => format!(
            "活動を終了した。\n開始時刻: {}\n終了時刻: {}\n活動時間: {}\n記録ID: #{}",
            format_datetime(session.started_at),
            format_datetime(session.completed_end()?),
            format_duration(session.duration_seconds_at(now)),
            session.id
        ),
        EndOutcome::AutoEndedCorrected {
            session,
            automatic_end,
        } => format!(
            "活動を終了した。\n自動終了（{}）を取り消し、入力された終了時刻を正として扱った。\n開始時刻: {}\n終了時刻: {}\n活動時間: {}\n記録ID: #{}",
            format_datetime(*automatic_end),
            format_datetime(session.started_at),
            format_datetime(session.completed_end()?),
            format_duration(session.duration_seconds_at(now)),
            session.id
        ),
        EndOutcome::AlreadyInactive(Some(session)) => format!(
            "現在、活動中の記録はない。\n\n直近の活動:\n{} ～ {}\n活動時間: {}\n記録ID: #{}",
            format_datetime(session.started_at),
            format_datetime(session.completed_end()?),
            format_duration(session.duration_seconds_at(now)),
            session.id
        ),
        EndOutcome::AlreadyInactive(None) => {
            "現在、活動中の記録はない。\n過去の活動記録も存在しない。".into()
        }
    })
}
