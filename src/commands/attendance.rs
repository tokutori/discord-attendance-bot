mod common;
mod daily;
mod guarded;
mod query;

use crate::{Context, Error};

pub use daily::{continue_activity, end, start};
pub use guarded::{confirm, delete, edit, revert};
pub use query::{help, history, month, status};

/// 活動時間の記録・確認・修正を行う。
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_attendance_commands_have_japanese_descriptions() {
        let command = attendance();
        assert!(
            command
                .description
                .as_deref()
                .is_some_and(|value| value != "A slash command")
        );
        assert_eq!(command.subcommands.len(), 11);
        for subcommand in command.subcommands {
            assert!(
                subcommand
                    .description
                    .as_deref()
                    .is_some_and(|value| value != "A slash command"),
                "missing description for {}",
                subcommand.name
            );
        }
    }
}
