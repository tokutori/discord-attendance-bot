use std::{env, path::PathBuf};

use chrono::Datelike;
use printpdf::{
    Line, LinePoint, Mm, Op, ParsedFont, PdfDocument, PdfFontHandle, PdfPage, PdfSaveOptions,
    Point, Pt, TextItem,
};

use super::{ExportRow, IdentityMode, MonthlyExport, format_cell_duration};

pub(super) fn pdf_font_path() -> anyhow::Result<PathBuf> {
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

#[derive(Debug, PartialEq)]
struct UserInfoLayout {
    lines: Vec<String>,
    font_size: f32,
    line_height: f32,
}

fn pdf_character_width_em(character: char) -> f32 {
    if character.is_ascii() || ('\u{ff61}'..='\u{ff9f}').contains(&character) {
        0.55
    } else {
        1.0
    }
}

fn wrap_pdf_text(value: &str, max_width_pt: f32, font_size: f32) -> Vec<String> {
    if value.is_empty() {
        return vec![String::new()];
    }

    let mut lines = Vec::new();
    let mut line = String::new();
    let mut line_width_pt = 0.0_f32;
    let mut previous_was_carriage_return = false;
    for character in value.chars() {
        if character == '\r' || character == '\n' {
            if character == '\n' && previous_was_carriage_return {
                previous_was_carriage_return = false;
                continue;
            }
            lines.push(std::mem::take(&mut line));
            line_width_pt = 0.0;
            previous_was_carriage_return = character == '\r';
            continue;
        }
        previous_was_carriage_return = false;
        let character_width_pt = pdf_character_width_em(character) * font_size;
        if !line.is_empty() && line_width_pt + character_width_pt > max_width_pt {
            lines.push(std::mem::take(&mut line));
            line_width_pt = 0.0;
        }
        line.push(character);
        line_width_pt += character_width_pt;
    }
    lines.push(line);
    lines
}

fn user_info_parts(row: &ExportRow, identity_mode: IdentityMode) -> Vec<String> {
    let mut parts = Vec::new();
    if let Some(generation) = row.generation {
        parts.push(format!("{generation}代"));
    }
    parts.push(identity_mode.name(row).to_owned());
    if identity_mode == IdentityMode::WithDiscordName {
        parts.push(format!("Discord: {}", row.display_name));
    }
    if let Some(role) = row.role.as_deref() {
        parts.push(format!("（{role}）"));
    }
    parts
}

fn layout_user_info(
    row: &ExportRow,
    identity_mode: IdentityMode,
    cell_width: f32,
    row_height: f32,
) -> UserInfoLayout {
    const MM_TO_PT: f32 = 72.0 / 25.4;
    const MAX_FONT_SIZE: f32 = 7.5;
    const MIN_HORIZONTAL_PADDING: f32 = 1.0;
    const VERTICAL_PADDING: f32 = 0.6;

    let max_width_pt = (cell_width - MIN_HORIZONTAL_PADDING * 2.0) * MM_TO_PT;
    let available_height = row_height - VERTICAL_PADDING * 2.0;
    let parts = user_info_parts(row, identity_mode);
    let mut font_size = MAX_FONT_SIZE;

    loop {
        let lines = parts
            .iter()
            .flat_map(|part| wrap_pdf_text(part, max_width_pt, font_size))
            .collect::<Vec<_>>();
        let line_height = font_size * 0.3528 * 1.12;
        if line_height * lines.len() as f32 <= available_height {
            return UserInfoLayout {
                lines,
                font_size,
                line_height,
            };
        }
        font_size *= 0.9;
    }
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
    identity_mode: IdentityMode,
) {
    let page_width = 297.0_f32;
    let margin = 8.0_f32;
    let user_width = 48.0_f32;
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
        "ユーザー情報",
        margin + 1.0,
        centered_text_y(top, row_height, 7.5),
        7.5,
    );
    for (index, date) in export.dates.iter().enumerate() {
        add_pdf_text(
            ops,
            font,
            date.day().to_string(),
            x_positions[index + 1] + 0.5,
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
        let user_info = layout_user_info(row, identity_mode, user_width, row_height);
        let text_height = user_info.line_height * user_info.lines.len() as f32;
        let font_height = user_info.font_size * 0.3528;
        let first_baseline = row_top - (row_height - text_height) / 2.0 - font_height * 0.8;
        for (line_index, line) in user_info.lines.into_iter().enumerate() {
            add_pdf_text(
                ops,
                font,
                line,
                margin + 1.0,
                first_baseline - user_info.line_height * line_index as f32,
                user_info.font_size,
            );
        }
        for (day_index, seconds) in row.daily_seconds.iter().enumerate() {
            add_pdf_text(
                ops,
                font,
                format_cell_duration(*seconds),
                x_positions[day_index + 1] + 0.5,
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

pub fn to_pdf(export: &MonthlyExport, identity_mode: IdentityMode) -> anyhow::Result<Vec<u8>> {
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
        add_pdf_table(&mut ops, export, rows, &font, 185.0, identity_mode);
        pages.push(PdfPage::new(Mm(297.0), Mm(210.0), ops));
    }

    let mut warnings = Vec::new();
    let options = PdfSaveOptions {
        subset_fonts: true,
        ..PdfSaveOptions::default()
    };
    let bytes = document.with_pages(pages).save(&options, &mut warnings);
    for warning in warnings {
        tracing::warn!(?warning, "PDF generation warning");
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_long_userconfig_identity_without_losing_generation_name_or_role() {
        let real_name =
            "非常に長い本名を持つ帳票確認用ユーザー山田太郎第二確認用氏名文字列佐藤花子";
        let role = "出席管理システム運用責任者兼会計監査担当";
        let row = ExportRow {
            user_id: 9_876_543_210,
            display_name: "Discord表示名".into(),
            generation: Some(17),
            real_name: Some(real_name.into()),
            role: Some(role.into()),
            name_reading: Some("やまだたろう".into()),
            daily_seconds: Vec::new(),
            total_seconds: 0,
        };

        let layout = layout_user_info(&row, IdentityMode::WithDiscordName, 48.0, 8.0);
        assert!(layout.lines.len() > 3);
        assert!(layout.font_size >= 3.0);
        assert!(layout.line_height * layout.lines.len() as f32 <= 8.0 - 1.2);
        assert_eq!(
            layout.lines.concat(),
            format!("17代{real_name}Discord: Discord表示名（{role}）")
        );
        assert!(!layout.lines.iter().any(|line| line.contains('…')));
        assert!(!layout.lines.iter().any(|line| line.contains("9876543210")));

        let max_width_pt = (48.0 - 2.0) * (72.0 / 25.4);
        for line in layout.lines {
            let width_pt = line.chars().map(pdf_character_width_em).sum::<f32>() * layout.font_size;
            assert!(width_pt <= max_width_pt + f32::EPSILON);
        }
    }

    #[test]
    fn real_name_only_pdf_identity_excludes_discord_name() {
        let row = ExportRow {
            user_id: 1,
            display_name: "Discord表示名".into(),
            generation: None,
            real_name: None,
            role: None,
            name_reading: None,
            daily_seconds: Vec::new(),
            total_seconds: 0,
        };

        assert_eq!(
            user_info_parts(&row, IdentityMode::RealNameOnly),
            vec!["未設定"]
        );
    }
}
