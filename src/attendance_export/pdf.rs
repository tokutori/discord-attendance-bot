use std::{env, path::PathBuf};

use chrono::Datelike;
use printpdf::{
    Line, LinePoint, Mm, Op, ParsedFont, PdfDocument, PdfFontHandle, PdfPage, PdfSaveOptions,
    Point, Pt, TextItem,
};

use super::{ExportRow, MonthlyExport, format_cell_duration};

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
        add_pdf_text(
            ops,
            font,
            truncate_pdf_text(&row.pdf_identity(), 16),
            margin + 1.0,
            centered_text_y(row_top, row_height, 7.5),
            7.5,
        );
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
