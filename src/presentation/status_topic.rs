use crate::{attendance::AttendanceSession, time::format_datetime};

const MAX_TOPIC_CHARS: usize = 1024;
const MAX_ACTIVITY_CHARS: usize = 128;

pub fn status_topic(sessions: &[AttendanceSession], updated_at: i64) -> String {
    status_text(sessions, updated_at, MAX_TOPIC_CHARS)
}

pub fn activity_status(sessions: &[AttendanceSession], updated_at: i64) -> String {
    status_text(sessions, updated_at, MAX_ACTIVITY_CHARS)
        .replace(":green_circle:", "🟢")
        .replace(":white_circle:", "⚪")
}

fn status_text(sessions: &[AttendanceSession], updated_at: i64, max_chars: usize) -> String {
    let updated = format!("(最終更新: {})", format_datetime(updated_at));
    if sessions.is_empty() {
        return format!(":white_circle: 現在0名活動中\n{updated}");
    }

    let mut topic = format!(":green_circle: 現在{}名活動中", sessions.len());
    for (index, session) in sessions.iter().enumerate() {
        let name = session
            .display_name
            .replace(['\r', '\n'], " ")
            .trim()
            .to_owned();
        let candidate = format!("{topic}\n({}) {name}", index + 1);
        if candidate.chars().count() + 1 + updated.chars().count() <= max_chars {
            topic = candidate;
        } else {
            topic.push_str("\n…");
            break;
        }
    }
    topic.push('\n');
    topic.push_str(&updated);
    topic
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(display_name: &str) -> AttendanceSession {
        AttendanceSession {
            id: 1,
            guild_id: 1,
            user_id: 1,
            display_name: display_name.into(),
            started_at: 0,
            ended_at: None,
            open_since: Some(0),
            note: None,
            created_at: 0,
            updated_at: 0,
            deleted_at: None,
        }
    }

    #[test]
    fn shows_empty_status() {
        let topic = status_topic(&[], 0);
        assert!(topic.contains(":white_circle: 現在0名活動中"));
        assert!(topic.contains("(最終更新:"));
    }

    #[test]
    fn shows_active_names() {
        let topic = status_topic(&[session("Bem130"), session("Alice")], 0);
        assert!(topic.contains(":green_circle: 現在2名活動中"));
        assert!(topic.contains("(1) Bem130"));
        assert!(topic.contains("(2) Alice"));
    }

    #[test]
    fn matches_requested_format() {
        use chrono::TimeZone;

        let updated_at = crate::time::DISPLAY_TIMEZONE
            .with_ymd_and_hms(2026, 8, 9, 11, 1, 0)
            .single()
            .unwrap()
            .timestamp();
        assert_eq!(
            status_topic(&[session("𝕭𝖊𝖒 Estas Malsaĝulo / 蓓眸")], updated_at),
            ":green_circle: 現在1名活動中\n(1) 𝕭𝖊𝖒 Estas Malsaĝulo / 蓓眸\n(最終更新: 2026年8月9日 11:01)"
        );
    }

    #[test]
    fn limits_long_topics() {
        let topic = status_topic(&[session(&"x".repeat(2_000))], 0);
        assert!(topic.chars().count() <= MAX_TOPIC_CHARS);
        assert!(topic.contains('…'));
    }

    #[test]
    fn limits_activity_status() {
        let activity = activity_status(&[session(&"x".repeat(2_000))], 0);
        assert!(activity.chars().count() <= MAX_ACTIVITY_CHARS);
        assert!(activity.contains('…'));
    }

    #[test]
    fn uses_unicode_emojis_for_activity() {
        let activity = activity_status(&[], 0);
        assert!(activity.starts_with("⚪ 現在0名活動中"));
        assert!(!activity.contains(":white_circle:"));
    }
}
