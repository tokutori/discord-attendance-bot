use poise::serenity_prelude as serenity;

use crate::{
    attendance::{AttendanceSession, MonthlyAttendance},
    repository::{ActiveAttendanceMember, AutoEndNotice, UserProfile},
    time::{format_duration, format_history_range},
};

const DEFAULT_COLOR: (u8, u8, u8) = (88, 101, 242);
const ERROR_COLOR: (u8, u8, u8) = (237, 66, 69);
const MAX_DESCRIPTION_CHARS: usize = 4096;

pub fn response_embed(
    title: impl Into<String>,
    description: impl Into<String>,
) -> serenity::CreateEmbed {
    let description = truncate_chars(&description.into(), MAX_DESCRIPTION_CHARS);
    serenity::CreateEmbed::new()
        .title(title)
        .description(description)
        .color(DEFAULT_COLOR)
}

pub fn error_embed(
    title: impl Into<String>,
    description: impl Into<String>,
) -> serenity::CreateEmbed {
    let description = truncate_chars(&description.into(), MAX_DESCRIPTION_CHARS);
    serenity::CreateEmbed::new()
        .title(title)
        .description(description)
        .color(ERROR_COLOR)
}

pub fn auto_end_notice_text(notice: &AutoEndNotice) -> String {
    if notice.corrected_at.is_some() {
        format!(
            "前回の終了忘れによる記録 #{} の自動終了（{}）は、ユーザー入力を正として扱った。",
            notice.session_id,
            crate::time::format_datetime(notice.automatic_ended_at)
        )
    } else {
        format!(
            "前回の終了忘れにより、記録 #{} は {} に自動終了として扱った。実際の終了時刻が異なる場合は `/attendance edit record:{}` で修正してほしい。",
            notice.session_id,
            crate::time::format_datetime(notice.automatic_ended_at),
            notice.session_id
        )
    }
}

pub fn export_help_embed() -> serenity::CreateEmbed {
    serenity::CreateEmbed::new()
        .title("活動時間エクスポート ヘルプ")
        .description("月単位の活動時間を、Discord表示名あり・本名のみのCSV/PDF（計4ファイル）で出力する。month は必須。通常は preview で本人だけに送信する。")
        .field(
            "使い方",
            "`/attendanceexport export month:YYYY-MM [mode:preview|publish]`\n月次帳票を出力する。\n\n`/attendanceexport userconfig [generation] [real_name] [role] [name_reading]`\n代・本名・役割と名簿用の読みを設定する。全項目を省略すると現在値を表示する。\n\n`/attendanceexport help`\nこのヘルプを表示する。",
            false,
        )
        .field(
            "出力内容",
            "4ファイルとも活動中名簿と同じく、役割 → 代 → 名前の読み順に並べる。Discord表示名あり版は本名未設定時にDiscord表示名を使用し、本名のみ版は「未設定」と表示する。活動時間があるセルは `時間:分` 形式。PDFの0時間セルは空欄、CSVの0時間セルは `0:00` と表示する。",
            false,
        )
        .field(
            "preview / publish",
            "`preview`（既定）: 実行者だけに表示する。\n`publish`: サーバー全員が見られるメッセージとして送信する。\n\n全員分の本名と活動時間を扱うため、exportの実行には「サーバー管理」権限が必要。",
            false,
        )
        .field(
            "集計上の注記",
            "対象月が終了していない場合は暫定集計と明記する。翌月1日に出力した場合も、修正の可能性があるため確定版ではないと明記する。",
            false,
        )
        .footer(serenity::CreateEmbedFooter::new(
            "PDFは日本語フォントの設定が必要な場合がある",
        ))
        .color(DEFAULT_COLOR)
}

pub fn user_profile_embed(profile: Option<&UserProfile>, updated: bool) -> serenity::CreateEmbed {
    let title = if updated {
        "ユーザー設定を更新した"
    } else {
        "現在のユーザー設定"
    };
    let generation = profile
        .and_then(|value| value.generation)
        .map(|value| format!("{value}代"))
        .unwrap_or_else(|| "未設定".into());
    let real_name = profile
        .and_then(|value| value.real_name.as_deref())
        .unwrap_or("未設定");
    let role = profile
        .and_then(|value| value.role.as_deref())
        .unwrap_or("未設定");
    let name_reading = profile
        .and_then(|value| value.name_reading.as_deref())
        .unwrap_or("未設定");
    serenity::CreateEmbed::new()
        .title(title)
        .description("この設定は月次CSV・PDFと活動中名簿のユーザー情報に使用する。")
        .field("代", generation, true)
        .field("本名", real_name, true)
        .field("役割", role, true)
        .field("名前の読み", name_reading, true)
        .footer(serenity::CreateEmbedFooter::new(
            "未指定の項目は既存値を維持する",
        ))
        .color(DEFAULT_COLOR)
}

pub fn active_roster_embeds(members: &[ActiveAttendanceMember]) -> Vec<serenity::CreateEmbed> {
    if members.is_empty() {
        return vec![response_embed(
            "現在活動中の名簿",
            "現在、活動中のメンバーはいない。",
        )];
    }

    const ROSTER_PAGE_CHARS: usize = 3_900;
    let mut pages = Vec::new();
    let mut body = String::new();
    let mut previous_role: Option<&str> = None;
    let mut previous_generation: Option<Option<i64>> = None;
    for member in members {
        let role_source = member.role.as_deref().unwrap_or("未設定");
        let role = escape_roster_markdown(role_source);
        let generation = member
            .generation
            .map(|value| format!("{value}代"))
            .unwrap_or_else(|| "代未設定".into());
        let name = escape_roster_markdown(member.real_name.as_deref().unwrap_or("名前未設定"));
        let display_name = escape_roster_markdown(&member.display_name);
        let role_changed = previous_role != Some(role_source);
        let generation_changed = role_changed || previous_generation != Some(member.generation);
        let mut block = String::new();
        if role_changed {
            if !body.is_empty() {
                block.push('\n');
            }
            block.push_str(&format!("# {role}\n"));
        }
        if generation_changed {
            block.push_str(&format!("## {generation}\n"));
        }
        block.push_str(&format!("- {name}（{display_name}）\n"));

        if !body.is_empty() && body.chars().count() + block.chars().count() > ROSTER_PAGE_CHARS {
            pages.push(std::mem::take(&mut body));
            block = format!("# {role}\n## {generation}\n- {name}（{display_name}）\n");
        }
        body.push_str(&block);
        previous_role = Some(role_source);
        previous_generation = Some(member.generation);
    }
    if !body.is_empty() {
        pages.push(body);
    }

    let page_count = pages.len();
    pages
        .into_iter()
        .enumerate()
        .map(|(index, page)| {
            let title = if page_count == 1 {
                "現在活動中の名簿".into()
            } else {
                format!("現在活動中の名簿 ({}/{page_count})", index + 1)
            };
            serenity::CreateEmbed::new()
                .title(title)
                .description(page)
                .footer(serenity::CreateEmbedFooter::new(format!(
                    "活動中: {}名 / 役割 → 代 → 名前の読み順",
                    members.len()
                )))
                .color(DEFAULT_COLOR)
        })
        .collect()
}

fn escape_roster_markdown(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .flat_map(|character| {
            if matches!(
                character,
                '\\' | '*' | '_' | '~' | '`' | '|' | '>' | '#' | '-' | '[' | ']'
            ) {
                vec!['\\', character]
            } else {
                vec![character]
            }
        })
        .collect()
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
            "`/attendance start [at] [note]` / `/join [at] [note]`\n活動を開始する。`at` は `HH:MM`（省略時は現在時刻）。\n\n`/attendance end [at] [note]` / `/exit [at] [note]`\n活動を終了する。\n\n`/attendance continue`\n直近の終了済み記録を活動中へ戻す。",
            false,
        )
        .field(
            "🔐 確認が必要な特殊操作コマンド",
            "`/attendance revert`\n直前の成功した変更を1件取り消す。start/end/continue/edit/delete が対象。1回確定した後にもう一度 `revert` → `confirm` を行えば、過去の変更を順に戻せる。\n\n`/attendance edit record [start] [end] [note]`\n変更前後を表示する。日時は `YYYY-MM-DD HH:MM`。`end` を空文字にすると活動中へ戻せる。\n\n`/attendance delete record`\n削除対象を表示する。削除は soft delete。\n\n`/attendance confirm [id]`\n発行済みの確認IDで変更を確定する。",
            false,
        )
        .field(
            "🔎 閲覧・集計コマンド",
            "`/attendance status`\n現在活動中か確認する。\n\n`/attendance list`\n活動中の全員を役割・代・名前の読み順で表示する。\n\n`/attendance history [limit]`\n最近の記録を表示する（1〜20件、既定5件）。\n\n`/attendance month [target]`\n合計・活動回数・1回/1日/1週間あたり平均・日別集計を表示する。`target` は `YYYY-MM`（省略時は当月）。日・週平均は当月なら今日を含む経過暦日、過去月なら全日数を基準にする。",
            false,
        )
        .field(
            "🔐 確認の流れ",
            "`revert` / `edit` / `delete` を実行すると、変更内容と5文字の確認IDが表示される。\n\n確認IDの有効期限は5分。確認前はDBを変更しない。\n`/attendance confirm id:<ID>` で確定する。IDは本人の要求にのみ使用でき、使用済みIDは再利用できない。別の変更が入った場合は安全のため確定されない。",
            false,
        )
        .footer(serenity::CreateEmbedFooter::new(
            "/attendance の応答は本人のみ。管理者向けexportのpublishだけは公開される",
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
        .field(
            "1回あたり平均",
            format_duration(monthly.average_per_session()),
            true,
        )
        .field(
            "1日あたり平均",
            format!(
                "{}（{}日基準）",
                format_duration(monthly.average_per_day()),
                monthly.elapsed_calendar_days
            ),
            true,
        )
        .field(
            "1週間あたり平均",
            format_duration(monthly.average_per_week()),
            true,
        )
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

    #[test]
    fn escapes_roster_markdown_and_flattens_newlines() {
        assert_eq!(
            escape_roster_markdown("#設計\n-班_[A]"),
            "\\#設計 \\-班\\_\\[A\\]"
        );
    }
}
