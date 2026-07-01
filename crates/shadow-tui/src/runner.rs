//! 主循环 -- 终端初始化/还原 + 事件分发 + 绘制

use anyhow::Result;
use crossterm::event::{self, Event, KeyEvent};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal as RatTerm;
use std::io::{self, Stdout};
use std::time::Duration;

use crate::app::AppState;
use crate::event::AppEvent;

pub type Frame = RatTerm<CrosstermBackend<Stdout>>;

/// 装好 panic hook: 崩溃前必须还原终端
fn install_panic_hook() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        prev(info);
    }));
}

/// 主循环 -- 接收 mpsc rx, 处理事件, 绘制
pub async fn run_loop(
    mut state: AppState,
    mut rx: tokio::sync::mpsc::Receiver<AppEvent>,
) -> Result<AppState> {
    install_panic_hook();
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut term = RatTerm::new(backend)?;

    let result = run_loop_inner(&mut term, &mut state, &mut rx).await;

    // 无论结果如何, 还原终端
    let _ = disable_raw_mode();
    let _ = execute!(io::stdout(), LeaveAlternateScreen);
    result?;

    Ok(state)
}

async fn run_loop_inner(
    term: &mut Frame,
    state: &mut AppState,
    rx: &mut tokio::sync::mpsc::Receiver<AppEvent>,
) -> Result<()> {
    while state.running {
        draw(term, state)?;

        // 优先 mpsc, 200ms 超时后退到 crossterm 轮询
        match rx.recv().await {
            Some(ev) => handle_event(state, ev)?,
            None => break, // 所有 sender 关闭
        }

        // 非阻塞拉 crossterm 输入 (避免阻塞 mpsc)
        while event::poll(Duration::from_millis(0))? {
            if let Event::Key(k) = event::read()? {
                handle_event(state, AppEvent::Key(k))?;
            }
        }
    }
    Ok(())
}

fn draw(term: &mut Frame, state: &AppState) -> Result<()> {
    let _ = term.draw(|f| {
        let area = f.area();
        // 占位: 实际布局各 Task 16 联动
        let _ = state;
        let _ = area;
    })?;
    Ok(())
}

fn handle_event(state: &mut AppState, ev: AppEvent) -> Result<()> {
    match ev {
        AppEvent::Key(k) => return handle_key(state, k),
        AppEvent::Status(s) => state.status_top.text = s,
        AppEvent::AgentMessage(msg) => {
            state.chat.messages.push(shadow_core::ChatMessage {
                role: "assistant".into(),
                content: msg,
                tool_call_id: None,
                tool_calls: vec![],
            });
        }
        AppEvent::AgentToolCall {
            name,
            success,
            output_preview,
            duration_ms: _,
        } => {
            let content = format!("{}\n{}", name, output_preview);
            state.chat.messages.push(shadow_core::ChatMessage {
                role: "tool".into(),
                content,
                tool_call_id: None,
                tool_calls: vec![],
            });
            if !success {
                state.last_error = Some(format!("tool {name} failed"));
            }
        }
        AppEvent::AgentDone => {
            state.chat.agent_busy = false;
        }
        AppEvent::AgentError(e) => {
            state.last_error = Some(e.clone());
            state.chat.messages.push(shadow_core::ChatMessage {
                role: "assistant".into(),
                content: format!("[错误] {e}"),
                tool_call_id: None,
                tool_calls: vec![],
            });
            state.chat.agent_busy = false;
        }
        AppEvent::MemoryLoaded(entries) => {
            state.memory_view.entries = entries;
            state.memory_view.loading = false;
        }
    }
    Ok(())
}

fn handle_key(state: &mut AppState, _k: KeyEvent) -> Result<()> {
    // 实际按键处理在 Task 16 联动时补全
    let _ = state;
    Ok(())
}
