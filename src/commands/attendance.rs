mod common;
mod daily;
mod guarded;
mod privacy;
mod query;

use crate::{Context, Error};

pub use daily::{continue_activity, end, exit, join, start};
pub use guarded::{confirm, delete, edit, revert};
pub use privacy::erase;
pub use query::{help, history, list, month, status};

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
        "list",
        "history",
        "month",
        "edit",
        "delete",
        "erase",
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
        assert_eq!(command.subcommands.len(), 13);
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

    #[test]
    fn short_commands_match_start_and_end_parameters() {
        let start = start();
        let join = join();
        let end = end();
        let exit = exit();

        assert_eq!(join.name, "join");
        assert_eq!(exit.name, "exit");
        assert_eq!(join.parameters.len(), start.parameters.len());
        assert_eq!(exit.parameters.len(), end.parameters.len());
        assert_eq!(
            join.parameters
                .iter()
                .map(|parameter| parameter.name.as_str())
                .collect::<Vec<_>>(),
            start
                .parameters
                .iter()
                .map(|parameter| parameter.name.as_str())
                .collect::<Vec<_>>()
        );
        assert_eq!(
            exit.parameters
                .iter()
                .map(|parameter| parameter.name.as_str())
                .collect::<Vec<_>>(),
            end.parameters
                .iter()
                .map(|parameter| parameter.name.as_str())
                .collect::<Vec<_>>()
        );
    }
}
