use poise::serenity_prelude as serenity;

use crate::{
    attendance::{AttendanceSession, MonthlyAttendance},
    time::{format_duration, format_history_range},
};

pub fn history_embed(
    display_name: &str,
    sessions: &[AttendanceSession],
    now: i64,
) -> serenity::CreateEmbed {
    let mut total = 0;
    let body = if sessions.is_empty() {
        "活動記録はまだない。".to_owned()
    } else {
        sessions
            .iter()
            .map(|s| {
                let duration = s.duration_seconds_at(now);
                total += duration;
                let head = if s.ended_at.is_none() {
                    format!("🟢 `#{}`  **活動中**", s.id)
                } else {
                    format!("`#{}`", s.id)
                };
                let mut text = format!(
                    "{head}\n{}\n**{}**",
                    format_history_range(s.started_at, s.ended_at),
                    format_duration(duration)
                );
                if let Some(note) = s.note.as_deref().filter(|x| !x.trim().is_empty()) {
                    text.push_str(&format!("\n{note}"));
                }
                text
            })
            .collect::<Vec<_>>()
            .join("\n\n")
    };
    serenity::CreateEmbed::new()
        .title("最近の活動記録")
        .author(serenity::CreateEmbedAuthor::new(display_name))
        .description(body)
        .footer(serenity::CreateEmbedFooter::new(format!(
            "表示中の合計: {}",
            format_duration(total)
        )))
}

pub fn month_embed(display_name: &str, monthly: &MonthlyAttendance) -> serenity::CreateEmbed {
    let average = if monthly.session_count == 0 {
        0
    } else {
        monthly.total_seconds / monthly.session_count as i64
    };
    let daily = if monthly.daily_totals.is_empty() {
        "この月の活動記録はない。".into()
    } else {
        monthly
            .daily_totals
            .iter()
            .map(|d| {
                format!(
                    "`{}`  {}",
                    d.date.format("%m/%d"),
                    format_duration(d.total_seconds)
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    serenity::CreateEmbed::new()
        .title(format!(
            "{}年{}月の活動記録",
            monthly.year_month.year, monthly.year_month.month
        ))
        .author(serenity::CreateEmbedAuthor::new(display_name))
        .field("合計活動時間", format_duration(monthly.total_seconds), true)
        .field("活動回数", format!("{}回", monthly.session_count), true)
        .field("1回あたり平均", format_duration(average), true)
        .field("日別", daily, false)
}
