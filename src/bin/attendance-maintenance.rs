#![deny(unsafe_code)]

use std::{env, path::Path};

use anyhow::bail;
use discord_attendance_bot::maintenance;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = env::args().collect::<Vec<_>>();
    match args.as_slice() {
        [_, command, source, destination] if command == "backup" => {
            maintenance::backup_database(Path::new(source), Path::new(destination)).await?;
            println!("backup created and verified: {destination}");
        }
        [_, command, database] if command == "verify" => {
            maintenance::verify_database(Path::new(database)).await?;
            println!("database integrity verified: {database}");
        }
        [_, command, backup, destination] if command == "restore" => {
            maintenance::restore_database(Path::new(backup), Path::new(destination)).await?;
            println!("backup restored and verified: {destination}");
        }
        _ => bail!(
            "usage:\n  attendance-maintenance backup <source.db> <destination.db>\n  attendance-maintenance verify <database.db>\n  attendance-maintenance restore <backup.db> <new-database.db>"
        ),
    }
    Ok(())
}
