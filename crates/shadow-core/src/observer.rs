//! 观察者 trait -- 指标和追踪

use std::any::Any;

use crate::attribution::{Attributable, Role};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// 观察者事件 -- #[non_exhaustive] 保证外部实现对新变体优雅降级
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ObserverEvent {
    /// LLM 请求开始
    LlmRequest { model: String, message_count: usize },
    /// LLM 响应完成
    LlmResponse { model: String, duration_ms: u64, tokens: u64 },
    /// 工具调用
    ToolCall { tool: String, success: bool, duration_ms: u64, output_preview: String },
    /// 会话开始
    SessionStart { session_id: String },
    /// 会话结束
    SessionEnd { session_id: String },
    /// 错误
    Error { message: String },
}

/// 观察者 trait
///
/// 后端实现: Log / Prometheus / OTel (未来)
#[async_trait]
pub trait Observer: Attributable {
    /// 记录事件
    fn record_event(&self, event: &ObserverEvent);

    /// 刷新缓冲
    fn flush(&self) {}

    /// as_any 用于 downcast
    fn as_any(&self) -> &dyn Any;
}

/// 空观察者 -- 默认实现, 零开销
pub struct NoopObserver;

impl Attributable for NoopObserver {
    fn role(&self) -> Role {
        Role::System
    }
    fn alias(&self) -> &str {
        "noop-observer"
    }
}

#[async_trait]
impl Observer for NoopObserver {
    fn record_event(&self, _event: &ObserverEvent) {}

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_call_event_carries_output_preview() {
        let ev = ObserverEvent::ToolCall {
            tool: "shell".to_string(),
            success: true,
            duration_ms: 42,
            output_preview: "hello\nworld".to_string(),
        };
        if let ObserverEvent::ToolCall { output_preview, .. } = ev {
            assert_eq!(output_preview, "hello\nworld");
        } else {
            panic!("wrong variant");
        }
    }
}
