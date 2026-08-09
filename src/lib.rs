pub mod attendance;
pub mod channel_status;
pub mod commands;
pub mod config;
pub mod presentation;
pub mod repository;
pub mod time;

use sqlx::SqlitePool;

pub struct Data {
    pub database: SqlitePool,
    pub status_channel_id: u64,
}

pub type Error = anyhow::Error;
pub type Context<'a> = poise::Context<'a, Data, Error>;
