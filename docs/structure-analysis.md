# Shadow 项目结构分析 (crate → module → trait → impl)

> 生成时间: 2026-07-14
> 范围: 全仓库代码现状梳理, 按 "现状 / 为什么 / 可以怎么优化" 三段式输出

---

## 0. 总览

### 0.1 Workspace 布局

`Cargo.toml` 只把 **6 个 crate** 纳入 workspace members:

| crate | 角色 | LOC | 文件数 |
|---|---|---|---|
| `.` (shadow) | 二进制入口 (CLI) | 455 | 3 |
| `shadow-core` | **trait 层 (微内核 ABI)** | 2306 | 12 |
| `shadow-config` | 配置 schema + 加载 | 2351 | 18 |
| `shadow-log` | 统一日志面 | 781 | 7 |
| `shadow-providers` | LLM provider 实现 | 1517 | 8 |
| `shadow-memory` | 记忆后端实现 | 3703 | 10 |
| `shadow-runtime` | agent loop + 工具集 | 4704 | 32 |

另外有 **10 个"影子 crate"目录** (channels / eval / gateway / hardware / infra / macros / plugins / spawn / tool-call-parser / tools), 每个只有 6 行 `// TODO` 占位, **不在 workspace members 里**, 不参与编译。它们是 ZeroClaw 对照的"未来占位"。

### 0.2 依赖分层 (设计意图)

```
                ┌──────────────────────────────────────┐
                │  shadow (bin)                        │
                │  ┌────────────────────────────────┐  │
                │  │ shadow-runtime (agent loop)    │  │
                │  │   ┌────────────┐ ┌──────────┐  │  │
                │  │   │ providers  │ │ memory   │  │  │
                │  │   └─────┬──────┘ └────┬─────┘  │  │
                │  │         │ config / log │        │  │
                │  └─────────┴──────────────┴────────┘  │
                │            │                          │
                └────────────┼──────────────────────────┘
                             ▼
                ┌────────────────────────┐
                │  shadow-core (trait)   │  ← 零内部依赖, 微内核 ABI
                └────────────────────────┘
```

设计原则 (来自 `shadow-core/src/lib.rs` 注释): **所有 crate 依赖 core, core 不依赖任何内部 crate**。这是经典的 hexagonal/microkernel 布局。

### 0.3 一句话结论

> **trait 层 (shadow-core) 设计成熟且自洽; 但实现层存在严重的"半成品"问题 —— 最致命的是 `shadow-runtime::agent::loop_` 是一个 109 行的 stub, 而真正的 1017 行 agent loop 被遗弃在 `agent_bak.rs` (未挂载到模块树, 不编译)。当前 `shadow agent` 命令实际不会跑 agent loop。**

---

## 1. shadow-core — trait 层 (微内核 ABI)

**职责**: 定义所有扩展点的 trait + 共享值类型, 不含任何业务实现。
**依赖**: 零内部依赖 (只依赖 anyhow / async-trait / serde / futures / strum)。

### 1.1 模块图

```
shadow-core/src/
├── lib.rs              # 重新导出 + AutonomyLevel
├── kennel/             # ★ 核心 trait 全在这
│   ├── mod.rs
│   ├── attribution.rs  # Attributable trait + Role 枚举
│   ├── provider.rs     # ModelProvider trait + Chat* 值类型
│   ├── memory.rs       # Memory trait + MemoryStrategy trait
│   ├── tool.rs         # Tool trait + ToolSpec/ToolResult
│   └── observer.rs     # Observer trait + ObserverEvent
├── channel.rs          # Channel trait
├── runtime.rs          # RuntimeAdapter trait
├── session_store.rs    # SessionStore trait + JsonlSessionStore (唯一内置 impl)
├── workspace.rs        # Workspace 路径布局 (struct, 非 trait)
└── platform.rs         # is_android() 单函数 (4 行)
```

> 注: "kennel" 这个命名 ( kennel = 狗窝, 呼应 "影子/Shadow" 的宠物意象) 把所有核心 trait 收拢到一个子目录, 是个有意的命名统一。

### 1.2 trait 逐个分析

#### 1.2.1 `Attributable` (attribution.rs:11)

```rust
pub trait Attributable: Send + Sync {
    fn role(&self) -> Role;       // 角色家族
    fn alias(&self) -> &str;      // 具体名称
}
```

- **现状**: 回答 "这个操作是谁干的"。`Role` 枚举有 14 个变体 (Agent/Channel/Tool/Provider/Memory/Session/System/Swarm/Cron/PeerGroup/Skill/Mcp/Sop), 每个变体可带子类型 (如 `Provider(ProviderKind::Model(ModelProviderKind::Anthropic))`)。为 `Arc<T>` / `Box<T>` / `&T` 做了 blanket impl。
- **为什么**: 归因是横切关注点 —— 日志、telemetry、计费都要知道"谁干的"。把它做成 super-trait, 让每个参与对象自带身份, 比到处传 `agent_alias: &str` 参数干净。
- **优化**:
  - `Role::family_str()` / `default_category()` / `composite_prefix()` 等一堆 match 是 O(变体数) 的硬编码。**注意: 不能盲目用 `strum::IntoStaticStr` 整体 derive** —— 组合变体 (如 `Provider(ProviderKind::Model(_))` 返回 `"provider.model"`) 的返回值依赖**内层 enum 变体**, strum 只能给外层变体挂一个固定字符串, 直接 derive 会让 `Model`/`Tts`/`Transcription`/`Tunnel` 全部塌缩成 `"provider"`, 数据丢失。正确做法分两层: 叶子变体 (Agent/System 等) 可 derive; 组合变体在内层 enum 上加 `qualifier()` 方法把外层 4 支 match 折成 1 支 (但无法完全消除 match, 因为两个 `&'static str` 不能编译期拼接)。**实际收益有限, 比起 strum 改造, 更建议精简变体本身** (Tts/Transcription/Tunnel 当前都无实现, 可推迟加入)。
  - `ChannelKind` 只有 `AcpChannel` / `Plugin` 两个变体, 没有 Telegram/Discord/CLI —— 说明 Channel 这条线还没真正落地 (与 shadow-channels 占位 crate 呼应)。

#### 1.2.2 `ModelProvider` (provider.rs:215)

```rust
#[async_trait]
pub trait ModelProvider: Attributable {
    fn capabilities(&self) -> ProviderCapabilities { ... }
    async fn chat_with_system(..) -> Result<String>;           // 唯一必须实现
    async fn chat(&self, request: ChatRequest<'_>, ..) -> Result<ChatResponse> { 默认实现 }
    async fn chat_with_tools(..) { 默认实现 }
    fn stream_chat(..) -> BoxStream<..> { 默认空流 }
    // ... 大量默认实现方法 (约 15 个)
}
```

- **现状**: 巨型 trait, **只有 `chat_with_system` 是必须实现的**, 其余 (chat / chat_with_history / chat_with_tools / stream_* / list_models / convert_tools) 全部有默认实现, 默认实现之间互相调用最终落到 `chat_with_system`。配套值类型丰富: `ChatMessage` / `ChatRequest` / `ChatResponse` / `ToolCall` / `TokenUsage` / `StreamChunk` / `StreamEvent` / `ToolsPayload` / `ProviderCapabilities` / `ModelInfo` / `AuthStyle` / `ModelProviderRuntimeOptions`。
- **为什么**: 让"只支持单轮文本"的最简 provider 也能跑 (只实现 `chat_with_system`), 复杂 provider 再覆盖 `chat` / `stream_chat` 拿到完整能力。这是 template-method 模式。`ToolsPayload` 用 enum 区分 Gemini/Anthropic/OpenAI/PromptGuided 四种格式, 适配多厂商。
- **优化** (问题较多):
  1. **`chat()` 默认实现有逻辑 bug 风险** (provider.rs:312-350): 判断 `self.supports_native_tools()` 为 true 时却走 PromptGuided 文本注入分支, 注释和代码矛盾 ("retunred" 拼写错误也在此)。这段分支语义混乱, 需要重写。
  2. **`ChatRequest` 用生命周期 `'a` 借用 messages**, 但 trait 方法又把 `model: &str` / `temperature` 单独传 —— 同一份请求信息被拆成"借用部分 + 内联部分", 调用方负担重。建议把 model/temperature 也收进 `ChatRequest`, 减少参数数量。
  3. **`Arc<T> impl ModelProvider` 的 blanket impl 手写转发所有 15+ 方法** (provider.rs:420-536), 极其冗长且易漏。一旦 trait 加方法, 这里必须同步加, 否则 `Arc<dyn ModelProvider>` 调用会绕过 inner。考虑用 `dyn-compatible` 设计或 `#[async_trait]` 的 Deref 模式替代。
  4. **流式 API 返回 `BoxStream<'static>`**, 但默认实现返回空流 —— 调用方无法区分"不支持流式"和"支持但这次没数据"。`supports_streaming()` 标志和方法行为容易不一致。

#### 1.2.3 `Memory` + `MemoryStrategy` (memory.rs)

```rust
#[async_trait]
pub trait Memory: Attributable {
    fn name(&self) -> &str;
    async fn store(&self, key, content, category, session_id) -> Result<()>;
    async fn recall(&self, query, limit, session_id, since, until) -> Result<Vec<MemoryEntry>>;
    async fn get(&self, key) -> Result<Option<MemoryEntry>>;
    async fn list(&self, category, session_id) -> Result<Vec<MemoryEntry>>;
    async fn forget(&self, key) -> Result<bool>;
    async fn count(&self) -> Result<usize>;
    async fn health_check(&self) -> bool;
    // + store_with_agent / recall_for_agents (必须实现)
    // + 一堆默认 bail!/Ok(0) 的可选方法 (purge_*/export_agent/rename_agent/stats/reindex...)
}

#[async_trait]
pub trait MemoryStrategy: Send + Sync {
    async fn load_context(&self, observer, query, session_id) -> Vec<MemoryEntry>;
    async fn consolidate_turn(&self, user, assistant, session_id) -> Result<()>;
    async fn run_governance(&self) -> Result<()>;
}
```

- **现状**: `Memory` 是后端能力 trait (存/取/删), `MemoryStrategy` 是策略 trait (何时存/取/治理)。值类型非常丰富: `MemoryEntry` (含 id/agent_id/key/content/timestamp/session_id/score/namespace/importance/superseded_by/kind/pinned/tenant_id/category 共 13 字段)、`MemoryCategory` (Core/Daily/Conversation/Custom)、`MemoryKind` (Episodic/Semantic/Procedural)、`StoreOptions` (builder)、`MemoryStats`、`ExportFilter`。
- **为什么**: 把"存储机制" (SQLite vs Markdown) 和"记忆策略" (何时召回/巩固) 正交分离 —— 同一个 strategy 可跑在不同 backend 上。分参数 `store(key, content, category, session_id)` 让调用方不必构造完整 `MemoryEntry`。
- **优化** (这是全项目最"胖"的 trait):
  1. **trait 方法爆炸**: 必须 2 个 + 可选 ~20 个, 可选方法默认实现一半是 `bail!("not supported")` 一半是 `Ok(默认值)`。默认行为不一致 (purge 报错, stats 返回空) 容易让上层踩坑。建议拆成 `Memory` (核心 CRUD) + `MemoryAdmin` (purge/stats/reindex) + `MemoryAgent` (for_agent 系列) 三个子 trait, 按能力组合。
  2. **`store_with_agent` / `recall_for_agents` 是必须实现**, 但 `store` 和 `recall` 也是必须实现 —— 多数实现里 `store_with_agent` 最终调 `store`, 存在重复。可以把 agent 相关方法默认委托到普通方法。
  3. `MemoryEntry` 13 个字段 + `MemoryKind` 嵌套 `SemanticSubType` —— 字段过半在后端里只是"存着不用" (如 `tenant_id` 全项目当前单用户)。按 YAGNI, 可以推迟非必需字段。
  4. `recall` 参数列表 6 个位置参数, 是典型的 long-parameter-list 坏味道, 应封装成 `RecallQuery` struct。

#### 1.2.4 `Tool` (tool.rs:75)

```rust
#[async_trait]
pub trait Tool: Attributable {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters_schema(&self) -> serde_json::Value;
    async fn execute(&self, args: serde_json::Value) -> Result<ToolResult>;
    fn spec(&self) -> ToolSpec { 默认聚合 }
}
```

- **现状**: 4 个必须方法 + 1 个默认 `spec()`。最干净的 trait。配套 `tool_attribution!` 宏自动实现 `Attributable`。
- **为什么**: 每个 tool 自描述 (name/description/schema), agent 把 `spec()` 喂给 LLM 即可。宏减少样板。
- **优化**: `parameters_schema` 返回 `serde_json::Value` 而非强类型, 失去编译期校验。可考虑用 `schemars::JsonSchema` derive 替代手写 schema。其余设计良好, 无需大改。

#### 1.2.5 `Observer` (observer.rs:200)

```rust
#[async_trait]
pub trait Observer: Send + Sync + 'static {
    fn record_event(&self, event: &ObserverEvent);
    fn record_metric(&self, metric: &ObserverMetric);
    fn flush(&self) {}
    fn name(&self) -> &str;
    fn as_any(&self) -> &dyn Any;
}
```

- **现状**: `ObserverEvent` 是 `#[non_exhaustive]` enum, 21 个变体 (AgentStart/LlmRequest/LlmResponse/AgentEnd/ToolCall/MemoryRecall/MemoryStore/RagRetrieve/TurnComplete/ChannelMessage/HeartbeatTick/CacheHit/CacheMiss/Error/Deployment*/RecoveryComplete/HistoryTrimmed)。`ObserverMetric` 6 个变体。有 `Arc<T>` blanket impl。
- **为什么**: `#[non_exhaustive]` 保证外部实现对新变体优雅降级 —— 这是为可观测性扩展预留的。`as_any` 支持 downcast 拿具体类型。
- **优化**:
  1. **`ObserverEvent` 变体字段大量重复** (几乎每个变体都带 `channel / agent_alias / turn_id` 三个 Optional)。建议提取 `struct TurnCtx { channel, agent_alias, turn_id }` 嵌进变体, 减少 boilerplate。
  2. trait 加了 `async_trait` 但 **所有方法都是同步的** (`fn` 不是 `async fn`), `async_trait` 在这里是多余的且带来一次 Box 开销。可以去掉。
  3. `shadow-log` 里有独立的 `LogObserver` trait (observer_bridge.rs:24), **与 core 的 `Observer` 是两套** —— 因为 shadow-log 不能依赖 shadow-core (会循环)。这个分裂需要上层桥接, 当前靠 `forward()` 投影。命名易混淆。

#### 1.2.6 `Channel` (channel.rs:29)

```rust
#[async_trait]
pub trait Channel: Attributable {
    fn name(&self) -> &str;
    async fn send(&self, message: &SendMessage) -> Result<()>;
    fn supports_approval(&self) -> bool { false }
}
```

- **现状**: 3 个方法, 极简。**但全项目零实现** —— `shadow-channels` crate 是 6 行占位, CLI 入口的 channel 注释都是 "暂时不接入"。这是当前最大的"设计有, 实现无"的扩展点。
- **为什么**: trait 先定义好, 留接口。
- **优化**: trait 缺 `receive` / `poll` 方向 —— 当前只有 `send` 出站, 没有入站消息流。等真正接 Telegram/Discord 时必须补 `async fn incoming(&self) -> BoxStream<ChannelMessage>`。建议现在就设计入站接口, 避免日后破坏性改动。

#### 1.2.7 `RuntimeAdapter` (runtime.rs:6)

```rust
pub trait RuntimeAdapter: Send + Sync {
    fn name(&self) -> &str;
    fn has_shell_access(&self) -> bool;
    fn has_filesystem_access(&self) -> bool;
    fn storage_path(&self) -> PathBuf;
    fn supports_long_running(&self) -> bool;
    fn memory_budget(&self) -> u64 { 0 }
    fn build_shell_command(&self, command, workspace_dir) -> Result<Command>;
}
```

- **现状**: 描述运行环境 (docker/native/serverless)。注意是 **普通 trait 不是 async_trait**。
- **为什么**: agent 在不同环境能力不同 (serverless 不能长跑/无 shell), 这个 trait 让运行时自描述。
- **优化**: 实现在 `shadow-config::platform` 里 (native/docker), 但 core 里没有默认实现或测试 stub, 且 trait 名字 `RuntimeAdapter` 与文件名 `runtime.rs` 容易和 "agent runtime" 混淆。建议改名 `PlatformAdapter` 或 `ExecutionEnvironment`。

#### 1.2.8 `SessionStore` (session_store.rs:58)

```rust
#[async_trait]
pub trait SessionStore: Attributable {
    async fn load(&self, id) -> Result<Option<Session>>;
    async fn append_message(&self, id, message) -> Result<()>;
    async fn save(&self, session) -> Result<()>;
    async fn delete(&self, id) -> Result<()>;
    async fn list(&self) -> Result<Vec<String>>;
    async fn list_with_metadata(&self) -> Result<Vec<SessionMetadata>>;
}
```

- **现状**: 唯一一个 **core 内置了实现** 的 trait —— `JsonlSessionStore` (session_store.rs:94-372, ~280 行) 就在 core 里, 用 JSONL + `.meta.json` sidecar 做持久化。
- **为什么**: session 持久化是 agent 跑起来的刚需, 内置一个能用的实现避免"有 trait 无实现"的尴尬。
- **优化**:
  1. **实现塞在 trait 层违反微内核原则**。JsonlSessionStore 应该搬到 `shadow-runtime` 或独立的 `shadow-session` crate, core 只留 trait。否则 core 会越来越胖。
  2. 所有 IO 是 **同步 `std::fs`** 包在 async fn 里 (阻塞 runtime)。高频写入时应换 `tokio::fs`。
  3. `list_with_metadata` 每次 read_dir + 逐文件读 meta, O(n) 全表扫, session 多了会慢。

### 1.3 shadow-core 总体优化

| 问题 | 严重度 | 建议 |
|---|---|---|
| `JsonlSessionStore` 实现塞在 trait 层 | 中 | 下沉到 runtime/session crate |
| `platform.rs` 只有 4 行 `is_android()` | 低 | 合并进 `RuntimeAdapter` 或删除 |
| `Arc<T>` blanket impl 手写转发 (provider/observer) | 中 | 用 crate macro 或减少 trait 方法数 |
| trait 方法数过多 (Memory 20+, ModelProvider 15+) | 高 | 拆子 trait / 用 builder 封装参数 |

---

## 2. shadow-config — 配置层

**职责**: TOML schema 定义 + 加载 + 多 provider 解析 + 平台/密钥/可观测性子配置。
**依赖**: shadow-core (反向? 需确认), 无循环。

### 2.1 模块图

```
shadow-config/src/
├── lib.rs              # 重新导出
├── schema.rs           # ★ 主 Config struct (774 行)
├── providers.rs        # ModelProviders 容器 + resolve_provider
├── model_provider/     # 每种 family 的 config (custom.rs)
├── multi/              # 多 agent 配置 (alias_agent / risk_profile / runtime_profile / skill_bundle)
├── platform/           # native / docker RuntimeAdapter 实现
├── secrets.rs          # 加密密钥存储
├── migration.rs        # schema 版本迁移
├── autonomy.rs         # AutonomyLevel 配置
├── observability.rs    # 可观测性后端配置
└── proxy_client.rs     # HTTP proxy 客户端构建
```

### 2.2 现状

- `Config::load_or_init()` 是入口 (异步)。
- `providers.models` 用 flatten HashMap 存 `[providers.<family>.<alias>]`, 通过 `resolve_provider("openai.default")` 解析。
- `multi/alias_agent.rs` 定义 `MemoryBackendKind` (Sqlite/Markdown/None/Unknown) —— **被 shadow-memory 直接依赖**。
- `platform/native.rs` / `platform/docker.rs` 实现了 `RuntimeAdapter` trait。

### 2.3 为什么

借鉴 ZeroClaw 的 `providers.models.<family>.<alias>` 设计, 但精简为 flatten HashMap 减少嵌套。多 agent 配置独立成 `multi/` 子模块, 为多 agent/多 profile 留路。

### 2.4 优化

1. **schema.rs 774 行单文件** —— Config god struct, 应按 section 拆 (ProviderConfig / MemoryConfig / AgentConfig / ObservabilityConfig 各一文件)。
2. **`shadow-config` 反向被 shadow-core 的值类型引用?** —— 检查: core 不依赖 config (微内核原则保持)。但 `shadow-memory/lib.rs` 里 `MemoryBackendKind` 来自 `shadow_config::multi::alias_agent` —— 说明 **memory 的工厂逻辑反向耦合了 config 的类型**, 这会让 core→config 间接耦合。建议 `MemoryBackendKind` 下沉到 core 或 memory crate 自己定义。
3. `proxy_client.rs` 把 HTTP client 构建放 config 层, 但 `build_runtime_proxy_client_with_timeouts` 被 shadow-providers 调用 —— config crate 承担了 infra 职责, 职责越界。

---

## 3. shadow-log — 统一日志面

**职责**: `record!` 宏 + JSONL 持久化 + 广播 + LogCaptureLayer (接 tracing) + Observer 桥接。
**LOC**: 781 (7 文件), 设计文档自称 "ZeroClaw 5079 行的精简版"。

### 3.1 模块图

```
shadow-log/src/
├── lib.rs              # install_subscriber + re-export
├── event.rs            # LogEvent / Severity / Action / EventCategory / EventOutcome
├── writer.rs           # JSONL 持久化 + load_page (读)
├── broadcast.rs        # 广播 hook (TUI 订阅)
├── layer.rs            # LogCaptureLayer (tracing subscriber layer)
├── observer_bridge.rs  # LogObserver trait + forward (投影到 Observer)
└── macro.rs            # record! / attribution_span! 宏
```

### 3.2 现状

- `install_subscriber(verbose)` 装双层 tracing: stderr fmt + LogCaptureLayer (独立 filter `shadow_log_event=info`)。
- `LogObserver` 是独立 trait (不依赖 core::Observer), 通过 `OnceLock<RwLock<Option<Arc<dyn LogObserver>>>>` 全局单例桥接。
- `record!` 宏走 `tracing::info!(target="shadow_log_event", ...)`, LogCaptureLayer 捕获后落 JSONL + forward 给 Observer。

### 3.3 为什么

- 不依赖 core (避免 core ↔ log 循环), 所以另起 `LogObserver` 而非复用 `core::Observer`。
- 双层 filter 保证 `record!` 事件即使全局日志级别是 warn 也仍持久化。

### 3.4 优化

1. **两套 Observer trait (core::Observer vs log::LogObserver) 是认知负担**。长期应合并: 让 core 定义 `Observer`, log 层通过泛型或 erasure 桥接, 而非平行 trait。当前 forward() 投影是临时方案。
2. 全局 `OnceLock` 单例 observer 不可重入、不可多实例 —— 测试不友好。考虑走 tracing 的 subscriber 机制而非自定义全局态。
3. `event.rs` 里 `Action` 枚举 (来自 lib.rs 导出) 与 core::ObserverEvent 变体有重叠, 维护两套事件分类易漂移。

---

## 4. shadow-providers — LLM provider 实现

**职责**: 实现 `ModelProvider` trait, 3 层架构 (Router → Reliable → Compat)。
**LOC**: 1517 (8 文件)。

### 4.1 模块图

```
shadow-providers/src/
├── lib.rs          # create_model_provider 工厂 + 注释掉的大量旧工厂
├── openai.rs       # ★ OpenAiCompatibleModelProvider (448 行, 唯一真正实现的 Compat 层)
├── factory.rs      # FamilyProviderFactory trait + dispatch_family_factory
├── dispatch.rs     # ProviderDispatch / ProviderDispatchRef (归因 span 包裹)
├── router.rs       # Router (按 alias 路由, 191 行)
├── reliable.rs     # ReliableModelProvider (重试/退避/key 轮换/限流, 308 行)
├── rate_limit.rs   # TokenBucket
└── error.rs        # ChatError / RetryClass
```

### 4.2 现状

- **Compat 层**: 只有 `OpenAiCompatibleModelProvider` 一个实现 (覆盖 openai/openrouter/ollama/compatible/custom 全家族)。注释里提到 Anthropic 但 **AnthropicProvider 已被删除/注释掉** (lib.rs:61-65 注释)。
- **Reliable 层**: `ReliableModelProvider` 有完整代码 (308 行), 但 `lib.rs` 里 `create_reliable_provider` 工厂 **整个被注释掉** (lib.rs:80-127), 当前主路径不走 Reliable 包装。
- **Router 层**: `router.rs` 191 行, 但同样未被主路径调用。
- **主路径实际走的**: `create_model_provider` → `create_model_provider_inner` → `factory::dispatch_family_factory` → 直接造 Compat 层, **跳过 Reliable 和 Router**。
- `ProviderDispatch` 是归因 span 自动包裹层, 用 `tracing::Instrument` 注入 future。

### 4.3 为什么

3 层架构借鉴 ZeroClaw, 设计上每层职责清晰 (路由/可靠性/兼容)。当前只激活最底层, 是 MVP 阶段的权宜。

### 4.4 优化 (问题严重)

1. **lib.rs 有 ~100 行注释掉的工厂代码** (lib.rs:25-127), 包括 `create_provider` / `create_provider_with_opts` / `create_reliable_provider`。死注释堆积, 应删除或恢复。`create_model_provider_inner` 里有空 `if let Some(idx) = raw_name.find(":") {}` 空块 (lib.rs:148-150) —— 明显是写到一半。
2. **Reliable/Router 两层写完了却没接入** —— 代码资产闲置。应在工厂里按配置条件激活 (有 retry policy 就包 Reliable, 有多 alias 就用 Router)。
3. **Anthropic 原生支持缺失** —— `ModelProviderKind::Anthropic` 在 core 里定义了, 但实现被删。当前只能走 OpenAI-compatible 降级, 丢失 Anthropic 原生 tool-call / prompt-cache 能力。
4. `factory.rs` 的 `apply_compat_options` 函数体只有 `Box::new(p)` (factory.rs:71-77), 完全没应用 opts —— 等于 `ModelProviderRuntimeOptions` 里的 timeout/extra_headers/reasoning_effort 全被丢弃。
5. `openai.rs` 里 `new_with_opts` 的 `user_agent` / `merge_system_into_user` 参数 **接收了但没存到 self** (openai.rs:53-73) —— 死参数。

---

## 5. shadow-memory — 记忆后端实现

**职责**: 实现 `Memory` + `MemoryStrategy` trait。
**LOC**: 3703 (10 文件), 最重的实现 crate。

### 5.1 模块图

```
shadow-memory/src/
├── lib.rs                    # ★ create_memory_for_agent 工厂 + embedding 路由解析
├── sqlite.rs                 # SqliteMemory (1752 行, FTS5 + 语义检索)
├── markdown.rs               # MarkdownMemory (366 行)
├── none.rs                   # NoneMemory (112 行)
├── agent_scoped.rs           # AgentScopedMemory (agent 隔离 + peer 共享装饰器, 416 行)
├── agent_scoped_markdown.rs  # AgentScopedMarkdownMemory (markdown 版)
├── strategy.rs               # DefaultMemoryStrategy (实现 MemoryStrategy)
├── embedding.rs              # EmbeddingProvider trait + Noop/OpenAI 实现
├── vector.rs                 # 余弦相似度 + 混合检索融合
└── conflict.rs               # 冲突处理
```

### 5.2 现状

- **实现矩阵**: NoneMemory / MarkdownMemory / SqliteMemory 三个 backend + AgentScoped* 装饰器两个。
- `create_memory_for_agent(config, agent_alias, api_key)` 是入口, 按 `MemoryBackendKind` 分发:
  - Markdown → 构 own + peers → AgentScopedMarkdownMemory
  - None → NoneMemory
  - Sqlite/其他 → create_memory_with_storage_and_routes → SqliteMemory → AgentScopedMemory 包装
- **SqliteMemory 是核心** (1752 行): FTS5 全文 + embedding 语义, 混合检索 (vector_weight + keyword_weight)。
- embedding 走 `EmbeddingProvider` (Noop/OpenAI), 支持 `embedding_routes` 按 `hint:` 前缀路由到不同模型。
- `DefaultMemoryStrategy` 实现巩固/召回/治理三方法。

### 5.3 为什么

- 装饰器模式 (AgentScoped*) 让"agent 自己的记忆 + 只读 peer 记忆"组合不侵入 backend 本身。
- 混合检索 (FTS5 + 向量) 是当前 LLM 记忆的最佳实践。
- embedding 路由让不同 agent 用不同 embedding 模型。

### 5.4 优化

1. **lib.rs 工厂逻辑混乱** —— `create_memory_with_builders` 的 match 里, Lucid/Postgres/Qdrant/Markdown 全注释掉, 最后 `_ => Markdown` 兜底 (lib.rs:216-240)。说明多 backend 路径半成品。`classify_memory_backend` 只识别 "sqlite"/"none", 其余一律 Unknown→Markdown 兜底, 语义模糊。
2. **bug 嫌疑** (lib.rs:136-138): `create_memory_for_agent` 里给 peer 解析 uuid 时用的是 `agent_alias` 而非 `peer_alias`:
   ```rust
   for peer in &agent_cfg.workspace.read_memory_from {
       let uuid = inner_arc.ensure_agent_uuid(agent_alias).await?;  // 应为 peer
       allowlist_ids.push(uuid);
   }
   ```
   这会让 peer 白名单全部解析成当前 agent 自己的 uuid, peer 共享失效。
3. **`agent_scoped` vs `agent_scoped_markdown` 两套并行**, 接口相似但无共同抽象 —— 可提取 `AgentScoped<M: Memory>` 泛型统一。
4. sqlite.rs 1752 行单文件, 应拆 (schema/migration/fts/vector/store 五块)。
5. `MemoryBackendKind::Unknown` 作为兜底变体容易被误用, 建议改成 `NonStd(String)` 保留原始名便于诊断。

---

## 6. shadow-runtime — agent loop + 工具集

**职责**: agent 主循环、工具注册表、prompt 构建、工具调度、cron、安全策略、skills。
**LOC**: 4704 (32 文件), 文件最多。

### 6.1 模块图

```
shadow-runtime/src/
├── lib.rs              # 模块声明 (agent_bak 未声明 = 死代码)
├── agent/
│   ├── mod.rs          # pub use loop_::*
│   ├── loop_.rs        # ★ 当前 agent::run (109 行 STUB!)
│   └── loop_detector.rs
├── agent_bak.rs        # ⚠️ 1017 行旧 agent loop, 未挂载, 不编译
├── dispatcher.rs       # ToolDispatcher trait + Native/XML 实现
├── tools/
│   ├── registry.rs     # ToolRegistry
│   ├── attribution.rs
│   └── cron/           # cron 工具 (add/remove/list/run/runs)
├── cron/               # cron 调度器 (schedule/types)
├── prompt/             # ★ SystemPromptBuilder + 9 个子模块
│   ├── mod.rs          # PromptSection trait + 5 个基础 section
│   ├── bootstrap.rs / caching.rs / context_compressor.rs
│   ├── injection_guard.rs / persona.rs / prompt_guided.rs
│   ├── safety_injection.rs / tools_payload.rs / truncation.rs
├── security/mod.rs     # SecurityPolicy
└── skills/mod.rs       # skills
```

### 6.2 现状 (⚠️ 关键问题)

- **`agent::run` (loop_.rs) 是 stub**: 只做了 config 解析 + 建 memory, 然后 `return Ok("exit".to_string())` (loop_.rs:96) —— **没有 LLM 调用, 没有工具循环, 没有 observer**。当前 `shadow agent` 命令实际不跑 agent。
- **`agent_bak.rs` (1017 行) 是上一版完整 agent loop**, 但 `lib.rs` 里 `pub mod agent_bak;` **没声明**, 文件不被编译。真正的 agent 循环逻辑 (LLM 调用/工具执行/历史管理) 全在这里。
- **prompt 子系统最完整**: `SystemPromptBuilder` + 9 个 PromptSection 实现 (DateTime/Identity/ToolHonesty/Safety/Workspace 基础 + bootstrap/caching/compressor/injection_guard/persona 高级), 设计良好。
- **ToolDispatcher** 有 Native/XML 两实现, 完整可用。
- **ToolRegistry** 简单但可用。
- **cron** 有完整工具 + 调度器。
- **security / skills** 有模块声明但内容薄。

### 6.3 为什么

项目处于 **agent loop 重构中途**: 旧实现 (agent_bak) 被整体冻结, 新 loop_ 还没写完。重构动机推测: 旧 agent 把所有逻辑揉在一个 1000 行函数里, 新版想拆成 config 解析 → runtime 构建 → loop 执行三段。但只完成了第一段。

### 6.4 优化 (最高优先级)

1. **🔴 P0: 决定 agent_bak 的命运**。两条路:
   - (a) 把 agent_bak 挂回模块树 (`pub mod agent_bak;`), 先让 agent 跑起来, 再渐进重构;
   - (b) 把 agent_bak 的逻辑移植进新的 `agent/loop_.rs` (推荐), 拆成 `resolve_config` / `build_runtime` / `run_loop` 三个函数。
   - 当前状态 (代码在但用不了) 是最坏的 —— 既不能用又增加维护混淆。
2. **prompt 子系统应推广为 runtime 的中心** —— 它设计最干净, 可作为其他模块的模板。
3. `agent_bak.rs` 文件名带 `_bak` 后缀, 容易被视为可删; 但它含唯一可用的 loop 逻辑, 切勿误删。重命名为 `agent/legacy.rs` 并显式标注。
4. `lib.rs` 里 `pub mod agent;` 和 `pub mod agent_bak;` 混乱的根因是模块声明与文件不同步 —— 建议加 CI 检查 "无孤儿 .rs 文件"。

---

## 7. shadow (bin) — CLI 入口

**LOC**: 455 (main.rs / lib.rs / config/mod.rs / proxy_main.rs)。

### 7.1 现状

- `main.rs` 用 clap 定义 `Commands::Agent` 一个子命令 (+ 注释里的 Eval/Config/Memory 未接入)。
- **双特性编译**: `--no-default-features` 走 kernel-only 直连 provider 路径; 默认 `--features runtime` 走 `agent::run` (但 run 是 stub, 见 §6)。
- `read_capped_line` 做了 stdin 1MB 截断保护。
- `src/lib.rs` 只有 2 行 (`pub mod config; pub use config::Config;`)。

### 7.2 优化

1. main.rs 里 `ConfigAction` / `MemoryAction` / `EvalCommands` 枚举定义了但 `Commands` 没包含它们 —— 死枚举。
2. `config/mod.rs` 只是 re-export shadow_config::Config, 多此一举 (src/lib.rs 已经 re-export)。可删。
3. `proxy_main.rs` 存在但未在 main.rs 引用 —— 另一个孤儿文件。

---

## 8. 非 workspace 占位 crate (10 个)

`shadow-channels / shadow-eval / shadow-gateway / shadow-hardware / shadow-infra / shadow-macros / shadow-plugins / shadow-spawn / shadow-tool-call-parser / shadow-tools`

每个都是 6 行:
```rust
//! 影子XX -- ...
//! 对照 ZeroClaw 对应 crate, 尚未实现。
// TODO: 实现模块
```

**不在 workspace members**, 不参与编译。

### 优化

这些占位目录有两种处理:
- **保留为路线图锚点** (当前做法): 优点是可视化未来蓝图; 缺点是目录噪音, 且 Cargo.toml 各自存在但无 src 内容, `cargo` 误入会报错。
- **移除, 改用 docs/roadmap.md 记录**: 更干净。
- 建议: 至少把它们从 `crates/` 移到 `crates/.planned/` 子目录, 或在各自 Cargo.toml 加注释说明状态, 避免新人误以为它们在编译。

---

## 9. 横切问题与优化优先级

### 9.1 结构性问题汇总

| # | 问题 | 位置 | 严重度 |
|---|---|---|---|
| 1 | agent loop 是 stub, 真实代码在未编译的 agent_bak.rs | shadow-runtime | 🔴 P0 |
| 2 | providers 工厂大量注释代码 + Reliable/Router 两层闲置 | shadow-providers/lib.rs | 🟠 P1 |
| 3 | memory 工厂 peer uuid 解析用错变量 (bug) | shadow-memory/lib.rs:137 | 🔴 P0 |
| 4 | MemoryBackendKind 反向耦合 config→memory | shadow-config/multi | 🟡 P2 |
| 5 | core 内置 JsonlSessionStore 实现 (违反微内核) | shadow-core/session_store.rs | 🟡 P2 |
| 6 | 两套 Observer trait (core vs log) | shadow-log/observer_bridge | 🟡 P2 |
| 7 | ModelProvider blanket Arc impl 手写 15+ 转发 | shadow-core/provider.rs:420 | 🟠 P1 |
| 8 | ModelProvider::chat 默认实现分支语义混乱 | shadow-core/provider.rs:312 | 🟠 P1 |
| 9 | factory apply_compat_options 空实现, opts 被丢 | shadow-providers/factory.rs:71 | 🟠 P1 |
| 10 | 10 个占位 crate 目录噪音 | crates/* | 🟢 P3 |

### 9.2 推荐优化顺序

1. **P0 — 让 agent 跑起来**: 把 agent_bak 逻辑迁入新 loop_ (或先挂回模块树)。同时修 memory peer uuid bug。
2. **P1 — providers 清理**: 删死注释, 接入 Reliable/Router 两层 (按配置激活), 恢复或明确放弃 Anthropic 原生支持, 修 apply_compat_options。
3. **P1 — core trait 瘦身**: ModelProvider 的 Arc blanket impl 用宏生成; chat() 默认实现重写; 拆 Memory 子 trait。
4. **P2 — 解耦**: JsonlSessionStore 下沉; 统一 Observer trait; MemoryBackendKind 下沉到 core。
5. **P3 — 清理**: 占位 crate 归档; 删 main.rs 死枚举 / proxy_main.rs / config/mod.rs 多余中间层。

### 9.3 设计优点 (值得保持)

- **微内核 + trait 驱动** 的分层是健康的, core 零内部依赖得以保持。
- **Attributable 归因系统** 是亮点, 横切且自洽, 比传参干净。
- **prompt 子系统** (PromptSection 可插拔 + priority 排序) 设计成熟, 可作模板。
- **Memory 的 backend × strategy 正交分离** + AgentScoped 装饰器, 扩展性好。
- **SessionStore 的 JSONL + sidecar meta** 向后兼容设计周到。
- `#[non_exhaustive]` ObserverEvent 为演进预留。

---

## 附: trait → impl 对照速查

| trait (shadow-core) | 实现位置 | 实现数 |
|---|---|---|
| `Attributable` | 所有 trait 的实现者自动 (super-trait) | 全覆盖 |
| `ModelProvider` | `OpenAiCompatibleModelProvider` (providers/openai.rs) | 1 (Anthropic/Reliable/Router 代码在但未接入) |
| `Memory` | `NoneMemory` / `MarkdownMemory` / `SqliteMemory` | 3 + 2 装饰器 |
| `MemoryStrategy` | `DefaultMemoryStrategy` (memory/strategy.rs) | 1 |
| `Tool` | (runtime/tools/cron/* 各工具, 需确认是否 impl Tool) | 待查 |
| `Observer` | (未找到具体 impl, LogCaptureLayer 间接) | 0 直接? |
| `Channel` | 无 | 0 |
| `SessionStore` | `JsonlSessionStore` (core 内置) | 1 |
| `RuntimeAdapter` | `shadow-config::platform::{native, docker}` | 2 |
| `ToolDispatcher` (runtime 定义) | `NativeToolDispatcher` / `XmlToolDispatcher` | 2 |
| `PromptSection` (runtime 定义) | DateTime/Identity/ToolHonesty/Safety/Workspace + 9 子模块 | 14+ |
| `FamilyProviderFactory` (providers 定义) | `CustomModelProviderConfig` / `ModelProviderConfig` | 2 |
| `LogObserver` (log 定义) | (桥接用, 待查具体 impl) | ? |
