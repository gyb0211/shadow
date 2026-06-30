//! LogCaptureLayer -- 捕获 record! 事件, 组装 LogEvent, 写入

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

        record_event(log_event);
    }
}

#[derive(Default)]
struct EventVisitor {
    action: Option<String>,
    message: Option<String>,
}

impl Visit for EventVisitor {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        match field.name() {
            "shadow_action" => self.action = Some(value.to_string()),
            "message" => self.message = Some(value.to_string()),
            _ => {}
        }
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        let s = format!("{value:?}");
        let s = s.trim_matches('"').to_string();
        match field.name() {
            "shadow_action" => self.action = Some(s),
            "message" => self.message = Some(s),
            _ => {}
        }
    }
}
