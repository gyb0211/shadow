//! shadow TUI -- ratatui dashboard

pub mod app;
pub mod event;
pub mod observer;
pub mod runner;
pub mod terminal;
pub mod theme;
pub mod views;
pub mod widgets;

pub fn run_tui(_config: shadow_config::Config) -> anyhow::Result<()> {
    Ok(())
}
