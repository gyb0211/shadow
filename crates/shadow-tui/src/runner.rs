//! 主循环 -- 终端初始化/还原 + 事件分发 + 绘制
//!
//! 架构: 独立 OS 线程阻塞读 crossterm → unbounded channel → tokio select!
//! 零输入延迟 (不轮询, 不 timeout).

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
use tokio::sync::mpsc;

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
    mut rx: mpsc::Receiver<AppEvent>,
) -> Result<AppState> {
    install_panic_hook();
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut term = RatTerm::new(backend)?;

    // ── 独立 OS 线程阻塞读 crossterm 事件 → unbounded channel ──
    let (input_tx, mut input_rx) = mpsc::unbounded_channel::<Event>();
    std::thread::spawn(move || {
        loop {
            // poll 短超时, 让线程有机会检查是否应退出
            match event::poll(Duration::from_millis(100)) {
                Ok(true) => {
                    if let Ok(ev) = event::read() {
                        if input_tx.send(ev).is_err() {
                            break; // channel 关闭, 退出
                        }
                    }
                }
                Ok(false) => {} // 超时, 继续
                Err(_) => break,
            }
        }
    });

    let result = run_loop_inner(&mut term, &mut state, &mut rx, &mut input_rx).await;

    // 无论结果如何, 还原终端
    let _ = disable_raw_mode();
    let _ = execute!(io::stdout(), LeaveAlternateScreen);
    result?;

    Ok(state)
}

async fn run_loop_inner(
    term: &mut Frame,
    state: &mut AppState,
    rx: &mut mpsc::Receiver<AppEvent>,
    input_rx: &mut mpsc::UnboundedReceiver<Event>,
) -> Result<()> {
    let mut dirty = true; // 首帧必须画
    while state.running {
        if dirty {
            draw(term, state)?;
            dirty = false;
        }

        // ── select!: 同时等待 agent 事件和 crossterm 输入 ──
        tokio::select! {
            // agent / observer 事件
            ev = rx.recv() => match ev {
                Some(ev) => { handle_event(state, ev)?; dirty = true; }
                None => break,
            },
            // crossterm 键盘 / 鼠标事件 (零延迟)
            ev = input_rx.recv() => {
                if let Some(crossterm_ev) = ev {
                    match crossterm_ev {
                        Event::Key(k) => { handle_event(state, AppEvent::Key(k))?; dirty = true; }
                        Event::Mouse(m) => {
                            use crossterm::event::MouseEventKind;
                            match m.kind {
                            MouseEventKind::ScrollUp => {
                                state.chat.scroll_up(3);
                                dirty = true;
                            }
                            MouseEventKind::ScrollDown => {
                                state.chat.scroll_down(3);
                                dirty = true;
                            }
                                _ => {} // 忽略移动/点击
                            }
                        }
                        Event::Resize(_, _) => { dirty = true; } // 窗口大小变化需要重绘
                        _ => {}
                    }
                }
            }
        }
    }
    Ok(())
}

fn draw(term: &mut Frame, state: &AppState) -> Result<()> {
    use ratatui::layout::{Constraint, Direction, Layout};
    use ratatui::widgets::{Block, Borders};
    use ratatui::style::Style;
    use crate::views::{ChatView, ConfigView, MemoryView};
    use crate::widgets::{CommandPalette, InputBox, StatusBar};
    use crate::theme;

    term.draw(|f| {
        let area = f.area();

        // 统一背景色: 整屏先填 BG
        f.render_widget(
            Block::default().style(Style::default().bg(theme::bg())),
            area,
        );

        // 三段布局: main(可变高, 滚动) + input(动态高 1-3 + 顶边框 1) + status(固定 2)
        // 参考 ZeroClaw input_bar.rs: 动态计算 input_height = visible_rows + border
        let input_content_h = state.chat.input_height(); // 1-3
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(1),                  // main: 消息列表, 占满剩余空间
                Constraint::Length(input_content_h + 1), // input: 内容行 + 顶边框 1 行
                Constraint::Length(2),               // status: 固定 2 行
            ])
            .split(area);

        // ── main: 视图内容 (可滚动) ──
        match state.view {
            crate::app::View::Chat => {
                f.render_widget(ChatView::new(&state.chat), chunks[0]);
            }
            crate::app::View::Config => {
                f.render_widget(ConfigView::new(&shadow_config::Config::default(), 0), chunks[0]);
            }
            crate::app::View::Memory => {
                f.render_widget(MemoryView::new(&state.memory_view), chunks[0]);
            }
        }

        // ── input: 顶部分隔线 + 输入框 (无底边框, 紧接 status) ──
        let input_block = Block::default()
            .borders(Borders::TOP)
            .border_style(Style::default().fg(theme::dim()))
            .style(Style::default().bg(theme::bg()));
        let input_inner = input_block.inner(chunks[1]);
        f.render_widget(input_block, chunks[1]);
        f.render_widget(
            InputBox::new(&state.chat.input, state.chat.cursor),
            input_inner,
        );

        // ── status: 两行插件化状态栏 ──
        let status_data = state.status_data();
        f.render_widget(StatusBar::new(&status_data), chunks[2]);

        // palette 浮层 (覆盖 main 区域)
        if let Some(p) = &state.palette {
            let items = state.palette_items();
            f.render_widget(
                CommandPalette::new(&p.query, &items, p.selected),
                chunks[0],
            );
        }
    })?;
    Ok(())
}

fn handle_event(state: &mut AppState, ev: AppEvent) -> Result<()> {
    match ev {
        AppEvent::Key(k) => return handle_key(state, k),
        AppEvent::Status(s) => state.llm_status = Some(s),
        AppEvent::AgentMessage(msg) => {
            state.chat.messages.push(shadow_core::ChatMessage {
                role: "assistant".into(),
                content: msg,
                tool_call_id: None,
                tool_calls: vec![], reasoning_content: None,
            });
            // 只有钉在底部时才跟随最新消息 (参考 ZeroClaw pinned_to_bottom)
            if state.chat.pinned_to_bottom {
                state.chat.scroll_offset = 0;
            }
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
            if state.chat.pinned_to_bottom {
                state.chat.scroll_offset = 0;
            }
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
        AppEvent::Mouse(_) => {} // 鼠标事件在 run_loop_inner 内联处理
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
        Char('l') if k.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) => {
            state.chat.messages.clear();
            state.chat.scroll_offset = 0;
            state.chat.pinned_to_bottom = true;
            state.last_error = None;
        }
        Esc => { /* 退出当前 view? 暂不处理 */ }
        PageUp => {
            // 向上滚动 10 行 (参考 ZeroClaw scroll_up)
            state.chat.scroll_up(10);
        }
        PageDown => {
            // 向下滚动 10 行 (参考 ZeroClaw scroll_down)
            state.chat.scroll_down(10);
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
                        state.chat.pinned_to_bottom = true;
                        state.chat.history_browse = None;
                        state.chat.history_draft.clear();
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
        Up => {
            let hist = &state.chat.input_history;
            if !hist.is_empty() {
                match state.chat.history_browse {
                    None => {
                        state.chat.history_draft = state.chat.input.clone();
                        state.chat.history_browse = Some(hist.len() - 1);
                    }
                    Some(0) => {} // 已在最旧
                    Some(idx) => {
                        state.chat.history_browse = Some(idx - 1);
                    }
                }
                if let Some(idx) = state.chat.history_browse {
                    state.chat.input = state.chat.input_history[idx].clone();
                    state.chat.cursor = state.chat.input.chars().count();
                }
            }
        }
        Down => {
            let hist_len = state.chat.input_history.len();
            match state.chat.history_browse {
                None => {} // 不在浏览模式
                Some(idx) if idx + 1 >= hist_len => {
                    // 回到草稿
                    state.chat.input = state.chat.history_draft.clone();
                    state.chat.cursor = state.chat.input.chars().count();
                    state.chat.history_browse = None;
                    state.chat.history_draft.clear();
                }
                Some(idx) => {
                    state.chat.history_browse = Some(idx + 1);
                    state.chat.input = state.chat.input_history[idx + 1].clone();
                    state.chat.cursor = state.chat.input.chars().count();
                }
            }
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
    use shadow_core::ChatMessage;

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
    fn ctrl_l_clears_messages() {
        let mut s = AppState::new();
        s.chat.messages.push(ChatMessage {
            role: "user".into(), content: "hi".into(),
            tool_call_id: None, tool_calls: vec![], reasoning_content: None,
        });
        s.last_error = Some("err".into());
        handle_key(&mut s, key(KeyCode::Char('l'), KeyModifiers::CONTROL)).unwrap();
        assert!(s.chat.messages.is_empty());
        assert!(s.last_error.is_none());
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

    #[test]
    fn up_arrow_recalls_last_input() {
        let mut s = AppState::new();
        s.chat.input_history = vec!["hello".into(), "world".into()];
        s.chat.input = "draft".into();
        handle_key(&mut s, key(KeyCode::Up, KeyModifiers::empty())).unwrap();
        assert_eq!(s.chat.input, "world"); // 最后一条
        assert_eq!(s.chat.history_browse, Some(1));
        assert_eq!(s.chat.history_draft, "draft");
    }

    #[test]
    fn up_again_goes_older() {
        let mut s = AppState::new();
        s.chat.input_history = vec!["hello".into(), "world".into()];
        s.chat.history_browse = Some(1);
        s.chat.input = "world".into();
        handle_key(&mut s, key(KeyCode::Up, KeyModifiers::empty())).unwrap();
        assert_eq!(s.chat.input, "hello");
        assert_eq!(s.chat.history_browse, Some(0));
    }

    #[test]
    fn down_arrow_restores_draft() {
        let mut s = AppState::new();
        s.chat.input_history = vec!["hello".into()];
        s.chat.history_browse = Some(0);
        s.chat.history_draft = "my draft".into();
        s.chat.input = "hello".into();
        handle_key(&mut s, key(KeyCode::Down, KeyModifiers::empty())).unwrap();
        assert_eq!(s.chat.input, "my draft");
        assert_eq!(s.chat.history_browse, None);
    }
}
