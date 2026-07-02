//! AppState -- 中心状态机, 纯逻辑无 IO

use crate::event::AppEvent;
use crate::widgets::status_bar::{StatusBarData, StatusSegment};
use crate::theme;
use shadow_core::{ChatMessage, MemoryEntry};
use std::sync::Arc;
use tokio::sync::mpsc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum View { Chat, Config, Memory }

impl View {
    pub fn label(&self) -> &'static str {
        match self {
            View::Chat => "chat",
            View::Config => "config",
            View::Memory => "memory",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ChatState {
    pub messages: Vec<ChatMessage>,
    pub input: String,
    pub cursor: usize,
    pub input_history: Vec<String>,
    /// 浏览历史时的索引 (None = 不在浏览模式)
    pub history_browse: Option<usize>,
    /// 浏览历史前暂存的草稿
    pub history_draft: String,
    pub scroll_offset: usize,
    pub agent_busy: bool,
    /// 是否钉在底部 (跟随最新消息). 参考 ZeroClaw pinned_to_bottom.
    /// 滚动向上时设为 false, 新消息到来时只有 pinned_to_bottom=true 才自动跳底.
    pub pinned_to_bottom: bool,
    /// 是否显示思考内容 (<think> 标签 / reasoning_content). 默认 false.
    /// Ctrl+T 切换.
    pub show_thinking: bool,
}

impl Default for ChatState {
    fn default() -> Self {
        Self {
            messages: Vec::new(),
            input: String::new(),
            cursor: 0,
            input_history: Vec::new(),
            history_browse: None,
            history_draft: String::new(),
            scroll_offset: 0,
            agent_busy: false,
            pinned_to_bottom: true, // 默认钉在底部, 跟随最新消息
            show_thinking: false,   // 默认不显示思考内容
        }
    }
}

impl ChatState {
    /// 计算输入框显示行数: 1 行起步, 按换行符数增长, 上限 3
    pub fn input_height(&self) -> u16 {
        let lines = self.input.split('\n').count();
        lines.min(3).max(1) as u16
    }

    /// 向上滚动 (远离底部). 参考 ZeroClaw scroll_up(): pinned_to_bottom = false
    pub fn scroll_up(&mut self, lines: usize) {
        self.scroll_offset = self.scroll_offset.saturating_add(lines);
        self.pinned_to_bottom = false;
    }

    /// 向下滚动 (靠近底部). 参考 ZeroClaw scroll_down(): 到底时 pinned_to_bottom = true
    pub fn scroll_down(&mut self, lines: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(lines);
        if self.scroll_offset == 0 {
            self.pinned_to_bottom = true;
        }
    }
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
    pub running: bool,
    pub last_error: Option<String>,
    /// LLM 请求/响应瞬时状态 (状态栏显示)
    pub llm_status: Option<String>,
    /// Agent 别名 (状态栏显示)
    pub agent_alias: String,
    /// 模型名 (状态栏显示)
    pub model_name: String,
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
            .field("model_name", &self.model_name)
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
            running: true,
            last_error: None,
            llm_status: None,
            agent_alias: "shadow".to_string(),
            model_name: String::new(),
            agent: None,
            tx: None,
        }
    }
}

impl AppState {
    pub fn new() -> Self { Self::default() }

    /// 收集状态栏数据 (插件化: 各模块贡献 segment)
    pub fn status_data(&self) -> StatusBarData {
        let mut data = StatusBarData::new();

        // ── 左侧: 别名 · 视图 · 模型 ──
        data.push_left(StatusSegment::new(
            self.agent_alias.as_str(),
            theme::accent(),
        ));
        data.push_left(StatusSegment::new(
            self.view.label(),
            theme::dim(),
        ));
        if !self.model_name.is_empty() {
            data.push_left(StatusSegment::new(
                self.model_name.as_str(),
                theme::tool_text(),
            ));
        }
        if self.chat.agent_busy {
            data.push_left(StatusSegment::new("⏳", theme::assistant()));
        }
        if let Some(llm) = &self.llm_status {
            data.push_left(StatusSegment::new(llm.as_str(), theme::tool_text()));
        }

        // ── 右侧: 消息数 · 滚动位置 ──
        data.push_right(StatusSegment::new(
            format!("msg {}", self.chat.messages.len()),
            theme::dim(),
        ));
        if self.chat.scroll_offset > 0 {
            data.push_right(StatusSegment::new(
                format!("↑{}", self.chat.scroll_offset),
                theme::accent(),
            ));
        }

        data.hint = "↵ send · ⌥↵ newline · ↑↓ history · PgUp/PgDn scroll · ^T thinking · ⌘K palette · ^L clear"
            .to_string();
        data.error = self.last_error.clone();

        data
    }

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
                self.chat.cursor = 0;
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
            tool_call_id: None, tool_calls: vec![], reasoning_content: None,
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

    #[test]
    fn status_data_includes_alias_and_view() {
        let s = AppState::new();
        let data = s.status_data();
        assert!(!data.left.is_empty());
        // 第一段是别名
        assert_eq!(data.left[0].text, "shadow");
    }

    #[test]
    fn status_data_shows_busy_indicator() {
        let mut s = AppState::new();
        s.chat.agent_busy = true;
        let data = s.status_data();
        let texts: Vec<_> = data.left.iter().map(|seg| seg.text.as_str()).collect();
        assert!(texts.contains(&"⏳"));
    }

    #[test]
    fn status_data_error_overrides_hint() {
        let mut s = AppState::new();
        s.last_error = Some("broken".into());
        let data = s.status_data();
        assert_eq!(data.error.as_deref(), Some("broken"));
        assert!(!data.hint.is_empty());
    }

    #[test]
    fn input_height_single_line() {
        let s = ChatState::default();
        assert_eq!(s.input_height(), 1);
    }

    #[test]
    fn input_height_empty_is_one() {
        let s = ChatState::default();
        assert_eq!(s.input_height(), 1);
    }

    #[test]
    fn input_height_multiline_caps_at_three() {
        let mut s = ChatState::default();
        s.input = "line1\nline2\nline3\nline4\nline5".to_string();
        assert_eq!(s.input_height(), 3);
    }

    #[test]
    fn input_height_two_lines() {
        let mut s = ChatState::default();
        s.input = "hello\nworld".to_string();
        assert_eq!(s.input_height(), 2);
    }

    #[test]
    fn scroll_up_sets_unpinned() {
        let mut s = ChatState::default();
        assert!(s.pinned_to_bottom); // 默认钉在底部
        s.scroll_up(5);
        assert!(!s.pinned_to_bottom);
        assert_eq!(s.scroll_offset, 5);
    }

    #[test]
    fn scroll_down_to_bottom_sets_pinned() {
        let mut s = ChatState::default();
        s.scroll_up(10);
        assert!(!s.pinned_to_bottom);
        s.scroll_down(10); // 滚回底部
        assert!(s.pinned_to_bottom);
        assert_eq!(s.scroll_offset, 0);
    }

    #[test]
    fn scroll_down_partial_stays_unpinned() {
        let mut s = ChatState::default();
        s.scroll_up(10);
        s.scroll_down(3); // 只滚 3 行, 还没到底
        assert!(!s.pinned_to_bottom);
        assert_eq!(s.scroll_offset, 7);
    }
}
