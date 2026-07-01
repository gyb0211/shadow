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

        // 优先 mpsc, 200ms 超时后退到 crossterm 轮询 (否则无 agent 时会永久阻塞)
        match tokio::time::timeout(Duration::from_millis(200), rx.recv()).await {
            Ok(Some(ev)) => handle_event(state, ev)?,
            Ok(None) => break, // 所有 sender 关闭
            Err(_) => {} // 超时, 继续下面的 crossterm 轮询
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
    use ratatui::layout::{Constraint, Direction, Layout};
    use crate::views::{ChatView, ConfigView, MemoryView};
    use crate::widgets::{CommandPalette, StatusBar};

    term.draw(|f| {
        let area = f.area();

        // 整屏分: 顶 status / 中 view / 底 status
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Min(1),
                Constraint::Length(1),
            ])
            .split(area);

        // 顶
        f.render_widget(
            StatusBar::new(&state.status_top.text, "⌘K"),
            chunks[0],
        );

        // 中
        match state.view {
            crate::app::View::Chat => {
                f.render_widget(ChatView::new(&state.chat), chunks[1]);
            }
            crate::app::View::Config => {
                f.render_widget(ConfigView::new(&shadow_config::Config::default(), 0), chunks[1]);
                // 注: 实际应传持久化的 config 引用, 这里简化 (Task 17 集成时改)
            }
            crate::app::View::Memory => {
                f.render_widget(MemoryView::new(&state.memory_view), chunks[1]);
            }
        }

        // 底
        let busy_label = if state.chat.agent_busy { "⏳ " } else { "" };
        let right = format!("{}hist {}/{}", busy_label, state.chat.messages.len(), 50);
        f.render_widget(
            StatusBar::new(&state.status_bottom.text, &right),
            chunks[2],
        );

        // palette 浮层
        if let Some(p) = &state.palette {
            let items = state.palette_items();
            f.render_widget(
                CommandPalette::new(&p.query, &items, p.selected),
                chunks[1],
            );
        }
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
                tool_calls: vec![], reasoning_content: None,
            });
            state.chat.scroll_offset = 0;
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
                tool_calls: vec![], reasoning_content: None,
            });
            state.chat.scroll_offset = 0;
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
                tool_calls: vec![], reasoning_content: None,
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

/// 在 cursor 位置插入字符
fn insert_char(input: &mut String, cursor: &mut usize, c: char) {
    let chars: Vec<char> = input.chars().collect();
    let pos = (*cursor).min(chars.len());
    let mut new_input: String = chars[..pos].iter().collect();
    new_input.push(c);
    new_input.extend(chars[pos..].iter());
    *input = new_input;
    *cursor += 1;
}

fn handle_key(state: &mut AppState, k: KeyEvent) -> Result<()> {
    use crossterm::event::KeyCode::*;

    // palette 模式
    if state.palette.is_some() {
        match k.code {
            Esc => state.close_palette(),
            Enter => { state.execute_palette(); }
            Up => {
                if let Some(p) = state.palette.as_mut() {
                    if p.selected > 0 { p.selected -= 1; }
                }
            }
            Down => {
                let max = state.palette_items().len().saturating_sub(1);
                if let Some(p) = state.palette.as_mut() {
                    p.selected = (p.selected + 1).min(max);
                }
            }
            Backspace => {
                if let Some(p) = state.palette.as_mut() {
                    p.query.pop();
                }
            }
            Char(c) => {
                let q = state.palette.as_ref().map(|p| p.query.clone()).unwrap_or_default();
                let mut new_q = q;
                new_q.push(c);
                state.update_palette_query(&new_q);
            }
            _ => {}
        }
        return Ok(());
    }

    // 普通模式
    match k.code {
        Char('k') if k.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) => {
            state.open_palette();
        }
        Char('c') if k.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) => {
            state.running = false;
        }
        Esc => { /* 退出当前 view? 暂不处理 */ }
        PageUp => {
            state.chat.scroll_offset = state.chat.scroll_offset.saturating_add(5);
        }
        PageDown => {
            state.chat.scroll_offset = state.chat.scroll_offset.saturating_sub(5);
        }
        Enter => {
            // Alt+Enter = 插入换行 (多行输入)
            if k.modifiers.contains(crossterm::event::KeyModifiers::ALT) {
                insert_char(&mut state.chat.input, &mut state.chat.cursor, '\n');
                return Ok(());
            }
            if !state.chat.agent_busy {
                if !state.try_slash_input() {
                    let text = state.chat.input.clone();
                    if !text.trim().is_empty() {
                        state.chat.input_history.push(text.clone());
                        state.chat.messages.push(shadow_core::ChatMessage {
                            role: "user".into(), content: text.clone(),
                            tool_call_id: None, tool_calls: vec![], reasoning_content: None,
                        });
                        state.chat.input.clear();
                        state.chat.cursor = 0;
                        state.chat.agent_busy = true;
                        state.chat.scroll_offset = 0;
                        // spawn 后台 task 调用 agent.chat(); UiObserver 已在 agent 内部,
                        // 会通过 mpsc 推送 Status/ToolCall/Error 事件, 完成后再推 AgentMessage + AgentDone
                        if let (Some(agent), Some(tx)) = (state.agent.clone(), state.tx.clone()) {
                            tokio::spawn(async move {
                                match agent.chat(&text).await {
                                    Ok(resp) => {
                                        let _ = tx.send(AppEvent::AgentMessage(resp)).await;
                                    }
                                    Err(e) => {
                                        let _ = tx.send(AppEvent::AgentError(e.to_string())).await;
                                    }
                                }
                                let _ = tx.send(AppEvent::AgentDone).await;
                            });
                        } else {
                            // 无 agent: 立即解锁, 避免卡死
                            state.chat.agent_busy = false;
                            state.last_error = Some("未配置 agent".into());
                        }
                    }
                }
            }
        }
        Backspace => {
            let chars: Vec<char> = state.chat.input.chars().collect();
            if state.chat.cursor > 0 && state.chat.cursor <= chars.len() {
                let pos = state.chat.cursor - 1;
                let mut new_input: String = chars[..pos].iter().collect();
                new_input.extend(chars[state.chat.cursor..].iter());
                state.chat.input = new_input;
                state.chat.cursor -= 1;
            }
        }
        Left => { state.chat.cursor = state.chat.cursor.saturating_sub(1); }
        Right => {
            let max = state.chat.input.chars().count();
            state.chat.cursor = (state.chat.cursor + 1).min(max);
        }
        Home => { state.chat.cursor = 0; }
        End => { state.chat.cursor = state.chat.input.chars().count(); }
        Char(c) => {
            insert_char(&mut state.chat.input, &mut state.chat.cursor, c);
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, KeyEventKind, KeyEventState};

    fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent {
            code, modifiers: mods, kind: KeyEventKind::Press, state: KeyEventState::NONE,
        }
    }

    #[test]
    fn ctrl_k_opens_palette() {
        let mut s = AppState::new();
        assert!(s.palette.is_none());
        handle_key(&mut s, key(KeyCode::Char('k'), KeyModifiers::CONTROL)).unwrap();
        assert!(s.palette.is_some());
    }

    #[test]
    fn ctrl_c_quits() {
        let mut s = AppState::new();
        handle_key(&mut s, key(KeyCode::Char('c'), KeyModifiers::CONTROL)).unwrap();
        assert!(!s.running);
    }

    #[test]
    fn enter_pushes_user_message_when_not_busy() {
        let mut s = AppState::new();
        s.chat.input = "hello".to_string();
        handle_key(&mut s, key(KeyCode::Enter, KeyModifiers::empty())).unwrap();
        assert_eq!(s.chat.messages.len(), 1);
        assert_eq!(s.chat.messages[0].role, "user");
        assert!(s.chat.input.is_empty());
        // 无 agent 注入时, agent_busy 立即重置 (避免卡死), 且 last_error 标记原因
        assert!(!s.chat.agent_busy);
        assert!(s.last_error.as_deref().unwrap_or("").contains("未配置"));
    }

    #[test]
    fn enter_ignored_when_busy() {
        let mut s = AppState::new();
        s.chat.input = "hello".to_string();
        s.chat.agent_busy = true;
        handle_key(&mut s, key(KeyCode::Enter, KeyModifiers::empty())).unwrap();
        assert_eq!(s.chat.messages.len(), 0);
        assert!(!s.chat.input.is_empty());
    }

    #[test]
    fn alt_enter_inserts_newline() {
        let mut s = AppState::new();
        s.chat.input = "hello".to_string();
        s.chat.cursor = 5;
        handle_key(&mut s, key(KeyCode::Enter, KeyModifiers::ALT)).unwrap();
        assert_eq!(s.chat.input, "hello\n");
        assert_eq!(s.chat.cursor, 6);
        assert_eq!(s.chat.messages.len(), 0); // 不发送
    }

    #[test]
    fn left_right_moves_cursor() {
        let mut s = AppState::new();
        s.chat.input = "abc".to_string();
        s.chat.cursor = 3;
        handle_key(&mut s, key(KeyCode::Left, KeyModifiers::empty())).unwrap();
        assert_eq!(s.chat.cursor, 2);
        handle_key(&mut s, key(KeyCode::Left, KeyModifiers::empty())).unwrap();
        assert_eq!(s.chat.cursor, 1);
        handle_key(&mut s, key(KeyCode::Right, KeyModifiers::empty())).unwrap();
        assert_eq!(s.chat.cursor, 2);
    }

    #[test]
    fn home_end_jump_cursor() {
        let mut s = AppState::new();
        s.chat.input = "abc".to_string();
        s.chat.cursor = 2;
        handle_key(&mut s, key(KeyCode::Home, KeyModifiers::empty())).unwrap();
        assert_eq!(s.chat.cursor, 0);
        handle_key(&mut s, key(KeyCode::End, KeyModifiers::empty())).unwrap();
        assert_eq!(s.chat.cursor, 3);
    }

    #[test]
    fn backspace_at_cursor_position() {
        let mut s = AppState::new();
        s.chat.input = "abc".to_string();
        s.chat.cursor = 1; // 在 a|bc
        handle_key(&mut s, key(KeyCode::Backspace, KeyModifiers::empty())).unwrap();
        assert_eq!(s.chat.input, "bc");
        assert_eq!(s.chat.cursor, 0);
    }

    #[test]
    fn char_inserted_at_cursor() {
        let mut s = AppState::new();
        s.chat.input = "ac".to_string();
        s.chat.cursor = 1; // a|c
        handle_key(&mut s, key(KeyCode::Char('b'), KeyModifiers::empty())).unwrap();
        assert_eq!(s.chat.input, "abc");
        assert_eq!(s.chat.cursor, 2);
    }
}
