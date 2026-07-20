# Shadow 日志系统设计文档

> 基于 `crates/shadow-log/` 的 `event.rs` / `layer.rs` / `observer_bridge.rs` 三大核心文件整理。
> 这三个文件共同构成 Shadow 的结构化日志流水线: 数据模型 → 采集 → 双通道输出。

## 1. 角色定位

`shadow-log` 是 Shadow 进程内**唯一的结构化日志面**。它解决三个问题:

1. **统一发射点** -- 所有子系统通过 `record!` 宏发事件, 不直接调 tracing/log
2. **结构化 schema** -- 每条日志是一条 JSONL, 含归因上下文, 可被查询/聚合
3. **双通道消费** -- 同一份事件, 持久化通道 (JSONL 文件 + broadcast) 与指标通道 (Observer trait) 并行消费

借鉴 ZeroClaw 的 record! 设计, 但大幅精简: 7 文件, 简化 schema。

## 2. 整体架构

### 2.1 数据流

```
┌─────────────────────────────────────────────────────────────────┐
│  业务代码 (agent / tool / channel / memory / provider / ...)    │
│                                                                 │
│    record!(INFO, Action::Send, "LLM 请求", "model", "gpt-4o")  │
│    attribution_span!(agent)                                     │
└──────────────────────────┬──────────────────────────────────────┘
                           │ tracing::event! / tracing::span!
                           ▼
┌─────────────────────────────────────────────────────────────────┐
│  tracing_subscriber                                             │
│  ┌─────────────────────┐  ┌──────────────────────────────────┐  │
│  │ fmt::layer (stderr) │  │ LogCaptureLayer (shadow-log)     │  │
│  │ 全局 EnvFilter      │  │ 独立 Targets filter              │  │
│  └─────────────────────┘  │  - shadow_log_event=INFO         │  │
│                           │  - shadow_log_attribution=INFO   │  │
│                           └──────────────┬───────────────────┘  │
└──────────────────────────────────────────┼──────────────────────┘
                                           │ on_event / on_new_span
                                           ▼
┌─────────────────────────────────────────────────────────────────┐
│  layer.rs (采集层)                                              │
│                                                                 │
│  1. Visit 出 sd_* 字段                                          │
│  2. 向上回溯 span 链合并 Attribution                            │
│  3. 组装 LogEvent                                               │
└──────────────────────────┬──────────────────────────────────────┘
                           │ record_event(LogEvent)
                           ▼
┌─────────────────────────────────────────────────────────────────┐
│  writer.rs (持久化)              observer_bridge.rs (投影)      │
│                                                                 │
│  ┌─────────────────────────┐     ┌──────────────────────────┐   │
│  │ JSONL 文件              │     │ LogEvent → ObserverEvent │   │
│  │ (~/.shadow/state/       │     │                          │   │
│  │  runtime-trace.jsonl)   │     │ Observer::record_event() │   │
│  │                         │     │                          │   │
│  │ broadcast 多读端        │     │ → TUI / Prometheus /     │   │
│  │ trim/rotate/retention   │     │   OTel exporter          │   │
│  └─────────────────────────┘     └──────────────────────────┘   │
└─────────────────────────────────────────────────────────────────┘
```

### 2.2 三大核心文件分工

| 文件 | 角色 | 输入 | 输出 |
|------|------|------|------|
| `event.rs` | **数据 schema** | -- | LogEvent / Severity / Action / Attribution 等类型定义 |
| `layer.rs` | **采集层** | tracing Event / span | LogEvent (交给 writer) |
| `observer_bridge.rs` | **观察者桥接** | LogEvent | ObserverEvent (交给 Observer trait) |

辅助文件:
- `macro.rs` -- `record!` / `attribution_span!` / `scope!` 宏定义
- `writer.rs` -- JSONL 落盘 + trim/rotate + broadcast
- `broadcast.rs` -- mpsc 广播通道
- `config.rs` -- 日志配置 schema
- `lib.rs` -- `install_subscriber` 装配全局 subscriber

---

## 3. 数据 Schema (event.rs)

### 3.1 LogEvent -- 一行 JSONL 长这样

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "@timestamp": "2026-07-20T10:30:45.123Z",
  "severity_number": 9,
  "severity_text": "INFO",
  "event": {
    "category": "Agent",
    "action": "agent_start",
    "outcome": "success"
  },
  "service": {"name": "shadow", "version": "0.3.0"},
  "trace_id": "abc123",
  "span_id": null,
  "attribution": {
    "fields": {"agent_alias": "assistant", "model": "gpt-4o"},
    "duration_ms": 1200
  },
  "message": "starting agent",
  "attributes": {"input_tokens": 1500, "_file": "agent.rs", "_line": 42},
  "schema_version": 1
}
```

字段命名混合业界标准:

| 字段族 | 来源 | 说明 |
|--------|------|------|
| `@timestamp` | ECS | `@` 前缀便于 Elasticsearch 索引识别 |
| `severity_number` / `severity_text` | OTel Logs Data Model | 数字编码 1/5/9/13/17, 间隔 4 留空可细分 |
| `event.category` / `event.action` / `event.outcome` | ECS | 顶层事件描述符 |
| `service.name` / `service.version` | ECS | 进程标识 |
| `trace_id` / `span_id` | OTel | 跨进程链路关联 |
| `attribution` | Shadow 自定义 | "是谁、对什么、用了多久" |
| `attributes` | OTel AnyValue | 自由属性兜底 |
| `schema_version` | Shadow | 当前固定 1, 未来字段迁移用 |

### 3.2 Severity -- 数字编码

```rust
pub enum Severity { Trace, Debug, Info, Warn, Error }
```

数字编码采用 OTel 风格的"留空可细分":

| Variant | number() | text() |
|---------|----------|--------|
| Trace | 1 | "TRACE" |
| Debug | 5 | "DEBUG" |
| Info | 9 | "INFO" |
| Warn | 13 | "WARN" |
| Error | 17 | "ERROR" |

间隔 4 -- 便于未来插入 sub-level (如 Trace2=2, Trace3=3) 而不破坏顺序。
落盘后查询时用 `severity_text_from_number` 把 0..20 还原成文本, >20 归为 FATAL。

### 3.3 EventCategory -- 9 类子系统标识

```rust
pub enum EventCategory {
    Agent, Channel, Tool, Provider, Memory, Session, System, Cron, Internal,
}
```

用于过滤/聚合: 例如"只看 Agent 事件"或"统计 Memory 子系统失败率"。
对应 Shadow 的核心 trait / 资源, 外加 System (运行时自身) 与 Internal (日志框架自身, 通常被过滤掉)。

### 3.4 EventOutcome -- 三态

```rust
pub enum EventOutcome { Success, Failure, Unknown }
```

刻意不用 bool: 区分"未观察到" (Unknown) 与"成功" (Success)。
Unknown 是默认值, 序列化时被 `skip_serializing_if = is_unknown_outcome` 去掉。

### 3.5 Action -- 36 个封闭枚举

```rust
pub enum Action {
    // 生命周期
    Start, Complete, Fail, Cancel, Skip, Timeout, Retry,
    // 通信方向
    Inbound, Outbound, Send, Receive,
    // 连接
    Connect, Disconnect, Reconnect,
    // 进程管理
    Spawn, Kill,
    // 调度
    Tick, Trigger, Schedule,
    // 审批
    Approve, Reject, Defer,
    // CRUD
    Read, Write, Delete, List, Query,
    // 调用
    Invoke, Dispatch, Resolve,
    // 注册
    Register, Unregister,
    // 数据
    Load, Save, Migrate, Validate,
    // 元事件
    Note,
}
```

**设计原则: 刻意没有 `Other` 逃逸分支** (借鉴 ZeroClaw)。
不允许 "Other" 让事件悄悄游离协议。新动作必须显式加入枚举, 强制审视语义类别, 保证 `event.action` 字段可被聚合。

新增 Action 时需同步更新 `observer_bridge.rs::project` 的 action 分发表。

---

## 4. 采集层 (layer.rs)

### 4.1 角色

`LogCaptureLayer` 实现 `tracing_subscriber::Layer`, 是 `record!` 宏与 `writer.rs` 之间的桥梁。

**四个职责:**

1. 识别 3 类受控 target (`log_event` / `log_attribution` / `log_scope`) 的 span/event
2. 用 `Visit` trait 把 tracing 字段值提取为强类型字段 (sd_* 约定)
3. 把 span 上挂载的归因上下文向上回溯, 合并到当前事件
4. 组装成 `LogEvent` 后交给 writer 落盘

### 4.2 三类 target 的分工

| target | 由谁发出 | 用途 |
|--------|----------|------|
| `log_event` | `record!` 宏的事件 | 一次具体动作, 落盘为一条 LogEvent |
| `log_attribution` | `attribution_span!` 宏 | 把归因字段绑定到 span, 所有内部事件自动继承 |
| `log_scope` | `scope_span!` 宏 | 同上, 额外支持 category / 自由 attributes |
| `log_internal*` | 日志框架自身 | **抑制** -- 避免反馈循环 |

**抑制前缀的目的:** 落盘失败 → warn → 再次尝试落盘 → 再次失败。`log_internal` 前缀的事件被 layer.rs 跳过, 避免这种循环。

### 4.3 on_event 主路径 (9 步)

```rust
fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
    // 1. 抑制 log_internal 前缀 (反馈循环防护)
    if target.starts_with(TARGET_SUPPRESS_PREFIX) { return; }

    // 2. Visit 出 sd_* 字段
    let mut visitor = EventCollector::default();
    event.record(&mut visitor);

    // 3. 决定 action: 显式 sd_action > tracing 事件名
    let action_str = visitor.action.as_deref()
        .map(str::to_string)
        .unwrap_or_else(|| metadata.name().to_string());

    // 4. 决定 category: 显式 > span 链上的 SpanCategory > 默认 Internal
    let category = visitor.category.as_deref()
        .and_then(EventCategory::parse)
        .or_else(|| /* 遍历 span 链找 SpanCategory */)
        .unwrap_or(EventCategory::Internal);

    // 5. 决定 name: sd_name > action_str
    let name = visitor.name.as_deref().unwrap_or(action_str.as_str()).to_string();
    let mut log_event = LogEvent::new(severity, &name, category);

    // 6. 处理 outcome / message / duration / attrs / extra / file+line
    if target == TARGET_EVENT { log_event.event.action = action_str; }
    log_event.message = Some(visitor.message.unwrap_or_default());
    if visitor.has_duration.unwrap_or(false) {
        log_event.attribution.duration_ms = visitor.duration_ms;
    }
    // sd_attrs 反序列化为 Value 整体覆盖 attributes
    // extra (非 sd_ 字段) 合并进 attributes, 已有键保留
    // file/line 以 _file/_line 前缀存进 attributes

    // 7. 向上回溯 span 链: 合并每一层的 Attribution 和 ScopeExtra
    //    merge_from 是 "已有键不覆盖", 最内层 (最近) span 优先
    while let Some(span) = current {
        if let Some(parent) = span.extensions().get::<Attribution>() {
            log_event.attribution.merge_from(parent);
        }
        if let Some(scope_extra) = span.extensions().get::<ScopeExtra>() {
            // 合并 scope_extra.extra 到 log_event.attributes
        }
        current = span.parent();
    }

    // 8. 兜底 trace_id: 若 attributes 里有 "trace_id" 字段, 提升到顶层
    if log_event.trace_id.is_none() {
        if let Some(tid) = log_event.attributes.get("trace_id").and_then(Value::as_str) {
            log_event.trace_id = Some(tid.to_string());
        }
    }

    // 9. 调 record_event 落盘
    record_event(log_event);
}
```

### 4.4 sd_* 字段命名约定

`record!` 宏传出的字段统一用 `sd_` 前缀 (shadow), 见 `EventCollector::put`:

| 常量 | 字段名 | 类型 | 说明 |
|------|--------|------|------|
| F_NAME | `sd_name` | String | 事件显示名 |
| F_ACTION | `sd_action` | String | 覆盖 event.action |
| F_OUTCOME | `sd_outcome` | String | "success"/"failure"/"unknown" |
| F_CATEGORY | `sd_category` | String | 覆盖 category |
| F_ATTRS | `sd_attrs` | String (JSON) | 自由属性, 反序列化后整体覆盖 attributes |
| F_HAS_DURATION | `sd_das_duration` | Bool | 是否采纳 duration_ms (拼写待对齐) |
| F_DURATION_MS | `sd_duration_ms` | u64 | 归因里的耗时 |
| F_FILE | `sd_file` | String | 源码文件 |
| F_LINE | `sd_line` | u64 | 源码行号 |
| F_MESSAGE | `sd_message` | String | 事件 message |

**非 sd_ 前缀字段** 被视为"自由属性", 进 `attributes` 而非归因。

### 4.5 两个 Visit 收集器

**EventCollector** -- 收集 `record!` 事件字段:

```rust
struct EventCollector {
    name: Option<String>,
    action: Option<String>,
    outcome: Option<String>,
    category: Option<String>,
    attrs: Option<String>,           // JSON 字符串
    has_duration: Option<bool>,
    duration_ms: Option<u64>,
    file: Option<String>,
    line: Option<u64>,
    message: Option<String>,
    extra: JsonMap<String, Value>,   // 非 sd_ 字段兜底
}
```

`put` 方法按 sd_* 字段名分发, 类型不符静默丢弃。`_ =>` 兜底分支把非协议字段塞进 `extra`, 后续合并到 `LogEvent.attributes`。

**AttributionSpanCollector / ScopeSpanCollector** -- 收集 span 字段, 挂到 span extensions 上, on_event 时回溯读取。

### 4.6 span 链回溯的归因继承

`attribution_span!` / `scope_span!` 把归因字段绑定到 span。当内部事件触发时, layer.rs 向上遍历 span 链, 逐层合并 Attribution:

```rust
// 嵌套示例
attribution_span!(agent)         // agent_alias="assistant"
  attribution_span!(tool)        // tool="shell"
    record!(...)                 // 自动继承 agent_alias + tool
```

合并语义: `merge_from` 是"已有键不覆盖", 所以**最内层 (最近) span 的归因优先**。

---

## 5. 观察者桥接 (observer_bridge.rs)

### 5.1 为什么需要桥接

Shadow 的事件流有两个独立消费者:

| 通道 | 数据结构 | 用途 |
|------|----------|------|
| LogEvent | ECS/OTel 风格, 自由 action 字符串 | 文件落盘 / 查询 / 排查 |
| ObserverEvent | 封闭枚举, ~20 个变体, 字段类型严格 | 实时指标 / TUI 面板 / Prometheus |

同一份 `record!` 调用, 两个消费者各自拿到适合自己场景的数据结构。`observer_bridge.rs` 负责投影:

```
LogEvent ──project──> Option<ObserverEvent>
                         │
                         │ Some(evt)
                         ▼
                     Observer::record_event(&evt)
```

### 5.2 全局 observer slot

```rust
static OBSERVER: OnceLock<RwLock<Option<Arc<dyn Observer>>>> = OnceLock::new();
```

- `OnceLock` 保证线程安全一次性初始化
- `RwLock` 允许运行时切换 observer
- 未绑定时 `forward` no-op, 让本 crate 在没有 observer 的场景下也能无副作用运行

### 5.3 投影规则 (action 分发表)

`project` 函数按 `event.action` 字符串分发到 ObserverEvent 变体:

| action | ObserverEvent 变体 | 备注 |
|--------|---------------------|------|
| `agent_start` | AgentStart | |
| `agent_end` | AgentEnd | 含 token / cost 提取 |
| `llm_request` | LlmRequest | |
| `llm_response` | LlmResponse | 含 token / error |
| `tool_call_start` | ToolCallStart | tool_call_id / arguments 未填 |
| `tool_call` / `tool_call_result` | ToolCall | 二者合并到同一变体 |
| `channel_message_inbound` | ChannelMessage | direction="inbound" |
| `channel_send` | ChannelMessage | direction="outbound" |
| `turn_complete` | TurnComplete | |
| `heartbeat_tick` | HeartbeatTick | |
| `error` | Error | component 取 channel_type |
| 其他 | None | 未识别 action 不转发 |

### 5.4 字段归一化

`project` 把字符串字段统一转 `Option<String>` (空串 → None), 让 ObserverEvent 的字段语义清晰:

```rust
let channel_opt = if channel.is_empty() { None } else { Some(channel.clone()) };
```

None = "未提供", Some("") 没有意义, 故统一转 None。

---

## 6. 宏调用面 (record! / attribution_span!)

### 6.1 record!

```rust
// 基本形式
record!(INFO, Action::Start, "starting agent");

// 带归因字段 (key-value 对)
record!(INFO, Action::Send, "LLM 请求", "model", "gpt-4o", "agent", "shadow");

// 带 outcome
record!(WARN, Action::Fail.with_outcome(EventOutcome::Failure), "tool failed");
```

宏展开为 `tracing::event!(target: "shadow_log_event", ...)`, 字段名:
- `shadow_action = %action.as_str()` -- 动作字符串
- `message = %$msg` -- 消息
- `shadow_attrs = %attrs_json` -- 归因字段 JSON (仅第二形式)

### 6.2 attribution_span!

```rust
let _span = attribution_span!(agent).entered();
// 在此 span 作用域内, 所有 record! 自动继承 agent 的归因
```

宏从 `Attributable` 对象自动填充 `role` / `field` / `alias`, 展开为 `tracing::info_span!(target: "shadow_log_attribution", ...)`。

### 6.3 scope!

```rust
let result = scope!(key1: value1, key2: value2 => async_body).await;
```

把自由属性绑定到异步体的执行 span, target 是 `shadow_log_internal_scope`。

---

## 7. 归因系统

### 7.1 两类归因字段

**标量归因** (`ATTRIBUTION_FIELDS`, 15 个) -- 单值字段:

```rust
pub const ATTRIBUTION_FIELDS: &[&str] = &[
    "agent_alias", "tool", "session_key", "cron_job_id",
    "risk_profile", "runtime_profile",
    "memory_namespace", "skill_bundle", "knowledge_bundle", "mcp_bundle",
    "peer_group", "sop_name",
    "model", "embedding_provider",
    "owner_tui_id",
];
```

每个字段是子系统的"身份指针", 用于关联事件到具体资源。

**复合归因** (`COMPOSITE_PREFIXES`, 5 个) -- 可拆分为 type/alias 的字段:

```rust
pub const COMPOSITE_PREFIXES: &[&str] = &[
    "channel", "model_provider", "tts_provider",
    "transcription_provider", "tunnel_provider",
];
```

每个前缀 P 对应三个字段: `P` / `P_type` / `P_alias`。
例如 `set_composite("channel", "feishu.default")` 一次性写入:

```
channel       = "feishu.default"
channel_type  = "feishu"
channel_alias = "default"
```

### 7.2 Attribution 结构

```rust
pub struct Attribution {
    pub fields: BTreeMap<String, String>,   // 有序输出, diff 友好
    pub duration_ms: Option<u64>,            // 数值单列, 不与 String 混存
}
```

**为何用 BTreeMap 而非 HashMap:** 序列化输出字段名有序, 便于 diff/阅读; 同一事件多次产生时 JSON 顺序稳定。

### 7.3 merge_from 语义

```rust
pub fn merge_from(&mut self, other: &Self) {
    for (k, v) in &other.fields {
        self.fields.entry(k.clone()).or_insert_with(|| v.clone());  // 已有键不覆盖
    }
    if self.duration_ms.is_none() {
        self.duration_ms = other.duration_ms;
    }
}
```

**已有键不覆盖** -- 子 span 的归因优先于父 span。这让 `attribution_span!(tool)` 嵌套在 `attribution_span!(agent)` 内时, 最内层的归因胜出。

---

## 8. 配置

`shadow-log` 的配置走 `LogConfig` (config.rs), 通过 `init_from_config` 初始化全局 writer。

关键配置项:

| 配置 | 类型 | 默认 | 说明 |
|------|------|------|------|
| `log_dir` | PathBuf | `~/.shadow/state/` | JSONL 文件目录 |
| `file_prefix` | String | `"runtime-trace"` | 文件名前缀 |
| `storage` | StoragePolicy | Rolling | None / Full / Rolling / Rotating |
| `max_file_bytes` | u64 | 50MB | Rolling 单文件上限 |
| `max_file_count` | u32 | 10 | Rolling 文件数 |
| `retention_hours` | u64 | 168 (7 天) | 留存时长 |
| `llm_request_payload` | LlmRequestPayloadPolicy | Off | 是否记录 LLM 请求体 (Off/Redacted/Full) |
| `tool_io_truncate_bytes` | usize | 40960 | 工具 IO 截断字节数 |

存储策略:

- **None** -- 不落盘
- **Full** -- 单文件无限增长
- **Rolling** -- 按大小切分, 保留 N 个旧文件
- **Rotating** -- 按时间轮转 (TODO)

---

## 9. 扩展指南

### 9.1 新增 Action

1. 在 `event.rs::Action` 枚举添加变体
2. (若需要 observer 通道) 在 `observer_bridge.rs::project` 添加 action 分支
3. (若需要 record! 宏端便利) 评估是否新增辅助构造方法

```rust
// event.rs
pub enum Action {
    // ...
    MyNewAction,  // 新增
}

// observer_bridge.rs (可选)
match action {
    "my_new_action" => Some(ObserverEvent::MyNewVariant { ... }),
    _ => None,
}
```

### 9.2 新增标量归因字段

```rust
// event.rs
pub const ATTRIBUTION_FIELDS: &[&str] = &[
    // ...
    "my_new_field",  // 新增
];
```

`is_attribution_field` 自动识别, layer.rs 自动归入 `Attribution.fields`。

### 9.3 新增复合归因前缀

```rust
// event.rs
pub const COMPOSITE_PREFIXES: &[&str] = &[
    // ...
    "my_prefix",  // 新增, 自动展开为 my_prefix / my_prefix_type / my_prefix_alias
];
```

### 9.4 新增 ObserverEvent 变体

1. 在 `shadow-core::kennel::observer::ObserverEvent` 添加变体 (`#[non_exhaustive]` 保证外部实现优雅降级)
2. 在 `observer_bridge.rs::project` 添加 action 分支
3. 在 Observer 后端实现 (TUI/Prometheus) 处理新变体

---

## 10. 已知问题 (重要)

### 10.1 target 命名不一致 (严重)

`macro.rs` 发出的事件 target 带 `shadow_` 前缀, 但 `layer.rs` 的常量定义不带前缀:

| 来源 | target 字符串 |
|------|--------------|
| `record!` 宏 | `shadow_log_event` |
| `attribution_span!` 宏 | `shadow_log_attribution` |
| `lib.rs` filter | `shadow_log_event` / `shadow_log_attribution` (与宏一致) |
| `layer.rs` TARGET_EVENT | `log_event` (**不一致**) |
| `layer.rs` SHADOW_ATTRIBUTION_SPAN | `log_attribution` (**不一致**) |
| `layer.rs` SHADOW_SCOPE_SPAN | `log_scope` (**不一致**) |
| `layer.rs` TARGET_SUPPRESS_PREFIX | `log_internal` (**不一致**) |

**影响:**
- filter 通过 `shadow_log_event` 事件到 layer.rs, 但 `on_event` 里 `target == TARGET_EVENT` 永远为 false, action 覆盖逻辑失效
- `on_new_span` 里 `target == SHADOW_ATTRIBUTION_SPAN` 永远为 false, AttributionSpanCollector 从不运行, attribution_span! 的归因绑定完全无效
- 反馈循环防护 (`log_internal` 前缀抑制) 也失效

**修复方向:** 把 layer.rs 的常量改成带 `shadow_` 前缀, 或把宏端改成不带前缀, 二选一统一。

### 10.2 字段名不一致 (严重)

`record!` 宏发出的字段名与 `EventCollector` 期望的字段名不匹配:

| 宏端字段名 | EventCollector 期望 | 是否匹配 |
|-----------|---------------------|---------|
| `shadow_action` | `sd_action` | ❌ |
| `message` | `sd_message` | ❌ |
| `shadow_attrs` | `sd_attrs` | ❌ |

**影响:** 所有 record! 发出的字段都落到 `EventCollector.extra` 兜底分支, 正确字段 (action / message / attrs) 全为 None。最终 LogEvent 的:
- `event.action` 回退到 tracing 事件默认名 (通常是 callsite 信息)
- `message` 为空串
- `attributes` 缺失宏端传入的归因

**修复方向:** 统一字段名, 推荐宏端改为 `sd_*` 前缀以匹配 layer.rs 约定。

### 10.3 scope_span! 宏未实现

`layer.rs` 多处引用 `scope_span!` 宏并处理 `log_scope` target, 但 `macro.rs` 中**没有定义** `scope_span!` 宏 (只有 `scope!`, target 是 `shadow_log_internal_scope`)。

**影响:** layer.rs 的 ScopeSpanCollector 路径永远走不到。

**修复方向:** 实现缺失的 `scope_span!` 宏, 或移除 layer.rs 的相关分支。

### 10.4 F_HAS_DURATION 拼写

`layer.rs` 中:
```rust
const F_HAS_DURATION: &str = "sd_das_duration";  // "das" 应为 "has"
```

疑似 typo, 但因为宏端目前没有正确对齐 sd_* 约定 (见 10.2), 这个字段实际不会命中。修复时一并改为 `sd_has_duration`。

### 10.5 Error 事件 component 字段语义

`observer_bridge.rs::project` 的 Error 分支:
```rust
component: attribution.get(&type_field("channel")).unwrap_or("system").to_string()
```

取的是 `channel_type` 而非完整 `channel`, 会丢失 alias 信息 (如 "feishu.default" 只剩 "feishu")。
属于设计权衡 -- 用 type 作为粗粒度 component 分组, 但若需要精确定位 channel 实例需要调整。

### 10.6 record_u64 duration 短路已修

历史 bug: `self.duration_ms == Some(value)` 用了 `==` (比较) 而非 `=` (赋值)。**已修复**为 `self.duration_ms = Some(value);`。

### 10.7 AttributionSpanCollector record_debug 已修

历史 bug: `record_debug` 用 `todo!()` 会在任何 Debug 字段出现时 panic。**已修复**为正常 Visit 字段后 put。

### 10.8 Observer 桥接调用已修

历史 bug: `observer.on_log_event(event)` 调用的方法不存在于 Observer trait, 应为 `observer.record_event(&obs_event)`。**已修复**。

### 10.9 agent_alias 兜底已修

历史 bug: `ok_or_else` 把 Option 转 Result, 兜底分支永远走不到。**已修复**为 `or_else`, 现在 attribution 与 attributes 双路径生效。

---

## 11. 未来工作

### 11.1 短期 (修 bug)

- [ ] 统一 target 命名 (10.1)
- [ ] 统一 sd_* 字段命名 (10.2)
- [ ] 实现 scope_span! 宏或移除相关分支 (10.3)
- [ ] 修正 F_HAS_DURATION 拼写 (10.4)

### 11.2 中期

- [ ] OTel exporter 实现 (Observer trait 后端)
- [ ] Prometheus exporter 实现
- [ ] Rotating 存储策略 (按时间轮转)
- [ ] 跨进程 trace_id 透传规范

### 11.3 长期

- [ ] schema_version 2 字段迁移机制
- [ ] 日志查询 API (按归因/时间范围/category 过滤)
- [ ] 嵌套 span 的深度限制 (防止恶意嵌套)
- [ ] 异步落盘队列 (当前是同步写, 高频时可能阻塞)

---

## 附录 A: 文件清单

```
crates/shadow-log/
├── Cargo.toml
└── src/
    ├── lib.rs              -- install_subscriber 装配全局 subscriber
    ├── event.rs            -- 数据 schema (LogEvent, Action, Attribution 等)
    ├── layer.rs            -- LogCaptureLayer 采集层
    ├── writer.rs           -- JSONL 落盘 + trim/rotate + broadcast
    ├── broadcast.rs        -- mpsc 广播通道
    ├── observer_bridge.rs  -- LogEvent → ObserverEvent 投影
    ├── config.rs           -- LogConfig / StoragePolicy 等
    └── macro.rs            -- record! / attribution_span! / scope! 宏
```

## 附录 B: 相关文档

- `docs/storage.md` -- 数据库与存储设计 (含 runtime-trace.jsonl 介绍)
- `docs/structure-analysis.md` -- 项目结构分析 (含 install_subscriber 流程)
- `docs/requirements.md` -- 反向需求文档 (从代码推导)
