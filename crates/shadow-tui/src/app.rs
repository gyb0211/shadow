//! AppState -- 中心状态机, 纯逻辑无 IO

use crate::event::AppEvent;
use shadow_core::{ChatMessage, MemoryEntry};
use std::sync::Arc;
use tokio::sync::mpsc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum View { Chat, Config, Memory }

#[derive(Debug, Clone, Default)]
pub struct ChatState {
    pub messages: Vec<ChatMessage>,
    pub input: String,
    pub input_history: Vec<String>,
    pub scroll_offset: usize,
    pub agent_busy: bool,
}

#[derive(Debug, Clone, Default)]
pub struct ConfigViewState {
    pub loaded: bool,
}

#[derive(Debug, Clone, Default)]
pub struct MemoryViewState {
    pub entries: Vec<MemoryEntry>,
    pub query: String,
    pub loading: bool,
}

#[derive(Debug, Clone, Default)]
pub struct PaletteState {
    pub query: String,
    pub selected: usize,
}

#[derive(Debug, Clone, Default)]
pub struct StatusLine {
    pub text: String,
}

/// 预定义命令
pub const COMMANDS: &[&str] = &[
    "chat", "config", "memory",
    "clear history", "quit",
];

#[derive(Clone)]
pub struct AppState {
    pub view: View,
    pub palette: Option<PaletteState>,
    pub chat: ChatState,
    pub config_view: ConfigViewState,
    pub memory_view: MemoryViewState,
    pub status_top: StatusLine,
    pub status_bottom: StatusLine,
    pub running: bool,
    pub last_error: Option<String>,
    /// 后台 Agent (TUI 启动时注入; None = 无 agent, Enter 不触发对话)
    pub agent: Option<Arc<shadow_runtime::agent::Agent>>,
    /// mpsc 发送端 (用于向主循环推送 Agent 事件)
    pub tx: Option<mpsc::Sender<AppEvent>>,
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState")
            .field("view", &self.view)
            .field("palette", &self.palette)
            .field("chat", &self.chat)
            .field("running", &self.running)
            .field("last_error", &self.last_error)
            .finish()
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            view: View::Chat,
            palette: None,
            chat: ChatState::default(),
            config_view: ConfigViewState::default(),
            memory_view: MemoryViewState::default(),
            status_top: StatusLine { text: "shadow".to_string() },
            status_bottom: StatusLine { text: "↵ send · ⌥↵ newline · ⌘K palette · /help".to_string() },
            running: true,
            last_error: None,
            agent: None,
            tx: None,
        }
    }
}

impl AppState {
    pub fn new() -> Self { Self::default() }

    /// 打开命令面板
    pub fn open_palette(&mut self) {
        self.palette = Some(PaletteState::default());
    }

    /// 关闭命令面板
    pub fn close_palette(&mut self) {
        self.palette = None;
    }

    /// 命令面板过滤后的候选项
    pub fn palette_items(&self) -> Vec<&'static str> {
        let q = self.palette.as_ref().map(|p| p.query.as_str()).unwrap_or("");
        COMMANDS.iter()
            .filter(|c| c.contains(q) || q.is_empty())
            .copied()
            .collect()
    }

    /// 更新 palette 搜索词
    pub fn update_palette_query(&mut self, q: &str) {
        if let Some(p) = self.palette.as_mut() {
            p.query = q.to_string();
            p.selected = 0;
        }
    }

    /// 执行当前选中的 palette 项; 返回 true 表示已处理
    pub fn execute_palette(&mut self) -> bool {
        let items = self.palette_items();
        let sel = self.palette.as_ref().map(|p| p.selected).unwrap_or(0);
        let Some(cmd) = items.get(sel).copied() else { return false; };
        self.close_palette();
        self.dispatch_command(cmd)
    }

    /// 执行 slash 命令或 palette 命令
    pub fn dispatch_command(&mut self, cmd: &str) -> bool {
        match cmd.trim() {
            "chat" => { self.view = View::Chat; true }
            "config" => { self.view = View::Config; true }
            "memory" => { self.view = View::Memory; true }
            "clear history" => { self.chat.messages.clear(); self.chat.input_history.clear(); true }
            "quit" | "exit" => { self.running = false; true }
            _ => false,
        }
    }

    /// 把输入框当作 slash 命令执行; 返回 true 表示已消费
    pub fn try_slash_input(&mut self) -> bool {
        let trimmed = self.chat.input.trim().to_string();
        if let Some(cmd) = trimmed.strip_prefix('/') {
            let consumed = self.dispatch_command(cmd);
            if consumed {
                self.chat.input.clear();
            }
            return consumed;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_filter_matches_substring() {
        let mut s = AppState::new();
        s.open_palette();
        s.update_palette_query("con");
        let names: Vec<_> = s.palette_items();
        assert!(names.contains(&"config"));
        assert!(!names.contains(&"chat"));
    }

    #[test]
    fn palette_empty_query_shows_all() {
        let mut s = AppState::new();
        s.open_palette();
        assert_eq!(s.palette_items().len(), COMMANDS.len());
    }

    #[test]
    fn slash_clear_truncates_messages() {
        let mut s = AppState::new();
        s.chat.messages.push(ChatMessage {
            role: "user".into(), content: "hi".into(),
            tool_call_id: None, tool_calls: vec![],
        });
        s.chat.input = "/clear history".to_string();
        assert!(s.try_slash_input());
        assert!(s.chat.messages.is_empty());
        assert!(s.chat.input.is_empty());
    }

    #[test]
    fn slash_unknown_returns_false() {
        let mut s = AppState::new();
        s.chat.input = "/foobar".to_string();
        assert!(!s.try_slash_input());
        assert_eq!(s.chat.input, "/foobar"); // 未消费
    }

    #[test]
    fn dispatch_quit_sets_running_false() {
        let mut s = AppState::new();
        assert!(s.running);
        s.dispatch_command("quit");
        assert!(!s.running);
    }

    #[test]
    fn palette_execute_selects_chat() {
        let mut s = AppState::new();
        s.view = View::Memory;
        s.open_palette();
        s.update_palette_query("chat");
        s.execute_palette();
        assert_eq!(s.view, View::Chat);
        assert!(s.palette.is_none());
    }
}
