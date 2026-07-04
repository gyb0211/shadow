//! LogCaptureLayer -- 捕获 record! 事件, 组装 LogEvent, 写入
//!
//! 参考 ZeroClaw layer.rs:
//! - 捕获 target=shadow_log_event 的事件
//! - 从 visit 中提取 action, message, shadow_attrs (归因字段)
//! - 组装 LogEvent 后调 record_event

use crate::event::{Action, EventCategory, LogEvent, Severity};
use crate::writer::record_event;
use tracing::field::Visit;
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::Context;
use tracing_subscriber::Layer;

pub struct LogCaptureLayer;

impl<S: Subscriber> Layer<S> for LogCaptureLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let metadata = event.metadata();
        let target = metadata.target();

        // 只捕获 record! 宏的事件
        if target != "shadow_log_event" {
            return;
        }

        let severity = Severity::from_tracing_level(*metadata.level());
        let mut visitor = EventVisitor::default();
        event.record(&mut visitor);

        let action_str = visitor.action.as_deref().unwrap_or("note");
        let action = match action_str {
            "start" => Action::Start,
            "complete" => Action::Complete,
            "fail" => Action::Fail,
            "cancel" => Action::Cancel,
            "send" => Action::Send,
            "receive" => Action::Receive,
            "read" => Action::Read,
            "write" => Action::Write,
            "delete" => Action::Delete,
            "query" => Action::Query,
            "invoke" => Action::Invoke,
            _ => Action::Note,
        };

        let category = EventCategory::System;
        let mut log_event = LogEvent::new(severity, action, category);

        if let Some(msg) = visitor.message {
            log_event = log_event.with_message(msg);
        }

        // 解析归因字段 (shadow_attrs JSON 字符串)
        if let Some(ref attrs_json) = visitor.attrs
            && let Ok(attrs) = serde_json::from_str::<serde_json::Value>(attrs_json)
                && let Some(obj) = attrs.as_object() {
                    for (key, val) in obj {
                        if let Some(s) = val.as_str() {
                            log_event = log_event.with_attr(key, s);
                        }
                    }
                }

        record_event(log_event);
    }
}

#[derive(Default)]
struct EventVisitor {
    action: Option<String>,
    message: Option<String>,
    /// record! 宏的归因字段 (JSON 字符串)
    attrs: Option<String>,
}

impl Visit for EventVisitor {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        match field.name() {
            "shadow_action" => self.action = Some(value.to_string()),
            "message" => self.message = Some(value.to_string()),
            "shadow_attrs" => self.attrs = Some(value.to_string()),
            _ => {}
        }
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        let s = format!("{value:?}");
        let s = s.trim_matches('"').to_string();
        match field.name() {
            "shadow_action" => self.action = Some(s),
            "message" => self.message = Some(s),
            "shadow_attrs" => self.attrs = Some(s),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{Action, EventCategory, LogEvent, Severity};

    #[test]
    fn log_event_with_attribution() {
        let event = LogEvent::new(Severity::Info, Action::Send, EventCategory::Provider)
            .with_message("LLM 请求")
            .with_attr("model", "MiniMax-M2.7")
            .with_attr("agent", "shadow");
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("MiniMax-M2.7"));
        assert!(json.contains("shadow"));
        assert!(json.contains("send"));
    }

    #[test]
    fn log_event_with_outcome() {
        let event = LogEvent::new(Severity::Warn, Action::Fail, EventCategory::Tool)
            .with_message("tool failed")
            .with_outcome(crate::event::EventOutcome::Failure);
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("failure"));
    }
}
