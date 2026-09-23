#![deny(unsafe_code)]
pub use attendance_query as repository;
pub use attendance_shared::{attendance, config, member_order, text, time};
pub mod attendance_export;
pub mod channel_status;
pub mod commands;
pub mod framework_error;
pub mod presentation;
pub struct Data {
    pub guild_id: u64,
    pub database: attendance_query::ReadDatabase,
}
pub type Error = anyhow::Error;
pub type Context<'a> = poise::Context<'a, Data, Error>;
