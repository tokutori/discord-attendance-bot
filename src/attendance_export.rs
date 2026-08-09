mod pdf;

use std::borrow::Cow;
use std::collections::BTreeMap;

use chrono::{DateTime, Datelike, Duration, NaiveDate, TimeZone, Utc};

use crate::{attendance::YearMonth, repository::UserProfile, time};

pub use pdf::to_pdf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportRow {
    pub user_id: i64,
    pub display_name: String,
    pub generation: Option<i64>,
    pub real_name: Option<String>,
    pub role: Option<String>,
    pub daily_seconds: Vec<i64>,
    pub total_seconds: i64,
}

impl ExportRow {
    pub fn export_name(&self) -> &str {
        self.real_name.as_deref().unwrap_or(&self.display_name)
    }

    pub fn pdf_identity(&self) -> String {
        let mut parts = Vec::new();
        if let Some(generation) = self.generation {
            parts.push(format!("{generation}代"));
        }
        parts.push(self.export_name().to_owned());
        if let Some(role) = self.role.as_deref() {
            parts.push(format!("（{role}）"));
        }
        parts.join(" ")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonthlyExport {
    pub year_month: YearMonth,
    pub dates: Vec<NaiveDate>,
    pub rows: Vec<ExportRow>,
    pub notices: Vec<String>,
}

impl MonthlyExport {
    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    pub fn pdf_column_count(&self) -> usize {
        self.dates.len() + 2
    }

    pub fn csv_column_count(&self) -> usize {
        self.dates.len() + 5
    }
}

pub fn build_monthly_export(
    year_month: YearMonth,
    sessions: &[crate::attendance::AttendanceSession],
    profiles: &[UserProfile],
    now: DateTime<Utc>,
) -> anyhow::Result<MonthlyExport> {
    let (range_start, range_end) = time::month_bounds(year_month)?;
    let first_date = NaiveDate::from_ymd_opt(year_month.year, year_month.month, 1)
        .ok_or_else(|| anyhow::anyhow!("invalid export month"))?;
    let next_date = if year_month.month == 12 {
        NaiveDate::from_ymd_opt(year_month.year + 1, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(year_month.year, year_month.month + 1, 1)
    }
    .ok_or_else(|| anyhow::anyhow!("invalid next export month"))?;
    let day_count = (next_date - first_date).num_days() as usize;
    let dates = (0..day_count)
        .map(|offset| first_date + Duration::days(offset as i64))
        .collect::<Vec<_>>();

    let profiles = profiles
        .iter()
        .map(|profile| (profile.user_id, profile))
        .collect::<BTreeMap<_, _>>();
    let mut rows = BTreeMap::<i64, ExportRow>::new();
    for session in sessions {
        let effective_start = session.started_at.max(range_start);
        let effective_end = session
            .ended_at
            .unwrap_or_else(|| now.timestamp())
            .min(range_end)
            .min(now.timestamp());
        if effective_end <= effective_start {
            continue;
        }
        let row = rows.entry(session.user_id).or_insert_with(|| {
            let profile = profiles.get(&session.user_id);
            ExportRow {
                user_id: session.user_id,
                display_name: session.display_name.clone(),
                generation: profile.and_then(|value| value.generation),
                real_name: profile.and_then(|value| value.real_name.clone()),
                role: profile.and_then(|value| value.role.clone()),
                daily_seconds: vec![0; day_count],
                total_seconds: 0,
            }
        });
        if !session.display_name.trim().is_empty() {
            row.display_name = session.display_name.clone();
        }
        let mut cursor = effective_start;
        while cursor < effective_end {
            let local = DateTime::<Utc>::from_timestamp(cursor, 0)
                .ok_or_else(|| anyhow::anyhow!("invalid session timestamp"))?
                .with_timezone(&time::DISPLAY_TIMEZONE);
            let date = local.date_naive();
            let Some(day_index): Option<usize> = date
                .signed_duration_since(first_date)
                .num_days()
                .try_into()
                .ok()
            else {
                break;
            };
            if day_index >= day_count {
                break;
            }
            let next_midnight = time::DISPLAY_TIMEZONE
                .from_local_datetime(&(date + Duration::days(1)).and_hms_opt(0, 0, 0).unwrap())
                .single()
                .ok_or_else(|| anyhow::anyhow!("invalid local midnight"))?
                .timestamp();
            let segment_end = effective_end.min(next_midnight);
            let seconds = segment_end - cursor;
            row.daily_seconds[day_index] += seconds;
            row.total_seconds += seconds;
            cursor = segment_end;
        }
    }

    let local_now = now.with_timezone(&time::DISPLAY_TIMEZONE);
    let mut notices = Vec::new();
    if now.timestamp() < range_end {
        notices
            .push("対象月はまだ終了していないため、現在活動中の時間を含む暫定集計である。".into());
    }
    if local_now.date_naive() == next_date {
        notices.push(
            "対象月の翌月1日である。ユーザーが記録を修正する可能性があるため、確定版ではない。"
                .into(),
        );
    }

    let mut rows = rows.into_values().collect::<Vec<_>>();
    rows.sort_by_key(|row| {
        (
            row.generation.is_none(),
            row.generation.unwrap_or_default(),
            row.export_name().to_owned(),
            row.user_id,
        )
    });

    Ok(MonthlyExport {
        year_month,
        dates,
        rows,
        notices,
    })
}

pub fn format_cell_duration(seconds: i64) -> String {
    if seconds <= 0 {
        return String::new();
    }
    format_duration_hm(seconds)
}

fn format_csv_cell_duration(seconds: i64) -> String {
    if seconds <= 0 {
        return "0:00".into();
    }
    format_duration_hm(seconds)
}

fn format_duration_hm(seconds: i64) -> String {
    let total_minutes = seconds.max(0) / 60;
    format!("{}:{:02}", total_minutes / 60, total_minutes % 60)
}

fn csv_escape(value: &str) -> String {
    if value.contains([',', '"', '\r', '\n']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_owned()
    }
}

fn is_csv_formula_prefix_padding(character: char) -> bool {
    character.is_whitespace()
        || character.is_control()
        || matches!(
            character,
            '\u{feff}' | '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{206f}'
        )
}

fn sanitize_csv_cell_content(value: &str) -> Cow<'_, str> {
    let first_significant = value
        .chars()
        .find(|character| !is_csv_formula_prefix_padding(*character));
    if matches!(first_significant, Some('=' | '+' | '-' | '@')) {
        Cow::Owned(format!("'{value}"))
    } else {
        Cow::Borrowed(value)
    }
}

fn encode_csv_cell(value: &str) -> String {
    csv_escape(&sanitize_csv_cell_content(value))
}

pub fn to_csv(export: &MonthlyExport) -> Vec<u8> {
    let mut output = String::from("\u{feff}");
    output.push_str("代,本名,役割,Discord表示名");
    for date in &export.dates {
        output.push(',');
        output.push_str(&format!("{}日", date.day()));
    }
    output.push_str(",合計\r\n");
    for row in &export.rows {
        output.push_str(
            &row.generation
                .map(|value| value.to_string())
                .unwrap_or_default(),
        );
        output.push(',');
        output.push_str(&encode_csv_cell(row.export_name()));
        output.push(',');
        output.push_str(&encode_csv_cell(row.role.as_deref().unwrap_or_default()));
        output.push(',');
        output.push_str(&encode_csv_cell(&row.display_name));
        for seconds in &row.daily_seconds {
            output.push(',');
            output.push_str(&format_csv_cell_duration(*seconds));
        }
        output.push(',');
        output.push_str(&format_csv_cell_duration(row.total_seconds));
        output.push_str("\r\n");
    }
    output.into_bytes()
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;
    use crate::attendance::AttendanceSession;

    fn session(
        user_id: i64,
        display_name: &str,
        started_at: i64,
        ended_at: i64,
    ) -> AttendanceSession {
        AttendanceSession {
            id: user_id,
            guild_id: 1,
            user_id,
            display_name: display_name.into(),
            started_at,
            ended_at: Some(ended_at),
            open_since: None,
            note: None,
            created_at: started_at,
            updated_at: ended_at,
            deleted_at: None,
        }
    }

    fn profile(user_id: i64) -> UserProfile {
        profile_with(user_id, Some(5), Some("山田太郎"), Some("代表"))
    }

    fn profile_with(
        user_id: i64,
        generation: Option<i64>,
        real_name: Option<&str>,
        role: Option<&str>,
    ) -> UserProfile {
        UserProfile {
            guild_id: 1,
            user_id,
            generation,
            real_name: real_name.map(str::to_owned),
            role: role.map(str::to_owned),
            updated_at: 0,
        }
    }

    fn local_timestamp(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> i64 {
        time::DISPLAY_TIMEZONE
            .with_ymd_and_hms(year, month, day, hour, minute, 0)
            .unwrap()
            .timestamp()
    }

    #[test]
    fn builds_user_by_day_table_and_csv() {
        let timezone = time::DISPLAY_TIMEZONE;
        let start = timezone
            .with_ymd_and_hms(2026, 8, 1, 18, 0, 0)
            .unwrap()
            .timestamp();
        let end = timezone
            .with_ymd_and_hms(2026, 8, 1, 19, 30, 0)
            .unwrap()
            .timestamp();
        let export = build_monthly_export(
            YearMonth {
                year: 2026,
                month: 8,
            },
            &[session(1, "山田", start, end)],
            &[profile(1)],
            timezone
                .with_ymd_and_hms(2026, 8, 2, 12, 0, 0)
                .unwrap()
                .with_timezone(&Utc),
        )
        .unwrap();
        assert_eq!(export.row_count(), 1);
        assert_eq!(export.pdf_column_count(), 33);
        assert_eq!(export.csv_column_count(), 36);
        assert_eq!(export.rows[0].daily_seconds[0], 90 * 60);
        let csv = String::from_utf8(to_csv(&export)).unwrap();
        assert!(csv.contains("5,山田太郎,代表,山田"));
        assert!(csv.contains(",0:00"));
    }

    #[test]
    fn leaves_zero_duration_pdf_cells_blank_but_csv_cells_explicit() {
        assert_eq!(format_cell_duration(0), "");
        assert_eq!(format_cell_duration(-1), "");
        assert_eq!(format_cell_duration(60), "0:01");
        assert_eq!(format_csv_cell_duration(0), "0:00");
    }

    #[test]
    fn sanitizes_csv_formula_prefixes_after_whitespace_and_control_characters() {
        let dangerous_values = [
            "=1+1",
            "+SUM(A1:A2)",
            "-2+3",
            "@SUM(A1:A2)",
            "  =1+1",
            "\t\r\n+cmd",
            "\u{0007} -2+3",
            "\u{3000}@SUM(A1:A2)",
            "\u{feff}\u{200b}=1+1",
        ];
        for value in dangerous_values {
            assert_eq!(
                sanitize_csv_cell_content(value).as_ref(),
                format!("'{value}")
            );
        }

        for value in ["", "山田太郎", " 山田太郎", "123", "'=-1", "\t通常名"] {
            assert_eq!(sanitize_csv_cell_content(value).as_ref(), value);
        }
    }

    #[test]
    fn keeps_csv_content_sanitizing_separate_from_csv_syntax_escaping() {
        assert_eq!(csv_escape("plain"), "plain");
        assert_eq!(csv_escape("姓,名"), "\"姓,名\"");
        assert_eq!(csv_escape("a\"b"), "\"a\"\"b\"");
        assert_eq!(csv_escape("a\rb\n"), "\"a\rb\n\"");

        assert_eq!(sanitize_csv_cell_content("通常,氏名").as_ref(), "通常,氏名");
        assert_eq!(encode_csv_cell("通常,氏名"), "\"通常,氏名\"");
        assert_eq!(encode_csv_cell(" \t=SUM(1,2)"), "\"' \t=SUM(1,2)\"");
        assert_eq!(
            encode_csv_cell("=HYPERLINK(\"x\",\"y\")"),
            "\"'=HYPERLINK(\"\"x\"\",\"\"y\"\")\""
        );
    }

    #[test]
    fn sanitizes_all_user_derived_csv_columns() {
        let export = MonthlyExport {
            year_month: YearMonth {
                year: 2026,
                month: 8,
            },
            dates: vec![NaiveDate::from_ymd_opt(2026, 8, 1).unwrap()],
            rows: vec![ExportRow {
                user_id: 1,
                display_name: "\u{0007}@discord".into(),
                generation: Some(5),
                real_name: Some(" \t=SUM(1,2)".into()),
                role: Some("\n+代表".into()),
                daily_seconds: vec![0],
                total_seconds: 0,
            }],
            notices: Vec::new(),
        };

        let csv = String::from_utf8(to_csv(&export)).unwrap();
        assert!(csv.contains("\"' \t=SUM(1,2)\""));
        assert!(csv.contains("\"'\n+代表\""));
        assert!(csv.contains("'\u{0007}@discord"));
    }

    #[test]
    fn splits_sessions_at_local_midnight() {
        let export = build_monthly_export(
            YearMonth {
                year: 2026,
                month: 8,
            },
            &[session(
                1,
                "山田",
                local_timestamp(2026, 8, 1, 23, 30),
                local_timestamp(2026, 8, 2, 0, 30),
            )],
            &[],
            time::DISPLAY_TIMEZONE
                .with_ymd_and_hms(2026, 9, 2, 0, 0, 0)
                .unwrap()
                .with_timezone(&Utc),
        )
        .unwrap();

        assert_eq!(export.rows[0].daily_seconds[0], 30 * 60);
        assert_eq!(export.rows[0].daily_seconds[1], 30 * 60);
        assert_eq!(export.rows[0].total_seconds, 60 * 60);
    }

    #[test]
    fn clips_december_session_at_the_year_boundary() {
        let export = build_monthly_export(
            YearMonth {
                year: 2025,
                month: 12,
            },
            &[session(
                1,
                "山田",
                local_timestamp(2025, 12, 31, 23, 30),
                local_timestamp(2026, 1, 1, 0, 30),
            )],
            &[],
            time::DISPLAY_TIMEZONE
                .with_ymd_and_hms(2026, 1, 2, 0, 0, 0)
                .unwrap()
                .with_timezone(&Utc),
        )
        .unwrap();

        assert_eq!(export.dates.len(), 31);
        assert_eq!(export.dates.last().unwrap().day(), 31);
        assert_eq!(export.rows[0].daily_seconds[30], 30 * 60);
        assert_eq!(export.rows[0].total_seconds, 30 * 60);
    }

    #[test]
    fn includes_leap_day_and_clips_at_march() {
        let export = build_monthly_export(
            YearMonth {
                year: 2024,
                month: 2,
            },
            &[session(
                1,
                "山田",
                local_timestamp(2024, 2, 29, 23, 30),
                local_timestamp(2024, 3, 1, 0, 30),
            )],
            &[],
            time::DISPLAY_TIMEZONE
                .with_ymd_and_hms(2024, 3, 2, 0, 0, 0)
                .unwrap()
                .with_timezone(&Utc),
        )
        .unwrap();

        assert_eq!(export.dates.len(), 29);
        assert_eq!(export.dates.last().unwrap().day(), 29);
        assert_eq!(export.rows[0].daily_seconds[28], 30 * 60);
        assert_eq!(export.rows[0].total_seconds, 30 * 60);
    }

    #[test]
    fn sorts_profiles_by_generation_and_export_name_with_display_name_fallback() {
        let started_at = local_timestamp(2026, 8, 1, 10, 0);
        let ended_at = local_timestamp(2026, 8, 1, 11, 0);
        let sessions = [
            session(1, "Zulu", started_at, ended_at),
            session(2, "表示Beta", started_at, ended_at),
            session(3, "Gamma", started_at, ended_at),
            session(4, "表示Alpha", started_at, ended_at),
        ];
        let profiles = [
            profile_with(2, Some(2), Some("Beta"), None),
            profile_with(3, Some(1), None, Some("会計")),
            profile_with(4, Some(1), Some("Alpha"), Some("代表")),
        ];

        let export = build_monthly_export(
            YearMonth {
                year: 2026,
                month: 8,
            },
            &sessions,
            &profiles,
            time::DISPLAY_TIMEZONE
                .with_ymd_and_hms(2026, 9, 2, 0, 0, 0)
                .unwrap()
                .with_timezone(&Utc),
        )
        .unwrap();

        assert_eq!(
            export
                .rows
                .iter()
                .map(|row| row.user_id)
                .collect::<Vec<_>>(),
            vec![4, 3, 2, 1]
        );
        assert_eq!(export.rows[1].export_name(), "Gamma");
        assert_eq!(export.rows[3].export_name(), "Zulu");
    }

    #[test]
    fn renders_pdf_when_a_system_font_is_available() {
        if pdf::pdf_font_path().is_err() {
            return;
        }
        let export = MonthlyExport {
            year_month: YearMonth {
                year: 2026,
                month: 8,
            },
            dates: vec![NaiveDate::from_ymd_opt(2026, 8, 1).unwrap()],
            rows: vec![ExportRow {
                user_id: 1,
                display_name: "山田".into(),
                generation: Some(5),
                real_name: Some("山田太郎".into()),
                role: Some("代表".into()),
                daily_seconds: vec![3600],
                total_seconds: 3600,
            }],
            notices: vec!["暫定集計".into()],
        };
        let pdf = to_pdf(&export).unwrap();
        assert!(pdf.starts_with(b"%PDF-"));
        assert!(pdf.len() < 5_000_000);
    }
}
