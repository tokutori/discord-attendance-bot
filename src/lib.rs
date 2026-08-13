#![deny(unsafe_code)]

pub mod attendance;
pub mod attendance_export;
pub mod channel_status;
pub mod commands;
pub mod config;
pub mod framework_error;
pub mod member_order;
pub mod presentation;
pub mod repository;
pub mod text;
pub mod time;

use sqlx::SqlitePool;

pub struct Data {
    pub database: SqlitePool,
}

pub type Error = anyhow::Error;
pub type Context<'a> = poise::Context<'a, Data, Error>;
