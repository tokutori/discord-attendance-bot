pub mod attendance;
pub mod commands;
pub mod presentation;
pub mod repository;
pub mod time;

use sqlx::SqlitePool;

pub struct Data {
    pub database: SqlitePool,
}

pub type Error = anyhow::Error;
pub type Context<'a> = poise::Context<'a, Data, Error>;
