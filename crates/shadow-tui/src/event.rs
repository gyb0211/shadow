//! 跨线程事件 -- 后台 Agent task → 主线程 UI

use crossterm::event::KeyEvent;
use shadow_core::MemoryEntry;

#[derive(Debug, Clone)]
pub enum AppEvent {
    /// crossterm 键盘事件
    Key(KeyEvent),
    /// 一条 assistant 回复 (完整)
    AgentMessage(String),
    /// 一次工具调用
    AgentToolCall {
        name: String,
        success: bool,
        output_preview: String,
        duration_ms: u64,
    },
    /// Agent 一次 chat 调用完成
    AgentDone,
    /// Agent 错误 (provider HTTP / 网络)
    AgentError(String),
    /// 顶部状态栏更新
    Status(String),
    /// Memory 异步加载完成
    MemoryLoaded(Vec<MemoryEntry>),
}

impl std::fmt::Display for AppEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AppEvent::Status(s) => write!(f, "{s}"),
            AppEvent::AgentMessage(s) => write!(f, "{s}"),
            _ => write!(f, "{self:?}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_event_clone_preserves_text() {
        let ev = AppEvent::Status("hello".to_string());
        assert_eq!(ev.clone().to_string(), "hello");
    }
}
