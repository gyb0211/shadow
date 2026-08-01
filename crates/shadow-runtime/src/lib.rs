//! 影子运行时 -- agent loop + 工具集
//!
//! 配置已迁移到 shadow-config crate

pub mod agent;
pub mod cron;
pub mod daemon;
pub mod dispatcher;
mod observability;
pub mod prompt;
pub mod security;
pub mod service;
pub mod skills;
pub mod tools;
