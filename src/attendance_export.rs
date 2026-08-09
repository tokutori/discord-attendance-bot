use std::{collections::BTreeMap, env, path::PathBuf};

use chrono::{DateTime, Datelike, Duration, NaiveDate, TimeZone, Utc};
use printpdf::{
    Line, LinePoint, Mm, Op, ParsedFont, PdfDocument, PdfFontHandle, PdfPage, PdfSaveOptions,
    Point, Pt, TextItem,
};

use crate::{attendance::YearMonth, time};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportRow {
    pub user_id: i64,
    pub display_name: String,
    pub daily_seconds: Vec<i64>,
    pub total_seconds: i64,
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

    pub fn column_count(&self) -> usize {
        self.dates.len() + 2
    }
}

pub fn build_monthly_export(
    year_month: YearMonth,
    sessions: &[crate::attendance::AttendanceSession],
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
        let row = rows.entry(session.user_id).or_insert_with(|| ExportRow {
            user_id: session.user_id,
            display_name: session.display_name.clone(),
            daily_seconds: vec![0; day_count],
            total_seconds: 0,
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

    Ok(MonthlyExport {
        year_month,
        dates,
        rows: rows.into_values().collect(),
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

pub fn to_csv(export: &MonthlyExport) -> Vec<u8> {
    let mut output = String::from("\u{feff}");
    output.push_str("ユーザー");
    for date in &export.dates {
        output.push(',');
        output.push_str(&format!("{}日", date.day()));
    }
    output.push_str(",合計\r\n");
    for row in &export.rows {
        output.push_str(&csv_escape(&row.display_name));
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

fn pdf_font_path() -> anyhow::Result<PathBuf> {
    if let Ok(path) = env::var("ATTENDANCE_PDF_FONT_PATH") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Ok(path);
        }
        anyhow::bail!(
            "ATTENDANCE_PDF_FONT_PATH に指定したフォントが存在しない: {}",
            path.display()
        );
    }
    let candidates = [
        PathBuf::from(r"C:\Windows\Fonts\NotoSansJP-VF.ttf"),
        PathBuf::from(r"C:\Windows\Fonts\SimsunExtG.ttf"),
        PathBuf::from("/usr/share/fonts/truetype/noto/NotoSansJP-Regular.ttf"),
    ];
    candidates
        .into_iter()
        .find(|path| path.is_file())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "日本語PDF用フォントが見つからない。ATTENDANCE_PDF_FONT_PATH に TTF フォントを指定してほしい"
            )
    })
}

fn add_pdf_text(
    ops: &mut Vec<Op>,
    font: &PdfFontHandle,
    text: impl Into<String>,
    x: f32,
    y: f32,
    size: f32,
) {
    ops.push(Op::SetFont {
        font: font.clone(),
        size: Pt(size),
    });
    ops.push(Op::StartTextSection);
    ops.push(Op::SetTextCursor {
        pos: Point {
            x: Mm(x).into(),
            y: Mm(y).into(),
        },
    });
    ops.push(Op::ShowText {
        items: vec![TextItem::Text(text.into())],
    });
    ops.push(Op::EndTextSection);
}

fn truncate_pdf_text(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let mut result = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        result.push('…');
    }
    result
}

fn centered_text_y(top: f32, row_height: f32, font_size: f32) -> f32 {
    let font_height_mm = font_size * 0.3528;
    top - row_height / 2.0 - font_height_mm * 0.3
}

fn add_pdf_grid(
    ops: &mut Vec<Op>,
    x_positions: &[f32],
    top: f32,
    row_height: f32,
    row_count: usize,
) {
    let bottom = top - row_height * row_count as f32;
    ops.push(Op::SetOutlineThickness { pt: Pt(0.2) });
    for &x in x_positions {
        ops.push(Op::DrawLine {
            line: Line {
                points: vec![
                    LinePoint {
                        p: Point {
                            x: Mm(x).into(),
                            y: Mm(top).into(),
                        },
                        bezier: false,
                    },
                    LinePoint {
                        p: Point {
                            x: Mm(x).into(),
                            y: Mm(bottom).into(),
                        },
                        bezier: false,
                    },
                ],
                is_closed: false,
            },
        });
    }
    for row_index in 0..=row_count {
        let y = top - row_height * row_index as f32;
        ops.push(Op::DrawLine {
            line: Line {
                points: vec![
                    LinePoint {
                        p: Point {
                            x: Mm(x_positions[0]).into(),
                            y: Mm(y).into(),
                        },
                        bezier: false,
                    },
                    LinePoint {
                        p: Point {
                            x: Mm(*x_positions.last().unwrap()).into(),
                            y: Mm(y).into(),
                        },
                        bezier: false,
                    },
                ],
                is_closed: false,
            },
        });
    }
}

fn add_pdf_table(
    ops: &mut Vec<Op>,
    export: &MonthlyExport,
    rows: &[ExportRow],
    font: &PdfFontHandle,
    top: f32,
) {
    let page_width = 297.0_f32;
    let margin = 8.0_f32;
    let user_width = 35.0_f32;
    let total_width = 18.0_f32;
    let day_width =
        (page_width - margin * 2.0 - user_width - total_width) / export.dates.len().max(1) as f32;
    let mut x_positions = vec![margin, margin + user_width];
    x_positions.extend(
        (1..=export.dates.len()).map(|index| margin + user_width + day_width * index as f32),
    );
    x_positions.push(page_width - margin);

    let row_height = 8.0_f32;
    add_pdf_grid(ops, &x_positions, top, row_height, rows.len() + 1);

    add_pdf_text(
        ops,
        font,
        "ユーザー",
        margin + 1.0,
        centered_text_y(top, row_height, 7.5),
        7.5,
    );
    for (index, date) in export.dates.iter().enumerate() {
        add_pdf_text(
            ops,
            font,
            date.day().to_string(),
            x_positions[index + 1] + 1.0,
            centered_text_y(top, row_height, 6.3),
            6.3,
        );
    }
    add_pdf_text(
        ops,
        font,
        "合計",
        x_positions[x_positions.len() - 2] + 1.0,
        centered_text_y(top, row_height, 6.3),
        6.3,
    );

    for (row_index, row) in rows.iter().enumerate() {
        let row_top = top - row_height * (row_index + 1) as f32;
        add_pdf_text(
            ops,
            font,
            truncate_pdf_text(&row.display_name, 13),
            margin + 1.0,
            centered_text_y(row_top, row_height, 7.5),
            7.5,
        );
        for (day_index, seconds) in row.daily_seconds.iter().enumerate() {
            add_pdf_text(
                ops,
                font,
                format_cell_duration(*seconds),
                x_positions[day_index + 1] + 1.0,
                centered_text_y(row_top, row_height, 6.3),
                6.3,
            );
        }
        add_pdf_text(
            ops,
            font,
            format_cell_duration(row.total_seconds),
            x_positions[x_positions.len() - 2] + 1.0,
            centered_text_y(row_top, row_height, 6.3),
            6.3,
        );
    }
}

pub fn to_pdf(export: &MonthlyExport) -> anyhow::Result<Vec<u8>> {
    let font_path = pdf_font_path()?;
    let font_bytes = std::fs::read(&font_path)
        .map_err(|error| anyhow::anyhow!("PDFフォントを読み込めない: {error}"))?;
    let parsed_font = ParsedFont::from_bytes(&font_bytes, 0, &mut Vec::new())
        .ok_or_else(|| anyhow::anyhow!("PDFフォントを解析できない"))?;
    let title = format!(
        "活動時間集計 {}-{:02}",
        export.year_month.year, export.year_month.month
    );
    let mut document = PdfDocument::new(&title);
    let font_id = document.add_font(&parsed_font);
    let font = PdfFontHandle::External(font_id);

    let rows_per_page = 19;
    let row_chunks = if export.rows.is_empty() {
        vec![&[][..]]
    } else {
        export.rows.chunks(rows_per_page).collect::<Vec<_>>()
    };
    let mut pages = Vec::with_capacity(row_chunks.len());
    for (page_index, rows) in row_chunks.into_iter().enumerate() {
        let mut ops = Vec::new();
        add_pdf_text(
            &mut ops,
            &font,
            format!(
                "活動時間集計 {}-{:02}",
                export.year_month.year, export.year_month.month
            ),
            8.0,
            201.0,
            10.0,
        );
        let mut notice_y = 195.0_f32;
        for notice in &export.notices {
            add_pdf_text(
                &mut ops,
                &font,
                format!("注記: {notice}"),
                8.0,
                notice_y,
                5.0,
            );
            notice_y -= 5.0;
        }
        if page_index > 0 {
            add_pdf_text(
                &mut ops,
                &font,
                format!("続き ({})", page_index + 1),
                260.0,
                201.0,
                5.0,
            );
        }
        add_pdf_table(&mut ops, export, rows, &font, 185.0);
        pages.push(PdfPage::new(Mm(297.0), Mm(210.0), ops));
    }

    let mut warnings = Vec::new();
    let options = PdfSaveOptions {
        subset_fonts: true,
        ..PdfSaveOptions::default()
    };
    Ok(document.with_pages(pages).save(&options, &mut warnings))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attendance::AttendanceSession;
    use chrono::TimeZone;

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
            note: None,
            created_at: started_at,
            updated_at: ended_at,
            deleted_at: None,
        }
    }

    #[test]
    fn builds_user_by_day_table_and_csv() {
        let tz = time::DISPLAY_TIMEZONE;
        let start = tz
            .with_ymd_and_hms(2026, 8, 1, 18, 0, 0)
            .unwrap()
            .timestamp();
        let end = tz
            .with_ymd_and_hms(2026, 8, 1, 19, 30, 0)
            .unwrap()
            .timestamp();
        let export = build_monthly_export(
            YearMonth {
                year: 2026,
                month: 8,
            },
            &[session(1, "山田", start, end)],
            tz.with_ymd_and_hms(2026, 8, 2, 12, 0, 0)
                .unwrap()
                .with_timezone(&Utc),
        )
        .unwrap();
        assert_eq!(export.row_count(), 1);
        assert_eq!(export.column_count(), 33);
        assert_eq!(export.rows[0].daily_seconds[0], 90 * 60);
        let csv = String::from_utf8(to_csv(&export)).unwrap();
        assert!(csv.contains("山田"));
        assert!(csv.contains(",0:00"));
    }

    #[test]
    fn leaves_zero_duration_cells_blank() {
        assert_eq!(format_cell_duration(0), "");
        assert_eq!(format_cell_duration(-1), "");
        assert_eq!(format_cell_duration(60), "0:01");
        assert_eq!(format_csv_cell_duration(0), "0:00");
    }

    #[test]
    fn renders_pdf_when_a_system_font_is_available() {
        if pdf_font_path().is_err() {
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
