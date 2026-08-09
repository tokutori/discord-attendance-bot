mod common;
mod daily;
mod guarded;
mod query;

use crate::{Context, Error};

pub use daily::{continue_activity, end, start};
pub use guarded::{confirm, delete, edit, revert};
pub use query::{help, history, month, status};

#[poise::command(
    slash_command,
    subcommands(
        "start",
        "end",
        "continue_activity",
        "revert",
        "confirm",
        "status",
        "history",
        "month",
        "edit",
        "delete",
        "help"
    ),
    subcommand_required
)]
pub async fn attendance(_: Context<'_>) -> Result<(), Error> {
    Ok(())
}
