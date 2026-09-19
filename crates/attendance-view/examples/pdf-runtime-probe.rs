//! Mandatory font/render smoke check in the real view runtime image. No Discord or dotenv.
use attendance_view::{
    attendance::AttendanceSession,
    attendance_export::{self, IdentityMode},
    time,
};
use chrono::{TimeZone, Utc};

fn main() -> anyhow::Result<()> {
    let month = time::parse_year_month("2026-08")?;
    let (start, _) = time::month_bounds(month)?;
    let session = AttendanceSession {
        id: 1,
        guild_id: 1,
        user_id: 1,
        display_name: "架空メンバー".into(),
        started_at: start,
        ended_at: Some(start + 3600),
        open_since: None,
        note: None,
        created_at: start,
        updated_at: start + 3600,
        deleted_at: None,
    };
    let export = attendance_export::build_monthly_export(
        month,
        &[session],
        &[],
        Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap(),
    )?;
    for mode in [IdentityMode::WithDiscordName, IdentityMode::RealNameOnly] {
        // No optional-font early return: the production view image must render.
        let pdf = attendance_export::to_pdf(&export, mode)?;
        anyhow::ensure!(
            pdf.starts_with(b"%PDF-") && pdf.windows(5).any(|b| b == b"%%EOF"),
            "invalid PDF envelope"
        );
        println!("PDF runtime generation passed: {} bytes", pdf.len());
    }
    Ok(())
}
