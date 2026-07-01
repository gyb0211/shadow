//! shadow TUI -- ratatui dashboard

pub mod app;
pub mod event;
pub mod terminal;
pub mod theme;
pub mod widgets;

pub fn run_tui(_config: shadow_config::Config) -> anyhow::Result<()> {
    // 实际实现在 Task 16
    Ok(())
}
