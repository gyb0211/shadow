//! 事件 schema -- 简化版 OTel + 归因

use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// 日志严重级别
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl Severity {
    pub const TRACE: Self = Self::Trace;
    pub const DEBUG: Self = Self::Debug;
    pub const INFO: Self = Self::Info;
    pub const WARN: Self = Self::Warn;
    pub const ERROR: Self = Self::Error;

    #[must_use]
    pub fn number(self) -> u8 {
        match self {
            Self::Trace => 1,
            Self::Debug => 5,
            Self::Info => 9,
            Self::Warn => 13,
            Self::Error => 17,
        }
    }

    #[must_use]
    pub fn text(self) -> &'static str {
        match self {
            Self::Trace => "TRACE",
            Self::Debug => "DEBUG",
            Self::Info => "INFO",
            Self::Warn => "WARN",
            Self::Error => "ERROR",
        }
    }

    #[must_use]
    pub fn from_tracing_level(level: tracing::Level) -> Self {
        match level {
            tracing::Level::TRACE => Self::Trace,
            tracing::Level::DEBUG => Self::Debug,
            tracing::Level::INFO => Self::Info,
            tracing::Level::WARN => Self::Warn,
            tracing::Level::ERROR => Self::Error,
        }
    }
}

/// 日志分类
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventCategory {
    Agent,
    Channel,
    Tool,
    Provider,
    Memory,
    Session,
    System,
}

impl EventCategory {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Agent => "agent",
            Self::Channel => "channel",
            Self::Tool => "tool",
            Self::Provider => "provider",
            Self::Memory => "memory",
            Self::Session => "session",
            Self::System => "system",
        }
    }
}

/// 事件结果
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventOutcome {
    Success,
    Failure,
    Unknown,
}

impl EventOutcome {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Failure => "failure",
            Self::Unknown => "unknown",
        }
    }
}

/// 动作 -- 封闭枚举, 无 Other 逃逸 (借鉴 ZeroClaw)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Start,
    Complete,
    Fail,
    Cancel,
    Send,
    Receive,
    Read,
    Write,
    Delete,
    Query,
    Invoke,
    Note,
}

impl Action {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Complete => "complete",
            Self::Fail => "fail",
            Self::Cancel => "cancel",
            Self::Send => "send",
            Self::Receive => "receive",
            Self::Read => "read",
            Self::Write => "write",
            Self::Delete => "delete",
            Self::Query => "query",
            Self::Invoke => "invoke",
            Self::Note => "note",
        }
    }
}

/// 一行规范日志
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEvent {
    pub id: String,
    #[serde(rename = "@timestamp")]
    pub timestamp: String,
    pub severity_number: u8,
    pub severity_text: String,
    pub category: String,
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub attribution: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub attributes: Value,
}

impl LogEvent {
    #[must_use]
    pub fn new(severity: Severity, action: Action, category: EventCategory) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            timestamp: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
            severity_number: severity.number(),
            severity_text: severity.text().to_string(),
            category: category.as_str().to_string(),
            action: action.as_str().to_string(),
            outcome: None,
            attribution: BTreeMap::new(),
            message: None,
            attributes: Value::Null,
        }
    }

    pub fn with_outcome(mut self, outcome: EventOutcome) -> Self {
        self.outcome = Some(outcome.as_str().to_string());
        self
    }

    pub fn with_message(mut self, msg: impl Into<String>) -> Self {
        self.message = Some(msg.into());
        self
    }

    pub fn with_attr(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.attribution.insert(key.into(), value.into());
        self
    }
}
