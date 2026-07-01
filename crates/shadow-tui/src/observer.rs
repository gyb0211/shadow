//! UiObserver -- 把 shadow_core::Observer 事件转发到 mpsc, 供 UI 渲染

use async_trait::async_trait;
use shadow_core::{Attributable, Observer, ObserverEvent, Role};
use std::any::Any;
use std::sync::Arc;
use tokio::sync::mpsc;

use crate::event::AppEvent;

pub struct UiObserver {
    tx: mpsc::Sender<AppEvent>,
}

impl UiObserver {
    pub fn new(tx: mpsc::Sender<AppEvent>) -> Self {
        Self { tx }
    }

    pub fn arc(tx: mpsc::Sender<AppEvent>) -> Arc<Self> {
        Arc::new(Self::new(tx))
    }
}

impl Attributable for UiObserver {
    fn role(&self) -> Role {
        Role::System
    }
    fn alias(&self) -> &str {
        "ui-observer"
    }
}

#[async_trait]
impl Observer for UiObserver {
    fn record_event(&self, event: &ObserverEvent) {
        let app = match event {
            ObserverEvent::LlmRequest { model, .. } => {
                AppEvent::Status(format!("→ {model}"))
            }
            ObserverEvent::LlmResponse {
                duration_ms, tokens, ..
            } => AppEvent::Status(format!("← {duration_ms}ms · {tokens} tok")),
            ObserverEvent::ToolCall {
                tool,
                success,
                output_preview,
                duration_ms,
            } => AppEvent::AgentToolCall {
                name: tool.clone(),
                success: *success,
                output_preview: output_preview.clone(),
                duration_ms: *duration_ms,
            },
            ObserverEvent::Error { message } => AppEvent::AgentError(message.clone()),
            _ => return,
        };
        // 非阻塞发送: 主线程消费慢的话丢弃最早事件
        let _ = self.tx.try_send(app);
    }

    fn flush(&self) {}

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shadow_core::{ChatRequest, ChatResponse, Provider, TokenUsage};
    use anyhow::Result;

    struct StubProvider;
    impl Attributable for StubProvider {
        fn role(&self) -> Role {
            Role::Provider
        }
        fn alias(&self) -> &str {
            "stub"
        }
    }
    #[async_trait]
    impl Provider for StubProvider {
        fn provider_type(&self) -> &str {
            "stub"
        }
        async fn chat(&self, _: ChatRequest) -> Result<ChatResponse> {
            Ok(ChatResponse {
                content: "ok".into(),
                tool_calls: vec![],
                usage: TokenUsage::default(),
            })
        }
        async fn list_models(&self) -> Result<Vec<String>> {
            Ok(vec![])
        }
    }

    #[tokio::test]
    async fn forwards_tool_call_to_channel() {
        let (tx, mut rx) = mpsc::channel::<AppEvent>(16);
        let obs = UiObserver::new(tx);
        obs.record_event(&ObserverEvent::ToolCall {
            tool: "shell".to_string(),
            success: true,
            duration_ms: 5,
            output_preview: "hello".to_string(),
        });
        let ev = rx.recv().await.unwrap();
        match ev {
            AppEvent::AgentToolCall {
                name, output_preview, ..
            } => {
                assert_eq!(name, "shell");
                assert_eq!(output_preview, "hello");
            }
            _ => panic!("wrong event"),
        }
    }

    #[tokio::test]
    async fn drops_event_when_channel_full() {
        let (tx, _rx) = mpsc::channel::<AppEvent>(1);
        let obs = UiObserver::new(tx.clone());
        // 先填满
        let _ = tx.try_send(AppEvent::Status("fill".into())).ok();
        // 再发, 应该静默 drop, 不 panic
        obs.record_event(&ObserverEvent::Error {
            message: "x".into(),
        });
    }
}
