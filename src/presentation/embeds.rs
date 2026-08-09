use poise::serenity_prelude as serenity;

use crate::{
    attendance::{AttendanceSession, MonthlyAttendance},
    time::{format_duration, format_history_range},
};

const DEFAULT_COLOR: (u8, u8, u8) = (88, 101, 242);
const ERROR_COLOR: (u8, u8, u8) = (237, 66, 69);
const MAX_DESCRIPTION_CHARS: usize = 4096;

pub fn response_embed(
    title: impl Into<String>,
    description: impl Into<String>,
) -> serenity::CreateEmbed {
    serenity::CreateEmbed::new()
        .title(title)
        .description(description)
        .color(DEFAULT_COLOR)
}

pub fn error_embed(
    title: impl Into<String>,
    description: impl Into<String>,
) -> serenity::CreateEmbed {
    serenity::CreateEmbed::new()
        .title(title)
        .description(description)
        .color(ERROR_COLOR)
}

pub fn help_embed(display_name: &str) -> serenity::CreateEmbed {
    serenity::CreateEmbed::new()
        .title("活動時間記録 ヘルプ")
        .author(serenity::CreateEmbedAuthor::new(display_name))
        .description(
            "活動時間を記録・確認・修正するためのコマンド一覧。\n時刻の入力と表示は日本時間。通常の応答は本人にだけ表示される。\n月次ファイル出力は `/attendanceexport help` を参照。",
        )
        .field(
            "▶ 日常の操作コマンド",
            "`/attendance start [at] [note]`\n活動を開始する。`at` は `HH:MM`（省略時は現在時刻）。\n\n`/attendance end [at] [note]`\n活動を終了する。\n\n`/attendance continue`\n直近の終了済み記録を活動中へ戻す。",
            false,
        )
        .field(
            "🔐 確認が必要な特殊操作コマンド",
            "`/attendance revert`\n直前の成功した変更を1件取り消す。start/end/continue/edit/delete が対象。1回確定した後にもう一度 `revert` → `confirm` を行えば、過去の変更を順に戻せる。\n\n`/attendance edit record [start] [end] [note]`\n変更前後を表示する。日時は `YYYY-MM-DD HH:MM`。`end` を空文字にすると活動中へ戻せる。\n\n`/attendance delete record`\n削除対象を表示する。削除は soft delete。\n\n`/attendance confirm [id]`\n発行済みの確認IDで変更を確定する。",
            false,
        )
        .field(
            "🔎 閲覧・集計コマンド",
            "`/attendance status`\n現在活動中か確認する。\n\n`/attendance history [limit]`\n最近の記録を表示する（1〜20件、既定5件）。\n\n`/attendance month [target]`\n月次集計を表示する。`target` は `YYYY-MM`（省略時は当月）。",
            false,
        )
        .field(
            "🔐 確認の流れ",
            "`revert` / `edit` / `delete` を実行すると、変更内容と5文字の確認IDが表示される。\n\n確認IDの有効期限は5分。確認前はDBを変更しない。\n`/attendance confirm id:<ID>` で確定する。IDは本人の要求にのみ使用でき、使用済みIDは再利用できない。別の変更が入った場合は安全のため確定されない。",
            false,
        )
        .footer(serenity::CreateEmbedFooter::new(
            "すべてのコマンド応答は本人にだけ表示される",
        ))
        .color(DEFAULT_COLOR)
}

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
    let description = truncate_chars(&body, MAX_DESCRIPTION_CHARS);
    let footer = if description == body {
        format!("表示中の合計: {}", format_duration(total))
    } else {
        format!(
            "取得した記録の合計: {}（表示は上限まで）",
            format_duration(total)
        )
    };

    serenity::CreateEmbed::new()
        .title("最近の活動記録")
        .author(serenity::CreateEmbedAuthor::new(display_name))
        .description(description)
        .color(DEFAULT_COLOR)
        .footer(serenity::CreateEmbedFooter::new(footer))
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
        .color(DEFAULT_COLOR)
        .field("合計活動時間", format_duration(monthly.total_seconds), true)
        .field("活動回数", format!("{}回", monthly.session_count), true)
        .field("1回あたり平均", format_duration(average), true)
        .field("日別", daily, false)
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_owned();
    }

    let take = max_chars.saturating_sub(1);
    let mut truncated = value.chars().take(take).collect::<String>();
    truncated.push('…');
    truncated
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncates_without_splitting_utf8() {
        assert_eq!(truncate_chars("あいうえお", 4), "あいう…");
        assert_eq!(truncate_chars("abc", 4), "abc");
    }
}
