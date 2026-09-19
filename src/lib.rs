#![deny(unsafe_code)]

pub mod attendance;
pub mod commands;
pub mod framework_error;
pub mod maintenance;
pub mod presentation;
pub mod repository;

use sqlx::SqlitePool;

pub use attendance_shared::{config, database, text, time};
pub mod auto_end;

pub struct Data {
    pub database: SqlitePool,
}

pub type Error = anyhow::Error;
pub type Context<'a> = poise::Context<'a, Data, Error>;
