//! 事件 schema -- 简化版 OTel + 归因
//!
//! # 设计意图
//!
//! 借鉴 OpenTelemetry Logs Data Model + Elastic ECS, 但大幅精简:
//! - ZeroClaw: 5,079 行, 13 文件, OTel/ECS 混合 schema, 37 种 Action
//! - Shadow: 精简版, 单文件, 9 个 Category, 36 个 Action, 15 个标准归因字段
//!
//! 一条 `LogEvent` 是一行 JSONL, 同时供:
//! 1. 文件持久化 (writer.rs)
//! 2. 实时广播 (broadcast.rs)
//! 3. 指标投影 (observer_bridge.rs -> ObserverEvent)
//!
//! # 归因系统 (Attribution)
//!
//! 归因分两类字段:
//! - **标量归因** (`ATTRIBUTION_FIELDS`): 单值字段, 如 `agent_alias="assistant"`、`tool="shell"`
//! - **复合归因** (`COMPOSITE_PREFIXES`): 可拆分为 type/alias 的字段,
//!   如 `channel="feishu.default"` 同时展开为 `channel_type="feishu"` + `channel_alias="default"`
//!
//! 归因字段可来自: 宏直接写入 / span 上下文 (layer.rs 向上回溯合并) / 调用方手动 set。

use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::str::FromStr;
use strum_macros::{EnumString, IntoStaticStr};
use uuid::Uuid;

/// 日志严重级别。
///
/// `number()` 采用 OTel 风格的"留空可细分"编码: Trace=1, Debug=5, Info=9, Warn=13, Error=17,
/// 每级间隔 4 -- 便于插入 sub-level (如 Trace2/Trace3) 而不破坏顺序。
/// `severity_text_from_number` 在落盘时把 0..20 范围还原成文本, >20 归为 FATAL。
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

    /// OTel 数字编码, 间隔 4 (见类型注释)。
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

    /// 从 `tracing::Level` 转换 -- layer.rs 用此桥接 tracing 事件到 LogEvent。
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

/// 日志分类 -- 标识事件来源子系统。
///
/// 用于过滤/聚合: 例如只看 Agent 事件, 或统计 Memory 子系统的失败率。
/// 9 类对应 Shadow 的核心 trait / 资源: Agent/Channel/Tool/Provider/Memory/Session/Cron
/// 外加 System (运行时自身) 与 Internal (日志框架自身, 通常被过滤掉)。
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

    /// 字符串解析, 未识别返回 None (非 panic)。
    pub fn parse(s: &str) -> Option<Self> {
        Self::from_str(s).ok()
    }
}

/// 事件结果 -- 三态而非 bool, 区分"未观察到"与"成功"。
///
/// `Unknown` 是默认值, 序列化时被 `skip_serializing_if = is_unknown_outcome` 去掉。
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

    pub fn parse(s: &str) -> Option<Self> {
        Self::from_str(s).ok()
    }
}

/// OTel 风格的事件描述符 (event.name + event.domain + event.outcome)。
///
/// - `category` 对应 OTel 的 `event.domain`, 但用 Shadow 自有分类
/// - `action` 对应 ECS `event.action`, 自由字符串 (实际由宏传入 `Action` 枚举的静态名)
/// - `outcome` 仅在非 unknown 时序列化
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

/// ECS `service.*` 描述符 -- 标识产生事件的进程。
/// 默认 `name="shadow"`, version 取编译期 cargo pkg version。
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

/// 标量归因字段白名单 (15 个)。
///
/// 这些字段是 Shadow 各子系统的"身份指针", 用于关联事件到具体资源:
/// - `agent_alias` / `tool` / `session_key` / `cron_job_id` -- 直接标识资源
/// - `risk_profile` / `runtime_profile` -- 引用配置
/// - `memory_namespace` / `skill_bundle` / `knowledge_bundle` / `mcp_bundle` -- 引用存储/打包
/// - `peer_group` / `sop_name` -- 多 Agent 协作
/// - `model` / `embedding_provider` -- 引用模型
/// - `owner_tui_id` -- TUI 模式下的属主
///
/// 在 layer.rs 里 `is_attribution_field` 用于识别宏传来的字段是否属于归因协议,
/// 决定是否归入 `Attribution.fields` 而非普通 attributes。
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

/// 复合归因前缀 (5 个)。
///
/// 每个前缀 P 对应三个字段: P / P_type / P_alias。
/// 写入时 `set_composite(P, "type.alias")` 一次性展开三个字段,
/// 例如 `set_composite("channel", "feishu.default")` 会写入:
/// - `channel = "feishu.default"` (完整复合值)
/// - `channel_type = "feishu"`
/// - `channel_alias = "default"`
///
/// 若传入不含 ".", 视为只有 type, alias 字段不写。
pub const COMPOSITE_PREFIXES: &[&str] = &[
    "channel",
    "model_provider",
    "tts_provider",
    "transcription_provider",
    "tunnel_provider",
];

/// 复合前缀 P 的 type 字段名: `"{P}_type"`。
#[must_use]
pub fn type_field(prefix: &str) -> String {
    format!("{prefix}_type")
}

/// 复合前缀 P 的 alias 字段名: `"{P}_alias"`。
#[must_use]
pub fn alias_field(prefix: &str) -> String {
    format!("{prefix}_alias")
}

/// 判断字段名是否属于归因协议 (标量或复合)。
///
/// layer.rs 用此分流: 归因字段进 `Attribution.fields`, 其它进 `attributes`。
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

/// 归因上下文 -- 一个事件"是谁、对什么、用了多久"的载体。
///
/// `fields` 用 `BTreeMap` 而非 `HashMap`: 序列化输出字段名有序,
/// 便于 diff/阅读; 同一事件多次产生时 JSON 顺序稳定。
///
/// `duration_ms` 单列出来不进 fields, 因为它是数值类型, 与 String fields 混存会丢类型信息。
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

    /// 一次性写入复合字段及其 type/alias 展开。详见 `COMPOSITE_PREFIXES` 注释。
    pub fn set_composite(&mut self, prefix: &str, composite: &str) {
        self.set(prefix.to_string(), composite.to_string());
        if let Some((ty, alias)) = composite.split_once(".") {
            self.set(type_field(prefix), ty);
            self.set(alias_field(alias), alias);
        } else {
            // 注意: 仅写 type 字段, 不写 alias。这会让 alias 缺失,
            // 但保持 type 字段始终存在 (查询时不必区分是否复合)。
            self.set(type_field(prefix), composite.to_string());
        }
    }

    /// 合并另一个归因, **已有键不被覆盖** (语义: 子 span 的归因优先于父 span)。
    /// duration_ms 仅在自身为 None 时采纳对方。
    pub fn merge_from(&mut self, other: &Self) {
        for (k, v) in &other.fields {
            self.fields.entry(k.clone()).or_insert_with(|| v.clone());
        }
        if self.duration_ms.is_none() {
            self.duration_ms = other.duration_ms;
        }
    }

    /// 是否所有协议字段都已填齐。用于诊断: 一条事件若 is_fully_populated=false,
    /// 说明归因链路上有缺失, 排查时能看到具体缺哪些。
    pub fn is_fully_populated(&self) -> bool {
        ATTRIBUTION_FIELDS
            .iter()
            .all(|k| self.fields.contains_key(*k))
            && COMPOSITE_PREFIXES
                .iter()
                .all(|p| self.fields.contains_key(*p))
    }
}

/// 一行规范日志 -- 最终落盘 / 广播 / 投影的载体。
///
/// 字段命名混合 ECS / OTel:
/// - `@timestamp` 按 ECS 用 `@` 前缀 (便于 Elasticsearch 索引识别)
/// - `severity_number` / `severity_text` 来自 OTel Logs Data Model
/// - `event` / `service` 是 ECS 顶层对象
/// - `trace_id` / `span_id` 来自 OTel, 但 Shadow 当前不强制 tracing context
/// - `attribution` 是 Shadow 自定义, ECS 没有对应概念
/// - `attributes` 是 OTel 的 AnyValue 自由属性, 兜底存放非协议字段
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEvent {
    /// 事件 UUID, 用于跨系统关联 (例如广播与文件落盘对应同一事件)
    pub id: String,
    /// RFC3339 毫秒精度 UTC, ECS 规范以 `@` 前缀
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

    /// 自由属性 (JSON Value)。layer.rs 会把 file/line 以 `_file`/`_line` 前缀存这里。
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub attributes: Value,
    /// schema 版本, 用于未来字段迁移。当前固定 1。
    #[serde(default = "default_schema_version")]
    pub schema_version: u8,
}
fn default_schema_version() -> u8 {
    LogEvent::SCHEMA_VERSION
}

impl LogEvent {
    pub const SCHEMA_VERSION: u8 = 1;
    /// 构造一条新事件: outcome 默认 Unknown, service 用进程默认, trace/span 为 None。
    /// 归因 / message / attributes 由调用方或 layer.rs 后续填充。
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

/// 把数字还原成严重度文本 -- 落盘后查询时用。
/// 阶梯与 `Severity::number()` 对齐: 每级 4 数字, >17 归 FATAL (无对应枚举值, 仅文本)。
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

/// 动作 -- 封闭枚举, **无 Other 逃逸** (借鉴 ZeroClaw)。
///
/// 设计原则: 不允许 "Other" 让事件悄悄游离协议。新动作必须显式加入枚举,
/// 这迫使每次新增都审视是否归入正确语义类别, 保证 `event.action` 字段可被聚合。
///
/// 按语义分组 (无显式 group, 注释帮助阅读):
/// - 生命周期: Start / Complete / Fail / Cancel / Skip / Timeout / Retry
/// - 通信方向: Inbound / Outbound / Send / Receive
/// - 连接: Connect / Disconnect / Reconnect
/// - 进程管理: Spawn / Kill
/// - 调度: Tick / Trigger / Schedule
/// - 审批: Approve / Reject / Defer
/// - CRUD: Read / Write / Delete / List / Query
/// - 调用: Invoke / Dispatch / Resolve
/// - 注册: Register / Unregister
/// - 数据: Load / Save / Migrate / Validate
/// - 元事件: Note (纯备注, 无副作用)
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

/// 宏展开用的临时事件结构 -- record! 宏构造此对象, layer.rs 再读取字段组装 LogEvent。
///
/// 设计: builder 模式 (`with_category` / `with_duration` / `with_attrs`),
/// 让宏调用点写法简洁, 复杂的字段名约定 (sd_*) 在 layer.rs 集中处理。
///
/// 注意 `name: &'static str` -- 动作名要求编译期字符串, 不能运行时拼接,
/// 这与 `Action` 枚举的封闭性呼应: 强制静态命名, 杜绝拼写出协议外字段。
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


    pub fn with_outcome(mut self, outcome: EventOutcome) -> Self {
        self.outcome = outcome;
        self
    }

    pub fn with_attrs(mut self, attrs: Value) -> Self {
        self.attrs = Some(attrs);
        self
    }

    /// category 字符串, 未设置时返回空串 (而非 None) -- 方便宏拼接 sd_category 字段。
    pub fn category_str(&self) -> &'static str {
        self.category.map_or("", EventCategory::as_str)
    }

    pub fn outcome_str(&self) -> &'static str {
        self.outcome.as_str()
    }

    /// attrs 序列化为 JSON 字符串, None 返回空串。layer.rs 会按需 parse 回 Value。
    pub fn attrs_str(&self) -> String {
        match &self.attrs {
            Some(v) => serde_json::to_string(v).unwrap_or_default(),
            None => String::new(),
        }
    }

    /// duration 取值或 0, 便于宏无条件写入 sd_duration_ms。
    pub fn duration_ms(&self) -> u64 {
        self.duration_ms.unwrap_or(0)
    }

    /// 是否显式设置了 duration -- 决定 layer.rs 是否把 duration_ms 写入归因。
    pub fn has_duration(&self) -> bool {
        self.duration_ms.is_some()
    }
}
