mod common;
mod export;
mod query;
use crate::{Context, Error};
use export::export;
use query::{help, history, list, month, status};
/// 記録済みの活動時間を閲覧・集計・出力する。
#[poise::command(
    slash_command,
    subcommands("status", "list", "history", "month", "export", "help"),
    subcommand_required
)]
pub async fn attendanceview(_: Context<'_>) -> Result<(), Error> {
    Ok(())
}
