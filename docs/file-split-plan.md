# 文件拆分规划

> 基于 main 分支 `d1c4b9a` 的真实 LOC 与代码骨架
> 原则: 按**自然语义边界**拆, 不为凑数切; 单一职责的文件即使 400 行也不拆; 公共 re-export (lib.rs) 保持稳定

---

## 拆分优先级总览

| 级别 | 文件 | 当前 LOC | 主要问题 |
|---|---|---|---|
| **P0** | `shadow-memory/src/sqlite.rs` | 1752 | 单 impl 块 974 行, schema/store/recall/agent 混在一起 |
| **删除** | `shadow-runtime/src/agent_bak.rs` | 1017 | 未挂载模块树, 死代码 |
| **P1** | `shadow-config/src/schema.rs` | 774 | Config god struct + 8 个子配置 + 路径解析 + 反序列化全堆一起 |
| **P1** | `shadow-runtime/src/skills/mod.rs` | 561 | skill 服务 + SKILL.md 解析 + 手写 YAML 解析器 3 件事 |
| **P1** | `shadow-config/src/providers.rs` | 557 | provider entry + reliable + router + resolve 4 个域 |
| **P2** | `shadow-core/src/kennel/provider.rs` | 640 | trait + 值类型 + blanket + helper 混 |
| **P2** | `shadow-core/src/kennel/memory.rs` | 493 | trait + 值类型混 |
| **P3** | `shadow-providers/src/openai.rs` | 448 | provider impl + 20 个 API DTO + stream 累加器 |
| **P3** | `shadow-runtime/src/tools/cron/add.rs` | 460 | 单 tool, 但 Arg/Schedule/JobType 枚举可抽 |
| **P3** | `shadow-runtime/src/security/mod.rs` | 338 | Sandbox + SecurityPolicy 两件事 |
| **不拆** | 见末节 | — | 单一职责, 拆了反而碎 |

---

## P0 — sqlite.rs (1752 行)

### 现状骨架

```
1-31     启动锁 helper
32-54    struct SqliteMemory
55-747   impl SqliteMemory        (692 行: 构造 + schema 初始化 + 各类内部 helper)
748-757  impl Attributable
758-1732 impl Memory for SqliteMemory  (974 行: 所有 trait 方法)
1733-52  模块级 ensure_agent_uuid 函数
```

### 拆分方案

```
shadow-memory/src/sqlite/
├── mod.rs          struct SqliteMemory + 构造器 (new/with_embedder) + Attributable impl
│                   re-export 子模块, 保持 lib.rs 的 SqliteMemory 路径不变
├── schema.rs       DDL / 建表 / 迁移 / 索引 (现在混在 impl 里的 CREATE TABLE 语句)
├── store.rs        store / store_with_agent / store_procedural / supersede (写路径)
├── recall.rs       recall / recall_for_agents / recall_namespaced + FTS5 + 向量混合检索
├── agent.rs        ensure_agent_uuid / count_agent / purge_agent / rename_agent / export_agent
└── util.rs         acquire_sqlite_startup_lock + 杂项 helper
```

**Memory trait impl** 留在 `mod.rs`, 但方法体瘦身后委托到子模块的自由函数:
```rust
// mod.rs
async fn store(&self, key, content, cat, sid) -> Result<()> {
    sqlite::store::store(&self.conn, key, content, cat, sid, &self.embedder).await
}
```

### 理由
- 974 行的 `impl Memory` 是全项目最大单体, 任何改动都要在巨函数里定位。
- schema/store/recall/agent 是**4 个互不相交的 SQL 子域**, 天然 seam。
- 拆完每个文件 200-400 行, 单测可按子模块组织 (现在测试也得在 1752 行里找)。

---

## 删除 — agent_bak.rs (1017 行)

未在 `shadow-runtime/src/lib.rs` 声明为模块, **不参与编译**。真正的 agent loop 逻辑困在这里, 当前 `agent/loop_.rs` 是 109 行 stub。

两条路 (见 structure-analysis.md §6.4), 二选一:
1. **移植**: 把 agent_bak 的逻辑按"config 解析 / runtime 构建 / loop 执行"三段重构进 `agent/loop_.rs`, 然后**删 agent_bak.rs**;
2. **挂回**: 若暂时不想重构, `lib.rs` 加 `pub mod agent_bak;` 先让它编译, 但**必须改名** (去掉 `_bak` 后缀, 否则容易被当垃圾删)。

不要保持现状 (代码在但用不了)。这条比所有拆分都优先。

---

## P1 — schema.rs (774 行)

### 现状骨架

```
25-71    struct Config (47 字段 god struct)
76-134   impl Default for Config
135-287  impl Config (load_or_init + 路径解析: macos homebrew 检测 / tilde 展开)
288-425  impl Config (resolution 方法: resolved_agent_config / model_provider_for_agent ...)
426-439  ActiveStorage enum
440-488  SqliteStorageConfig / SchedulerConfig
489-513  ResolvedRuntime / ObservabilityConfig
515-573  MemoryConfig / StorageConfig
574-596  EmbeddingRouteConfig / SearchMode
597-727  RuntimeConfig / RuntimeKind / DockerRuntimeConfig
728-774  自定义反序列化函数 (reasoning_effort / lenient enum / email skip)
```

### 拆分方案

```
shadow-config/src/
├── lib.rs              重新导出 (保持外部 API 路径不变)
├── config.rs           struct Config + Default + load_or_init + resolution 方法
├── paths.rs            default_config_dir / resolve_runtime_config_dirs /
│                       try_resolve_macos_homebrew_config_dir / expand_tilde_path
│                       (纯路径推导, 与 Config 业务无关)
├── storage.rs          SqliteStorageConfig / StorageConfig / ActiveStorage
├── scheduler.rs        SchedulerConfig
├── observability.rs    (已存在, 保持)
├── runtime_cfg.rs      RuntimeConfig / RuntimeKind / DockerRuntimeConfig / ResolvedRuntime
├── memory_cfg.rs       MemoryConfig / EmbeddingRouteConfig / SearchMode
└── deserializers.rs    deserialize_reasoning_effort_opt / deserialize_enum_lenient /
                        deserialize_optional_email_skip_empty / normalize_reasoning_effort
```

### 理由
- 8 个独立子配置域混在一个文件, 改 storage 不该滚到 runtime 配置。
- 路径解析 (macOS homebrew 探测 70+ 行) 是平台特定逻辑, 独立后便于条件编译与测试。
- 反序列化函数是横切 helper, 单独成文件避免业务 struct 文件里穿插 `fn deserialize_*`。

---

## P1 — skills/mod.rs (561 行)

### 现状骨架

```
38-139   数据结构 + SkillsService (业务)
140-236  SKILL.md 解析 (parse_skill_md / split_frontmatter)
237-485  ★ 手写 YAML 解析器 (YamlLine / YamlValue / parse_block/mapping/sequence)
486-560  目录加载 (load_skills_from_dir / load_skills)
```

### 拆分方案

```
shadow-runtime/src/skills/
├── mod.rs          re-export + SkillsService
├── skill.rs        Skill / SkillTool / SkillFrontmatter struct
├── parser.rs       parse_skill_md / split_frontmatter
├── yaml.rs         YamlLine / YamlValue / parse_yaml 全家 (纯解析, 无业务)
└── loader.rs       load_skills_from_dir / load_skills
```

### 理由
- **手写 YAML 解析器 248 行**是最大的独立块, 完全纯函数, 与 skill 业务无关, 独立后甚至可单独 fuzz/test。
- 现在改 skill 加载逻辑要在 YAML 解析器代码中间穿行, 心智负担大。
- `mod.rs` 名字暗示是入口聚合, 却装着 561 行实现, 误导。

---

## P1 — providers.rs (557 行)

### 现状骨架

```
1-117    use + ModelProviders impl (find_by_name / ensure)
118-213  Providers / ProviderEntry struct
214-247  ProviderEntry impl
264-327  ProviderEntry Deserialize (自定义, 64 行)
328-402  ReliableConfig + impl
403-430  RouterConfig / RouteEntry
431-497  ProviderRef / ResolvedProvider + impl
498-557  default_base_url / resolve_provider / list_providers
```

### 拆分方案

```
shadow-config/src/providers/
├── mod.rs          re-export + Providers / ModelProviders 容器
├── entry.rs        ProviderEntry struct + impl + Deserialize
├── reliable.rs     ReliableConfig + impl (当前 shadow-providers/reliable.rs 是 impl 层,
│                   这个是 config 层, 别混)
├── router.rs       RouterConfig / RouteEntry
└── resolve.rs      ProviderRef / ResolvedProvider / resolve_provider /
                    list_providers / default_base_url
```

### 理由
- 4 个独立 config 域 (entry/reliable/router/resolve) 混在一起, 每个有自己的 struct + impl + 默认值函数。
- `ProviderEntry` 的自定义 Deserialize 64 行, 独立后业务 struct 文件更干净。

---

## P2 — core trait 值类型外提

### provider.rs (640 行)

trait 本身 200 行 + 值类型 (ChatMessage/ChatRequest/ChatResponse/ToolCall/TokenUsage/StreamChunk/StreamEvent/ToolsPayload/AuthStyle/ModelProviderRuntimeOptions/ProviderCapabilities/ModelInfo) ~250 行 + **Arc blanket impl 117 行** + helper (build_tool_instructions_text)。

```
shadow-core/src/kennel/provider/
├── mod.rs          ModelProvider trait 定义 + re-export
├── types.rs        所有值类型 (Chat*/ToolCall/TokenUsage/Stream*/ToolsPayload/Auth*/...)
├── tools_payload.rs build_tool_instructions_text + ToolsPayload 方法
└── (删除 blanket.rs)  ← 见 structure-analysis §1.2.2, 整段删
```

> 注: blanket impl 拆分前先**删除**它 (靠 Deref 工作), 否则拆出去还是 117 行手写转发的负担。

### memory.rs (493 行)

```
shadow-core/src/kennel/memory/
├── mod.rs          Memory trait + MemoryStrategy trait + re-export
├── types.rs        MemoryEntry / MemoryCategory / MemoryKind / SemanticSubType /
│                   StoreOptions / MemoryStats / ExportFilter / ProceduralMessage
└── helpers.rs      is_recent_recall_query / normalize_recent_recall_query
```

### 理由
- trait crate 把"接口"和"数据形状"分开是 Rust 惯例 (如 tokio::io::{AsyncRead, AsyncReadExt} vs 各 Buf 类型)。
- 值类型外提后, trait 文件只剩接口, 阅读时一眼看清能力契约。
- **风险**: 必须保证 `lib.rs` 的 re-export 不变, 外部 `shadow_core::ChatMessage` 等路径要照常工作。拆分时 lib.rs 补 `pub use kennel::provider::types::*;`。

---

## P3 — 边界拆分

### openai.rs (448 行)
```
shadow-providers/src/openai/
├── mod.rs      OpenAiCompatibleModelProvider + Attributable + ModelProvider impl
├── types.rs    ChatRequest/ApiMessage/ApiTool/ApiResponse/Choice/ResponseMessage 等 20 个 DTO
└── stream.rs   StreamToolCallAccumulator / StreamToolCallDelta / StreamFunctionDelta
```

### cron/add.rs (460 行)
单 tool 但枚举多。**可抽可不抽**:
```
shadow-runtime/src/tools/cron/
├── add.rs      CronAddTool + impl Tool
└── types.rs    Arg / Schedule / JobType (被 add/remove/list 共用, 目前每个 tool 各自重复定义)
```
> 注: 顺带把 cron 各 tool (add/remove/list/run/runs) 里**重复的 Arg/Schedule/JobType** 统一到一个 types.rs, 这是比拆 add.rs 本身更大的收益。

### security/mod.rs (338 行)
```
shadow-runtime/src/security/
├── mod.rs          SecurityPolicy + Default + impl
├── sandbox.rs      Sandbox trait + NoopSandbox
└── risk.rs         CommandRiskLevel enum
```

---

## 明确不拆的文件

| 文件 | LOC | 不拆理由 |
|---|---|---|
| `agent_scoped.rs` | 416 | 单一职责 (AgentScopedMemory 装饰器), impl Memory 是主体, 拆了反而碎 |
| `markdown.rs` | 366 | 单 impl, 同上 |
| `reliable.rs` (providers) | 308 | 单 impl ReliableModelProvider, 内聚 |
| `session_store.rs` (core) | 373 | trait + 唯一默认 impl, 在一起便于对照 (见 structure-analysis 决定: JsonlSessionStore 留 core) |
| `main.rs` | 342 | CLI 入口, 子命令扩到 `commands/` 是另一个话题, 当前只 1 个 Agent 命令不值得 |
| `memory/lib.rs` | 339 | 工厂函数, 逻辑连贯 |
| `strategy.rs` | (待查) | DefaultMemoryStrategy 单 impl |

---

## 执行顺序建议

1. **先删/移植 agent_bak.rs** (不拆分, 但最优先 — 释放 1017 行死代码, 让 agent 跑通)
2. **sqlite.rs 拆 P0** (收益最大, 1752→5×300)
3. **schema.rs 拆 P1** (Config 是所有配置改动的入口, 瘦身后开发体验明显好)
4. **skills/mod.rs 拆 P1** (YAML 解析器外提是纯赚)
5. **providers.rs 拆 P1** (顺带统一 cron 重复枚举)
6. P2/P3 按需, 不急

每步拆完**保持 `cargo test` 绿 + lib.rs re-export 不变**, 这样拆分对下游 crate 透明, 可逐步独立 PR。

---

## 风险与守则

- **re-export 稳定**: 所有拆分必须通过 lib.rs 的 `pub use` 维持外部路径, 下游 crate 零改动。
- **可见性**: 子模块里的 struct/fn 默认 `pub(crate)`, 只通过 mod.rs 暴露该暴露的。
- **测试位置**: 随源码迁移, `#[cfg(test)] mod tests` 跟着各自的子文件走。
- **不要过度拆**: 一个文件 < 200 行就不该再拆。本规划的最小目标文件 ≥ 150 行。
- **每个拆分独立提交**: 便于 review 和回滚, 不要一把梭。
