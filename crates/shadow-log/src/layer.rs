//! LogCaptureLayer -- 捕获 record! 事件, 组装 LogEvent, 写入
//!
//! # 角色
//!
//! 实现 `tracing_subscriber::Layer`, 是 `record!` 宏与 `record_event` (writer.rs) 之间的桥梁。
//! 职责:
//! 1. 识别 3 类受控 target (`log_event` / `log_attribution` / `log_scope`) 的 span/event
//! 2. 用 `Visit` trait 把 tracing 字段值提取为强类型字段 (sd_* 约定)
//! 3. 把 span 上挂载的归因上下文向上回溯, 合并到当前事件
//! 4. 组装成 `LogEvent` 后交给 writer 落盘
//!
//! # 3 类 target 的分工
//!
//! | target | 由谁发出 | 用途 |
//! |--------|----------|------|
//! | `log_event` | record! 宏的事件 | 一次具体动作, 落盘为一条 LogEvent |
//! | `log_attribution` | attribution_span! 宏 | 把归因字段绑定到 span, 所有内部事件自动继承 |
//! | `log_scope` | scope_span! 宏 | 同上, 额外支持 category / 自由 attributes |
//!
//! `log_internal` 前缀的事件被抑制 -- 用于本 crate 自身的 warn (如 append 失败),
//! 避免反馈循环: 落盘失败 -> warn -> 再次尝试落盘 -> 再次失败。
//!
//! # sd_* 字段命名约定
//!
//! `record!` 宏传出的字段统一用 `sd_` 前缀 (shadow), 见 `EventCollector::put` 的分支。
//! 非 sd_ 前缀的字段被视为"自由属性", 进 `attributes` 而非归因。
//!
//! 参考 ZeroClaw layer.rs:
//! - 捕获 target=shadow_log_event 的事件
//! - 从 visit 中提取 action, message, shadow_attrs (归因字段)
//! - 组装 LogEvent 后调 record_event

use std::fmt::Write;
use crate::EventOutcome;
use crate::event::{ Attribution, EventCategory, LogEvent, Severity, ATTRIBUTION_FIELDS, COMPOSITE_PREFIXES};
use crate::writer::record_event;
use serde_json::Value;
use serde_json::map::Map as JsonMap;
use std::error::Error;
use std::fmt::Debug;
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Record};
use tracing::{Event, Id, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::{LookupSpan, SpanRef};

/// 空 struct -- Layer 行为零状态, 实例化无配置。
pub struct LogCaptureLayer;

/// record! 宏事件 target -- 真正落盘的事件来源。
const TARGET_EVENT: &str = "log_event";

/// 归因 span target -- 由 attribution_span! 宏发出, 把归因字段绑定到调用栈。
const SHADOW_ATTRIBUTION_SPAN: &str = "log_attribution";
/// 作用域 span target -- 由 scope_span! 宏发出, 类似 attribution 但支持更多元数据。
const SHADOW_SCOPE_SPAN: &str = "log_scope";
/// 抑制前缀 -- 防止日志框架自身的 warn 形成反馈循环。
const TARGET_SUPPRESS_PREFIX: &str = "log_internal";

/// 角色家族字段名 -- 归因里的"顶层角色类别" (如 agent / tool / channel)。
const SHADOW_ROLE: &str = "sd_role";

impl<S> Layer<S> for LogCaptureLayer
where
    S: Subscriber + for<'l> LookupSpan<'l>,
{
    /// span 创建时:按 target 分流, 把字段挂到 span extensions 上。
    /// 这些 extensions 在 on_event 时被回溯读取。
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        let target = attrs.metadata().target();
        let Some(span) = ctx.span(id) else {
            return;
        };

        // 只捕获 record! 宏的事件
        if target == SHADOW_ATTRIBUTION_SPAN {
            let mut v = AttributionSpanCollector::default();
            attrs.record(&mut v);
            let mut attribution = Attribution::default();
            let default_category = v.default_category.as_deref().and_then(EventCategory::parse);
            v.apply_info(&mut attribution);

            let mut exts = span.extensions_mut();
            exts.insert(attribution);

            if let Some(cat) = default_category {
                exts.insert(SpanCategory(cat))
            }
            return;
        }

        if target == SHADOW_SCOPE_SPAN {
            let mut v = ScopeSpanCollector::default();
            attrs.record(&mut v);
            v.install(span);
        }
    }

    /// span 字段后续追加 (span.record(...)) 时触发。
    /// 与 on_new_span 走相同的收集器, 关键区别: 与已有 extensions **合并而非覆盖**。
    fn on_record(&self, id: &Id, values: &Record<'_>, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(id) else {
            return;
        };
        let target = span.metadata().target();
        if target == SHADOW_ATTRIBUTION_SPAN {
            let mut v = AttributionSpanCollector::default();
            values.record(&mut v);
            let mut attribution = Attribution::default();
            v.apply_info(&mut attribution);
            let mut exts = span.extensions_mut();
            if let Some(exist) = exts.get_mut::<Attribution>() {
                exist.merge_from(&attribution)
            } else {
                exts.insert(attribution);
            }
            return;
        }

        if target == SHADOW_SCOPE_SPAN {
            let mut v = ScopeSpanCollector::default();
            values.record(&mut v);
            v.install(span);
        }
    }

    /// 事件触发 -- 主路径。组装 LogEvent 并落盘。
    ///
    /// 步骤:
    /// 1. 抑制 log_internal (避免反馈循环)
    /// 2. 用 EventCollector visit 出 sd_* 字段
    /// 3. 决定 action: 显式 sd_action 优先, 否则用 metadata.name() (tracing 宏的字符串字面量)
    /// 4. 决定 category: 显式 > span 链上的 SpanCategory > 默认 Internal
    /// 5. 决定 name: sd_name > action_str (兜底)
    /// 6. 处理 outcome / message / duration / attrs / extra / file+line
    /// 7. 向上遍历 span 链, 合并 Attribution 和 ScopeExtra
    /// 8. 兜底 trace_id: 若 attributes 里有 "trace_id" 字段, 提升到顶层
    /// 9. 调 record_event 落盘
    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        let metadata = event.metadata();
        let target = metadata.target();

        if target.starts_with(TARGET_SUPPRESS_PREFIX) {
            return;
        }

        let severity = Severity::from_tracing_level(*metadata.level());
        let mut visitor = EventCollector::default();
        event.record(&mut visitor);

        // action 优先级: 显式 sd_action > tracing 宏的事件名 (如 "agent start")
        let action_str = visitor
            .action
            .as_deref()
            .map(str::to_string)
            .unwrap_or_else(|| metadata.name().to_string());

        // category 优先级: 显式 > span 链查找 > Internal 兜底。
        // span 链查找遍历当前 span 及其所有祖先, 取第一个设置了 SpanCategory 的。
        let category = visitor
            .category
            .as_deref()
            .filter(|s| !s.is_empty())
            .and_then(EventCategory::parse)
            .or_else(|| {
                ctx.lookup_current()
                    .into_iter()
                    .flat_map(|span| span.scope())
                    .find_map(|span| span.extensions().get::<SpanCategory>().map(|c| c.0))
            })
            .unwrap_or(EventCategory::Internal);

        // name 优先级: sd_name > action_str
        let name = visitor
            .name
            .as_deref()
            .unwrap_or(action_str.as_str())
            .to_string();

        let mut log_event = LogEvent::new(severity, &name, category);

        // 仅当 target=log_event 时才用 action_str 覆盖 LogEvent::new() 设的 action。
        // 其它 target 的 action 由 LogEvent::new 用 name 初始化。
        if target == TARGET_EVENT {
            log_event.event.action = action_str;
        }

        if let Some(outcome) = visitor.outcome.as_deref().and_then(EventOutcome::parse) {
            log_event.set_outcome(outcome);
        }

        // message 即使为空也写入, 保证字段存在
        log_event.message = Some(visitor.message.unwrap_or_default());

        // duration 仅在显式 has_duration=true 时才采纳, 避免 0 值污染归因
        if visitor.has_duration.unwrap_or(false) {
            log_event.attribution.duration_ms = visitor.duration_ms;
        }

        // sd_attrs 是 record! 宏传入的 JSON 字符串, 反序列化为 Value 整体覆盖
        if let Some(attrs) = visitor.attrs
            && !attrs.is_empty()
            && let Ok(v) = serde_json::from_str::<Value>(&attrs)
        {
            log_event.attributes = v;
        }

        // extra (非 sd_ 前缀的自由字段) 合并进 attributes, 已有键保留 (sd_attrs 优先)
        if !visitor.extra.is_empty() {
            if log_event.attributes.is_null() {
                log_event.attributes = Value::Object(visitor.extra);
            } else if let Value::Object(map) = &mut log_event.attributes {
                for (k, v) in visitor.extra {
                    map.entry(k).or_insert(v);
                }
            }
        }

        // file/line 用下划线前缀避免与归因协议字段冲突
        if visitor.file.is_some() || visitor.line.is_some() {
            let map = match &mut log_event.attributes {
                Value::Object(m) => m,
                _ => {
                    log_event.attributes = Value::Object(JsonMap::new());
                    match &mut log_event.attributes {
                        Value::Object(m) => m,
                        _ => unreachable!(),
                    }
                }
            };
            if let Some(f) = visitor.file {
                map.entry("_file".to_string()).or_insert(Value::String(f));
            }
            if let Some(l) = visitor.line {
                map.entry("_line".to_string()).or_insert(Value::from(l));
            }
        }

        // 向上回溯 span 链: 合并每一层的 Attribution 和 ScopeExtra 到当前事件。
        // 这让 attribution_span! / scope_span! 宏定义的上下文自动应用到内部所有事件。
        // 注意 merge_from 是 "已有键不覆盖", 所以最内层 (最近) span 的归因优先。
        if let Some(span_ref) = ctx.lookup_current() {
            let mut current = Some(span_ref);
            while let Some(span) = current {
                let exts = span.extensions();
                if let Some(parent) = exts.get::<Attribution>() {
                    log_event.attribution.merge_from(parent);
                }
                if let Some(scope_extra) = exts.get::<ScopeExtra>() {
                    if log_event.attributes.is_null() {
                        log_event.attributes = Value::Object(scope_extra.extra.clone());
                    } else if let Value::Object(map) = &mut log_event.attributes {
                        for (k, v) in &scope_extra.extra {
                            map.entry(k.clone()).or_insert_with(|| v.clone());
                        }
                    }
                }
                drop(exts);
                current = span.parent();
            }
        }

        // 兜底 trace_id: 若调用方在 attributes 里写了 "trace_id", 提升到顶层字段。
        // 用于跨进程关联 (子进程把父进程的 trace_id 写进 attrs 透传)。
        if log_event.trace_id.is_none()
            && let Some(tid) = log_event.attributes.get("trace_id").and_then(Value::as_str)
        {
            log_event.trace_id = Some(tid.to_string());
        }
        record_event(log_event);
    }
}
/// attribution_span! 宏的字段收集器 -- span 创建时 Visit 出归因字段。
///
/// `attribution_span!` 宏的调用形式 (概念):
/// ```text
/// attribution_span!(alias = "assistant", role_family = "agent");
/// attribution_span!(composite = "channel", role_type = "feishu", alias = "default");
/// attribution_span!(field = "tool", alias = "shell");
/// ```
///
/// 收集到的字段在 `apply_info` 里组装为 `Attribution`。
/// 只处理字符串字段 -- `record_debug` 走 `todo!()` (FIXME: 见下方)。
#[derive(Default)]
struct AttributionSpanCollector {
    /// 顶层角色类别, 写入 sd_role 字段 (如 "agent" / "tool" / "channel")。
    role_family: Option<String>,
    /// 复合归因的 type 部分 (如 "feishu"), 与 alias 组合成 "feishu.default"。
    role_type: Option<String>,
    /// 标量归因字段名 (如 "tool" / "agent_alias"), 见 ATTRIBUTION_FIELDS。
    attribution_field: Option<String>,
    /// 复合前缀 (如 "channel" / "model_provider"), 见 COMPOSITE_PREFIXES。
    composite_prefix: Option<String>,
    /// span 默认 category -- 不属于归因, 但随 span 一起收集, 单独走 SpanCategory 通道。
    default_category: Option<String>,
    /// 角色别名 -- 归因的核心值 (如 "assistant" / "default" / "shell")。
    alias: Option<String>,
}

const F_ROLE_FAMILY: &str = "sd_role_family";
const F_ROLE_TYPE: &str = "sd_role_type";
const F_ATTRIBUTION_FIELD: &str = "sd_attribution_field";
const F_COMPOSITE_PREFIX: &str = "sd_composite_prefix";
const F_DEFAULT_CATEGORY: &str = "sd_default_category";
const F_ALIAS: &str = "sd_alias";

impl AttributionSpanCollector {
    /// 把收集到的字段写入 Attribution。
    ///
    /// 分支:
    /// 1. composite_prefix 非空 -> set_composite(prefix, "type.alias") 或 set_composite(prefix, alias)
    /// 2. 否则 attribution_field 非空 -> set(field, alias)
    /// 3. 两种都没填则跳过 (只有 role_family 时归因仍然写入 sd_role)
    ///
    /// role_family 独立处理 -- 它不依赖 alias 是否存在。
    fn apply_info(&self, attr: &mut Attribution) {
        let Some(alias) = self.alias.as_deref().filter(|s| !s.is_empty()) else {
            return;
        };
        if let Some(prefix) = self.composite_prefix.as_deref().filter(|s| !s.is_empty()) {
            let ty = self.role_type.as_deref().unwrap_or("");
            if !ty.is_empty() {
                attr.set_composite(prefix, &format!("{ty}.{alias}"));
            } else {
                attr.set_composite(prefix, alias);
            }
        } else if let Some(field) = self.attribution_field.as_deref().filter(|s| !s.is_empty()) {
            attr.set(field, alias);
        }

        if let Some(family) = self.role_family.as_deref().filter(|s| !s.is_empty()) {
            attr.set(SHADOW_ROLE, family);
        }
    }

    fn put(&mut self, name: &str, value: &str) {
        match name {
            F_ROLE_FAMILY => self.role_family = Some(value.to_string()),
            F_ROLE_TYPE => self.role_type = Some(value.to_string()),
            F_ATTRIBUTION_FIELD => self.attribution_field = Some(value.to_string()),
            F_COMPOSITE_PREFIX => self.composite_prefix = Some(value.to_string()),
            F_DEFAULT_CATEGORY => self.default_category = Some(value.to_string()),
            F_ALIAS => self.alias = Some(value.to_string()),
            _ => {}
        }
    }
}

impl Visit for AttributionSpanCollector {
    /// FIXME: `todo!()` 会在任何 Debug 字段出现时 panic。
    ///
    /// attribution_span! 宏目前只发字符串字段, 所以生产路径上不会命中。
    /// 但若未来宏扩展支持 Debug 记录 (如 `?`-格式参数), 这里会直接崩。
    /// 建议改成 no-op 或写入 String 化后丢弃。
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        let mut buf = String::new();
        let _ = write!(&mut buf, "{value:?}");
        let trimmed = strip_outer_quotes(&buf);
        self.put(field.name(), &trimmed)
    }
}

/// span extensions 里的 category 载体 -- on_event 时向上回溯 span 链查找它。
/// newtype 包一层是因为 trait object 要求具体类型, 不能直接塞枚举。
#[derive(Clone, Copy)]
struct SpanCategory(EventCategory);

/// scope_span! 宏的字段收集器 -- 与 AttributionSpanCollector 类似, 但支持更多元数据:
/// 1. 显式 category (attribution_span 没有)
/// 2. 归因字段 (与 attribution_span 同语义, 走 Attribution)
/// 3. 自由属性 extra (非协议字段, 进 ScopeExtra 而非归因)
#[derive(Default)]
struct ScopeSpanCollector {
    category: Option<String>,
    attribution: Attribution,
    extra: JsonMap<String, Value>,
}

/// scope_span! 的自由属性载体 -- on_event 时合并进 LogEvent.attributes。
/// 与 LogEvent.attributes 的区别: ScopeExtra 存在 span extensions 上,
/// 向上回溯时逐层合并到事件的 attributes。
#[derive(Default)]
struct ScopeExtra {
    extra: JsonMap<String, Value>,
}

impl ScopeSpanCollector {
    /// 把收集到的字段挂到 span extensions 上 (3 类, 各自独立合并):
    ///
    /// 1. **Attribution**: 若非空, 与已有 Attribution merge_from (已有键优先)。
    /// 2. **ScopeExtra**: 若非空, 与已有 ScopeExtra 逐键 or_insert (已有键优先)。
    /// 3. **SpanCategory**: 若解析成功, 覆盖式插入 (不 merge, 后写胜出)。
    ///
    /// 语义: scope_span! 嵌套时, 最内层的归因 / 自由属性优先, 但 category 后写覆盖。
    fn install<'a>(self, span: SpanRef<'a, impl Subscriber + for<'l> LookupSpan<'l>>) {
        if !self.attribution.fields.is_empty() || self.attribution.duration_ms.is_some() {
            let mut exts = span.extensions_mut();
            if let Some(exist) = exts.get_mut::<Attribution>() {
                exist.merge_from(&self.attribution)
            } else {
                exts.insert(self.attribution);
            }
        }

        if !self.extra.is_empty() {
            let mut exts = span.extensions_mut();
            if let Some(exist) = exts.get_mut::<ScopeExtra>() {
                for (k, v) in self.extra {
                    exist.extra.entry(k).or_insert(v);
                }
            } else {
                exts.insert(ScopeExtra { extra: self.extra });
            }
        }

        if let Some(cat) = self.category.as_deref().and_then(EventCategory::parse) {
            span.extensions_mut().insert(SpanCategory(cat))
        }
    }


    fn put(&mut self, name: &str, value: Value) {
        if name == "category" {
            if let Value::String(v) = value {
                self.category = Some(v);
            }
            return;
        }

        for prefix in COMPOSITE_PREFIXES {
            if name == *prefix && let Value::String(v) = &value {
                if v.contains(".") {
                    self.attribution.set_composite(prefix, v);
                } else {
                    self.attribution.set(format!("{prefix}_type"), v.clone());
                }
                return;
            }
        }

        if ATTRIBUTION_FIELDS.contains(&name) && let Value::String(v) = value {
            self.attribution.set(name, v);
            return;;
        }
        self.extra.insert(name.to_string(), value);
    }
}

impl Visit for ScopeSpanCollector {
    fn record_f64(&mut self, field: &Field, value: f64) {
      self.put(field.name(), serde_json::Number::from_f64(value).map(Value::Number).unwrap_or(Value::Null));
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.put(field.name(), Value::from(value))
    }

    fn record_u64(&mut self, field: &Field, value: u64) {

        if field.name() == "duration_ms" {
            self.attribution.duration_ms = Some(value);
            return;
        }

        self.put(field.name(), Value::from(value))
    }


    fn record_bool(&mut self, field: &Field, value: bool) {
        self.put(field.name(), Value::Bool(value))
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.put(field.name(), Value::String(value.to_string()))
    }

    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        let mut buf = String::new();
        let _ = write!(&mut buf, "{value:?}");
        self.put(field.name(), Value::String(strip_outer_quotes(&buf)));
    }
}

/// record! 宏事件的字段收集器 -- on_event 时 Visit 出 sd_* 字段。
///
/// record! 宏把所有字段都作为 tracing 字段传出, 这里的每个 Option 对应一个 sd_* 字段。
/// `extra` 兜底: 不在 sd_* 白名单里的字段进自由属性 (on_event 时合并进 attributes)。
#[derive(Default)]
struct EventCollector {
    name: Option<String>,
    action: Option<String>,
    outcome: Option<String>,
    category: Option<String>,
    /// record! 宏的归因字段 (JSON 字符串, on_event 时 parse 回 Value 覆盖 attributes)
    attrs: Option<String>,
    has_duration: Option<bool>,
    duration_ms: Option<u64>,
    file: Option<String>,
    line: Option<u64>,
    message: Option<String>,
    /// 非协议字段 (不以 sd_ 开头), 合并进 attributes 的 _* 命名空间。
    extra: JsonMap<String, Value>,
}

// sd_* 字段名常量 -- record! 宏传出的字段名必须与这些精确匹配。
// 非匹配字段 (不以 sd_ 开头或不在此列表) 走 EventCollector.extra -> attributes。
const F_NAME: &str = "sd_name";
const F_ACTION: &str = "sd_action";
const F_OUTCOME: &str = "sd_outcome";
const F_CATEGORY: &str = "sd_category";
const F_ATTRS: &str = "sd_attrs";
/// 注意: 拼写为 "sd_das_duration" (疑似 typo, 应为 sd_has_duration),
/// 但因为宏端也用同样的拼写, 实际功能正常。改名需同步宏端。
const F_HAS_DURATION: &str = "sd_das_duration";
const F_DURATION_MS: &str = "sd_duration_ms";
const F_FILE: &str = "sd_file";
const F_LINE: &str = "sd_line";
const F_MESSAGE: &str = "sd_message";

impl EventCollector {
    /// 按 sd_* 字段名分发 value 到对应 Option 字段。
    ///
    /// 设计: 宽松解析 -- 每个分支都做类型匹配, 类型不符 (如传 String 给 sd_line)
    /// 时静默丢弃而非 panic。这让 record! 宏调用方不必关心 tracing 的类型推断。
    ///
    /// `_ =>` 兜底: 不在白名单的字段名直接进 extra, 后续合并到 attributes。
    ///
    /// 数值字段 (duration_ms / line) 同时接受 Number 和 String -- 因为 tracing
    /// record_str 时会走 String 路径, record_i64/u64 时走 Number 路径。
    fn put(&mut self, name: &str, value: Value) {
        match name {
            F_NAME => {
                if let Value::String(s) = value {
                    self.name = Some(s)
                }
            }
            F_ACTION => {
                if let Value::String(s) = value {
                    self.action = Some(s);
                }
            }
            F_OUTCOME => {
                if let Value::String(s) = value {
                    self.outcome = Some(s);
                }
            }
            F_CATEGORY => {
                if let Value::String(s) = value {
                    self.category = Some(s);
                }
            }

            F_ATTRS => {
                if let Value::String(s) = value {
                    self.attrs = Some(s);
                }
            }

            // duration_ms / line 双路径: Number (record_u64/i64) 或 String (record_str)
            F_DURATION_MS => {
                if let Value::Number(s) = &value
                    && let Some(u) = s.as_u64()
                {
                    self.duration_ms = Some(u);
                } else if let Value::String(s) = value
                    && let Ok(u) = s.parse::<u64>()
                {
                    self.duration_ms = Some(u);
                }
            }
            F_HAS_DURATION => {
                if let Value::Bool(b) = value {
                    self.has_duration = Some(b);
                } else if let Value::String(s) = value {
                    self.has_duration = Some(s == "true");
                }
            }

            F_MESSAGE => {
                if let Value::String(s) = value {
                    self.message = Some(s);
                }
            }

            F_FILE => {
                if let Value::String(s) = value {
                    self.file = Some(s);
                }
            }

            F_LINE => {
                if let Value::Number(s) = &value
                    && let Some(u) = s.as_u64()
                {
                    self.line = Some(u);
                } else if let Value::String(s) = value
                    && let Ok(u) = s.parse::<u64>()
                {
                    self.line = Some(u);
                }
            }
            // 非协议字段 -- 后续合并到 LogEvent.attributes
            _ => {
                self.extra.insert(name.to_string(), value);
            }
        }
    }
}

/// tracing::field::Visit trait 实现 -- tracing 按值的 Rust 类型分发到对应方法。
///
/// 每个方法把 tracing 值转成 serde_json::Value, 再委托给 EventCollector::put。
/// 注意几个特例:
/// - record_u64 / record_bool 对 sd_duration_ms / sd_has_duration 直接短路,
///   因为 put 的通用 match 路径对这些字段没有比直接赋值更高效。
/// - record_debug 是兜底路径: tracing 的 `%message` 走 record_str, 但
///   `message = "..."` 字面量走 record_debug (带外层引号), 需要 strip_outer_quotes。
impl Visit for EventCollector {
    fn record_f64(&mut self, field: &Field, value: f64) {
        self.put(
            field.name(),
            serde_json::Number::from_f64(value)
                .map(Value::Number)
                .unwrap_or(Value::Null),
        )
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.put(field.name(), Value::from(value))
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        if field.name() == F_DURATION_MS {
            self.duration_ms = Some(value);
            return;
        }
        self.put(field.name(), Value::from(value));
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        if field.name() == F_HAS_DURATION {
            self.has_duration = Some(value);
            return;
        }
        self.put(field.name(), Value::Bool(value));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.put(field.name(), Value::String(value.to_string()))
    }

    /// error 字段特殊处理: 链式输出整个错误链 (error.source() 逐级向下)。
    /// 格式: "top error: cause1: cause2: ..."
    fn record_error(&mut self, field: &Field, value: &(dyn Error + 'static)) {
        let mut buf = String::new();
        let _ = write!(&mut buf, "{value}");

        let mut current = value.source();

        while let Some(src) = current {
            let _ = write!(&mut buf, ": {src}");
            current = src.source();
        }
        self.put(field.name(), Value::String(buf));
    }

    /// Debug 兜底: 处理 record_str / record_bool 等不走的情况
    /// (如 `message = "..."` 字面量 tracing 会以 Debug 格式记录)。
    ///
    /// F_MESSAGE 特殊: Debug 格式会给字符串加外层引号, 需要 strip_outer_quotes 去掉。
    /// F_HAS_DURATION 特殊: Debug 格式的 bool 是 "true"/"false" 字符串。
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        let mut buf = String::new();
        let _ = write!(&mut buf, "{value:?}");
        if field.name() == F_MESSAGE {
            self.message = Some(strip_outer_quotes(&buf));
            return;
        }
        if field.name() == F_HAS_DURATION {
            self.has_duration = Some(buf == "true");
            return;
        }
        self.put(field.name(), Value::String(buf));
    }
}

/// 去掉 Debug 格式化带来的外层双引号。
///
/// tracing 把 `message = "hello"` 记录为 Debug `"hello"` (含引号),
/// 直接写入会变成 `"\"hello\""`, 这里去掉外层引号还原原始字符串。
fn strip_outer_quotes(s: &str) -> String {
    let trimmed = s.trim();
    if trimmed.starts_with('"') && trimmed.ends_with('"') && trimmed.len() >= 2 {
        return trimmed[1..trimmed.len() - 1].to_string();
    }
    trimmed.to_string()
}
