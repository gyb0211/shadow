//! Terminal 抽象 -- 测试可注入假终端
//!
//! 真终端: crossterm + ratatui
//! 测试:   FakeTerminal 收集 draw 输出, 模拟键入

use anyhow::Result;
use crossterm::event::{KeyEvent, KeyEventKind, KeyCode, KeyModifiers};
use std::collections::VecDeque;
use std::time::Duration;

/// 终端抽象
pub trait Terminal {
    /// 渲染一帧 (具体 widget 由调用方用 ratatui::Terminal 在内部画)
    fn draw_raw(&mut self, render_fn: &dyn Fn(&mut dyn std::io::Write)) -> Result<()>;
    /// 取下一条事件, timeout 内没有则 None
    fn poll_event(&mut self, timeout: Duration) -> Result<Option<crate::event::AppEvent>>;
}

/// 假终端 -- 测试用
pub struct FakeTerminal {
    pub outputs: Vec<String>,
    pub inputs: VecDeque<crate::event::AppEvent>,
}

impl FakeTerminal {
    pub fn new() -> Self {
        Self { outputs: Vec::new(), inputs: VecDeque::new() }
    }

    /// 队列模拟键入 (字符串 → 多个 Char KeyEvent)
    pub fn input_keys(&mut self, s: &str) {
        for c in s.chars() {
            let code = if c == '\n' {
                KeyCode::Enter
            } else {
                KeyCode::Char(c)
            };
            self.inputs.push_back(crate::event::AppEvent::Key(KeyEvent {
                code,
                modifiers: KeyModifiers::empty(),
                kind: KeyEventKind::Press,
                state: crossterm::event::KeyEventState::NONE,
            }));
        }
    }

    /// 队列任意事件
    pub fn push_event(&mut self, ev: crate::event::AppEvent) {
        self.inputs.push_back(ev);
    }

    /// 把所有已渲染输出拼成一个字符串
    pub fn snapshot(&self) -> String {
        self.outputs.concat()
    }
}

impl Default for FakeTerminal {
    fn default() -> Self {
        Self::new()
    }
}

impl Terminal for FakeTerminal {
    fn draw_raw(&mut self, render_fn: &dyn Fn(&mut dyn std::io::Write)) -> Result<()> {
        let mut buf: Vec<u8> = Vec::new();
        render_fn(&mut buf);
        self.outputs.push(String::from_utf8_lossy(&buf).into_owned());
        Ok(())
    }

    fn poll_event(&mut self, _timeout: Duration) -> Result<Option<crate::event::AppEvent>> {
        Ok(self.inputs.pop_front())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fake_terminal_records_draw_output() {
        let mut term = FakeTerminal::new();
        term.draw_raw(&|w| { use std::io::Write; let _ = write!(w, "hello"); }).unwrap();
        assert_eq!(term.snapshot(), "hello");
    }

    #[test]
    fn fake_terminal_inputs_queue_fifo() {
        let mut term = FakeTerminal::new();
        term.input_keys("a");
        let ev = term.poll_event(Duration::ZERO).unwrap().unwrap();
        if let crate::event::AppEvent::Key(k) = ev {
            assert_eq!(k.code, KeyCode::Char('a'));
        } else {
            panic!("expected Key event");
        }
    }
}
