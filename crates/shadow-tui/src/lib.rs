//! shadow TUI -- ratatui dashboard

pub mod app;
pub mod event;
pub mod observer;
pub mod runner;
pub mod terminal;
pub mod theme;
pub mod views;
pub mod widgets;

pub use app::AppState;
pub use event::AppEvent;
pub use observer::UiObserver;
pub use runner::run_loop;

use anyhow::Result;
use shadow_config::Config;
use tokio::sync::mpsc;

/// 启动 TUI. 默认进入 Chat view.
pub async fn run_tui(config: Config) -> Result<()> {
    let (tx, rx) = mpsc::channel::<AppEvent>(256);
    let _observer = UiObserver::arc(tx); // 注: 实际在 Task 17 联动时交给 AgentBuilder
    let state = AppState::new();
    let _final = run_loop(state, rx).await?;
    let _ = config;
    Ok(())
}
