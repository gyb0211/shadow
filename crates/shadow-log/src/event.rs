//! 事件 schema -- 简化版 OTel + 归因

use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::str::FromStr;
use strum_macros::{EnumString, IntoStaticStr};
use uuid::Uuid;

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
#[derive(Debug, Clone, Copy, PartialEq, Eq, IntoStaticStr, EnumString)]
pub enum EventCategory {
    Agent,
    Channel,
    Tool,
    Provider,
    Memory,
    Session,
    System,
    Cron,
    Internal,
}

impl EventCategory {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        self.into()
    }
    
    pub fn parse( s: &str) -> Option<Self>{
        Self::from_str(s).ok()
    }
}

/// 事件结果
#[derive(Debug, Clone, Copy, PartialEq, Eq, IntoStaticStr, EnumString)]
pub enum EventOutcome {
    Success,
    Failure,
    Unknown,
}

impl EventOutcome {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        self.into()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventDescriptor {
    pub category: String,
    pub action: String,
    #[serde(default, skip_serializing_if = "is_unknown_outcome")]
    pub outcome: String,
}

fn is_unknown_outcome(out: &String) -> bool {
    out == "unknown" || out.is_empty()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceDescriptor {
    pub name: String,
    pub version: String,
}

impl Default for ServiceDescriptor {
    fn default() -> Self {
        Self {
            name: "shadow".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }
}

pub const ATTRIBUTION_FIELDS: &[&str] = &[
    "agent_alias",
    "tool",
    "session_key",
    "cron_job_id",
    "risk_profile",
    "runtime_profile",
    "memory_namespace",
    "skill_bundle",
    "knowledge_bundle",
    "mcp_bundle",
    "peer_group",
    "sop_name",
    "model",
    "embedding_provider",
    "owner_tui_id",
];

pub const COMPOSITE_PREFIXES: &[&str] = &[
    "channel",
    "model_provider",
    "tts_provider",
    "transcription_provider",
    "tunnel_provider",
];

#[must_use]
pub fn type_field(prefix: &str) -> String {
    format!("{prefix}_type")
}

#[must_use]
pub fn alias_field(prefix: &str) -> String {
    format!("{prefix}_alias")
}

#[must_use]
pub fn is_attribution_field(name: &str) -> bool {
    if ATTRIBUTION_FIELDS.contains(&name) {
        return true;
    }
    for prefix in COMPOSITE_PREFIXES {
        if name == *prefix {
            return true;
        }
        if name == type_field(prefix) || name == alias_field(prefix) {
            return true;
        }
    }
    false
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Attribution {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub fields: BTreeMap<String, String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
}

impl Attribution {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.fields.get(key).map(String::as_str)
    }

    pub fn set(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.fields.insert(key.into(), value.into());
    }

    pub fn set_composite(&mut self, prefix: &str, composite: &str) {
        self.set(prefix.to_string(), composite.to_string());
        if let Some((ty, alias)) = composite.split_once(".") {
            self.set(type_field(prefix), ty);
            self.set(alias_field(alias), alias);
        } else {
            self.set(type_field(prefix), composite.to_string());
        }
    }

    pub fn merge_from(&mut self, other: &Self) {
        for (k, v) in &other.fields {
            self.fields.entry(k.clone()).or_insert_with(|| v.clone());
        }
        if self.duration_ms.is_none() {
            self.duration_ms = other.duration_ms;
        }
    }

    pub fn is_fully_populated(&self) -> bool {
        ATTRIBUTION_FIELDS
            .iter()
            .all(|k| self.fields.contains_key(*k))
            && COMPOSITE_PREFIXES
                .iter()
                .all(|p| self.fields.contains_key(*p))
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

    pub event: EventDescriptor,
    #[serde(default)]
    pub service: ServiceDescriptor,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span_id: Option<String>,
    #[serde(default)]
    pub attribution: Attribution,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,

    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub attributes: Value,
    #[serde(default = "default_schema_version")]
    pub schema_version: u8,
}
fn default_schema_version() -> u8 {
    LogEvent::SCHEMA_VERSION
}

impl LogEvent {
    pub const SCHEMA_VERSION: u8 = 1;
    #[must_use]
    pub fn new(severity: Severity, action: &str, category: EventCategory) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            timestamp: Utc::now()
                .to_rfc3339_opts(SecondsFormat::Millis, true)
                .to_string(),
            severity_number: severity.number(),
            severity_text: severity.text().to_string(),
            event: EventDescriptor {
                category: category.as_str().to_string(),
                action: action.to_string(),
                outcome: EventOutcome::Unknown.as_str().to_string(),
            },
            service: Default::default(),
            trace_id: None,
            span_id: None,
            attribution: Default::default(),
            message: None,
            attributes: Value::Null,
            schema_version: Self::SCHEMA_VERSION,
        }
    }

    pub fn set_outcome(&mut self, outcome: EventOutcome) {
        self.event.outcome = outcome.as_str().to_string();
    }
}

#[must_use]
pub fn severity_text_from_number(n: u8) -> &'static str {
    match n {
        0..=4 => "TRACE",
        5..=8 => "DEBUG",
        9..=12 => "INFO",
        13..=16 => "WARN",
        17..=20 => "ERROR",
        _ => "FATAL",
    }
}

#[must_use]
pub fn severity_text_from_tracing_level(level: tracing::Level) -> &'static str {
    Severity::from_tracing_level(level).text()
}

/// 动作 -- 封闭枚举, 无 Other 逃逸 (借鉴 ZeroClaw)
#[derive(Debug, Clone, Copy, PartialEq, Eq, IntoStaticStr, EnumString)]
pub enum Action {
    Start,
    Complete,
    Fail,
    Cancel,
    Skip,
    Timeout,
    Retry,
    Inbound,
    Outbound,
    Send,
    Receive,
    Connect,
    Disconnect,
    Reconnect,
    Spawn,
    Kill,
    Tick,
    Trigger,
    Schedule,
    Approve,
    Reject,
    Defer,
    Read,
    Write,
    Delete,
    List,
    Query,
    Invoke,
    Dispatch,
    Resolve,
    Register,
    Unregister,
    Load,
    Save,
    Migrate,
    Validate,
    Note,
}

impl Action {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        self.into()
    }
}

#[derive(Debug, Clone)]
pub struct Event {
    pub name: &'static str,
    pub action: Action,
    pub category: Option<EventCategory>,
    pub outcome: EventOutcome,
    pub duration_ms: Option<u64>,
    pub attrs: Option<Value>,
}

impl Event {
    pub fn new(name: &'static str, action: Action) -> Self {
        Self {
            name,
            action,
            category: None,
            outcome: EventOutcome::Unknown,
            duration_ms: None,
            attrs: None,
        }
    }

    pub fn with_category(mut self, category: EventCategory) -> Self {
        self.category = Some(category);
        self
    }

    pub fn with_duration(mut self, duration: u64) -> Self {
        self.duration_ms = Some(duration);
        self
    }

    pub fn with_attrs(mut self, attrs: Value) -> Self {
        self.attrs = Some(attrs);
        self
    }

    pub fn category_str(&self) -> &'static str {
        self.category.map_or("", EventCategory::as_str)
    }

    pub fn outcome_str(&self) -> &'static str {
        self.outcome.as_str()
    }

    pub fn attrs_str(&self) -> String {
        match &self.attrs {
            Some(v) => serde_json::to_string(v).unwrap_or_default(),
            None => String::new(),
        }
    }

    pub fn duration_ms(&self) -> u64 {
        self.duration_ms.unwrap_or(0)
    }

    pub fn has_duration(&self) -> bool {
        self.duration_ms.is_some()
    }
}
