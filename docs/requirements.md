# Shadow 需求规格说明书

> 本文档由现有代码反向归纳,描述 Shadow 运行时各子系统**应当满足**的需求。
> 与已有的 `*-design.md`(设计) / `*-plan.md`(实施计划) / `analysis/*.md`(差距分析)不同,
> 本文档关注 **要做什么、降级策略、缺省值、哪些可配置/哪些硬编码**,作为单一权威需求源。
>
**状态约定**: ✅ 已实现 · ⚠️ 部分实现/脚手架 · ❌ 设计存在但未启用 · 🚧 待实现

**版本**: 对应 `main` 分支 @ ed7e0a4 (2026-07-14)

---

## 0. 文档定位

| 项 | 说明 |
|---|---|
| 读者 | Shadow 维护者、二次开发者、架构评审人 |
| 来源 | 直接读取 `crates/` 源码归纳,非前瞻设计 |
| 与其他文档关系 | 本文档为需求基线;`*-design.md` 为方案论证;`*-plan.md` 为拆解步骤 |
| 不包含 | 具体代码示例、迁移路径、竞品对比(已在其它文档) |

---

## 1. 总体描述

### 1.1 产品定位

Shadow 是一个基于 Rust 的 **trait 驱动 AI Agent 运行时**,采用微内核架构:
- 核心能力(LLM、Memory、Tool、Channel、Observer)全部抽象为 trait
- 通过组合具体实现装配出可运行的 Agent
- 支持多 Provider、多 Memory 后端、多平台(native/docker)、多 Agent 别名

### 1.2 架构分层

```
┌─────────────────────────────────────────────────────┐
│  src/main.rs (shadow CLI · clap)                    │
├─────────────────────────────────────────────────────┤
│  shadow-runtime                                      │
│   agent loop · tools · prompt · security · skills · │
│   cron · dispatcher · loop_detector                 │
├─────────────────────────────────────────────────────┤
│  shadow-core (微内核 · trait 层 · 零内部依赖)        │
│   ModelProvider · Memory · Tool · Channel ·         │
│   Observer · RuntimePlatformAdapter · SessionStore  │
├─────────────────────────────────────────────────────┤
│  shadow-providers │ shadow-memory │ shadow-config │ │
│                   │                 │ shadow-log    │
└─────────────────────────────────────────────────────┘
```

Crate 依赖单向向内,`shadow-core` 是 ABI 层,不依赖任何内部 crate。

### 1.3 核心抽象(全部为 trait)

| Trait | 位置 | 作用 |
|---|---|---|
| `ModelProvider` | `shadow-core/src/kennel/provider.rs:247` | LLM 后端 |
| `Memory` | `shadow-core/src/kennel/memory.rs:221` | 记忆存储 + 召回 |
| `MemoryStrategy` | `shadow-core/src/kennel/memory.rs:448` | 记忆加载/整合策略 |
| `Tool` | `shadow-core/src/kennel/tool.rs:75` | Agent 可调用工具 |
| `Channel` | `shadow-core/src/channel.rs:29` | 消息平台渠道 |
| `Observer` | `shadow-core/src/kennel/observer.rs` | 指标/事件观察 |
| `RuntimePlatformAdapter` | `shadow-core/src/runtime.rs:6` | 运行环境(native/docker) |
| `SessionStore` | `shadow-core/src/session_store.rs:58` | 会话持久化 |
| `Attributable` | `shadow-core/src/kennel/attribution.rs` | 归因(角色+别名) |
| `Sandbox` | `shadow-runtime/src/security/mod.rs:86` | 命令沙箱包装 |
| `ToolDispatcher` | `shadow-runtime/src/dispatcher.rs:10` | 工具协议适配 |
| `PromptSection` | `shadow-runtime/src/prompt/mod.rs:58` | 系统提示分段 |
| `FamilyProviderFactory` | `shadow-providers/src/factory.rs:5` | 按家族构建 Provider |

> ⚠️ **README 与多处注释提到的 `AgentRuntime` trait 在代码中不存在**。
> 实际承担 Agent 运行职责的是:
> 1. `RuntimePlatformAdapter`(环境抽象)
> 2. `shadow-runtime/src/agent/loop_.rs::run()`(运行入口函数)
> 3. `AgentRuntimeOverrides`(运行时注入结构)

---

## 2. Memory 子系统需求

### 2.1 功能性需求

| ID | 需求 | 状态 | 来源 |
|---|---|---|---|
| MEM-01 | **存储**:写入一条带分类、会话、命名空间的记忆 | ✅ | `memory.rs:226` `store` |
| MEM-02 | **带元数据存储**:支持 namespace / importance / kind / pinned / tenant | ✅ | `memory.rs:400` `store_with_options` |
| MEM-03 | **Agent 限定存储**:绑定 agent_id,避免越权 | ✅ | `memory.rs:419` `store_with_agent` |
| MEM-04 | **程序性记忆**:存多轮对话消息为一条程序性记忆 | ✅ | `memory.rs:305` `store_procedural` |
| MEM-05 | **关键词召回**:按 query + session + 时间窗过滤 | ✅ | `memory.rs:235` `recall` |
| MEM-06 | **命名空间召回**:先 recall 再按 namespace 过滤(默认实现) | ✅ | `memory.rs:338` `recall_namespaced` |
| MEM-07 | **多 Agent 召回**:在允许列表范围内召回 | ✅ | `memory.rs:430` `recall_for_agents` |
| MEM-08 | **按 key 获取 / 删除** | ✅ | `memory.rs:245,261` |
| MEM-09 | **Agent 限定的 get / forget** | ✅ | `memory.rs:248,264` |
| MEM-10 | **列出**(按 category + session 过滤) | ✅ | `memory.rs:253` |
| MEM-11 | **批量清除**:purge_namespace / purge_session / purge_agent | ⚠️ | `memory.rs:266-280` 默认 `bail!`,Sqlite 后端实现 |
| MEM-12 | **版本替代**:新记忆取代旧记忆,记录 superseded_by | ✅ | `memory.rs:301` `supersede` |
| MEM-13 | **导出**:按 namespace/session/category/时间窗导出 | ✅ | `memory.rs:358` `export` |
| MEM-14 | **Agent 导出 / 重命名 / 计数** | ⚠️ | `memory.rs:282-296` 默认空实现或 `bail!` |
| MEM-15 | **Agent UUID 保证**:按 alias 查/分配稳定 UUID | ✅ | `memory.rs:440` `ensure_agent_uuid` |
| MEM-16 | **健康检查** | ✅ | `memory.rs:299` |
| MEM-17 | **统计**:总条数、按分类、被取代条数、pinned 条数、字节 | ✅ | `memory.rs:321` `stats` |
| MEM-18 | **重建索引** | ✅ | `memory.rs:325` `reindex` |
| MEM-19 | **热更换 Embedding 模型** | ✅ | `memory.rs:329` `refresh_embedder` |
| MEM-20 | **记忆策略**:load_context / consolidate_turn / run_governance | ⚠️ | `memory.rs:448` `MemoryStrategy` trait,Sqlite 实现完整 |

### 2.2 数据模型(MemoryEntry)

字段定义见 `memory.rs:99-134`:

| 字段 | 类型 | 必填 | 含义 |
|---|---|---|---|
| `id` | String(uuid) | 是 | 后端生成 |
| `agent_id` | Option<String> | 否 | 绑定 Agent(uuid) |
| `key` | String | 是 | 业务键(用于 get/forget) |
| `content` | String | 是 | 记忆内容 |
| `timestamp` | String(RFC3339) | 是 | 后端生成 |
| `session_id` | Option<String> | 否 | 会话级隔离 |
| `score` | Option<f64> | 否 | 检索相关度(仅 recall 结果) |
| `agent_alias` | Option<String> | 否 | 归因别名 |
| `namespace` | String(默认 "default") | 是 | 命名空间 |
| `importance` | Option<f64> | 否 | 权重/重要度 |
| `superseded_by` | Option<String> | 否 | 版本替代链 |
| `kind` | Option<MemoryKind> | 否 | 类型(与持久性无关) |
| `pinned` | bool | 是 | 预算驱逐保护 |
| `tenant_id` | Option<String> | 否 | 多用户隔离 |
| `category` | MemoryCategory | 是 | 分类枚举 |

### 2.3 分类与类型

**MemoryCategory 枚举**(`memory.rs:26`):

| 分类 | as_str | 语义 |
|---|---|---|
| `Core` | "core" | 长期事实/偏好 |
| `Daily` | "daily" | 日常会话 |
| `Conversation` | "conversation" | 对话上下文 |
| `Custom(String)` | 任意字符串 | 自定义分类 |

**MemoryKind 枚举**(`memory.rs:142`):
- `Episodic` — 场景记忆(何时何地发生何事)
- `Semantic(SemanticSubType)` — 语义记忆;子类型:`Preference` / `Fact` / `Decision` / `Entity`
- `Procedural` — 程序性记忆(如何做)

### 2.4 后端实现

| 后端 | MemoryBackendKind | 状态 | 说明 |
|---|---|---|---|
| SQLite | `Sqlite`(默认) | ✅ | FTS5 + 向量检索 + Embedding 缓存 |
| Markdown | `Markdown` | ✅ | 单文件追加,无召回能力,仅 list/get |
| None | `None` | ✅ | 空实现,所有方法返回空/默认 |
| AgentScoped | — | ✅ | 装饰器,为 Sqlite/Markdown 附加 agent_id + allowlist 隔离 |
| AgentScopedMarkdown | — | ✅ | 装饰器,一个 own + N 个 peer 的 Markdown 组合 |
| Postgres / Lucid / Qdrant | 枚举存在 | ❌ | 代码占位,未实现 |

### 2.5 召回检索模式(SearchMode)

`shadow-config/src/schema.rs:587-594`:

| 模式 | 含义 |
|---|---|
| `Bm25` | 仅关键词(FTS5) |
| `Embedding` | 仅向量(余弦相似度) |
| `Hubrid`(默认,**拼写如此**) | 混合检索,vector_weight + keyword_weight 加权融合 |

### 2.6 Memory 降级与缺省

| 场景 | 降级/缺省逻辑 |
|---|---|
| 后端识别失败 | `classify_memory_backend` 对未知类型 **降级为 Markdown** (`lib.rs:237-240`) |
| 未配置 memory.backend | 默认 `"sqlite"` (`schema.rs:72`) |
| 短期查询被识别为"近期/时间召回" | `is_recent_recall_query` 把 `""` 或 `"*"` 归一化为空 query (`memory.rs:480-492`) |
| recall_namespaced 未命中足够条数 | 先取 `limit*2` 再过滤,保证不漏 |
| `export` 未命中过滤项 | 静默返回空 Vec |
| Embedding 模型配置缺失 | `resolve_embedding_config` 回退到 `config.memory.embedding_*` 字段 |
| Embedding hint 路由未命中 | 回退到 fallback() |
| Agent UUID 后端不支持 | 默认实现返回 alias 本身 |
| purge_xxx 后端不支持 | 默认 `bail!("not supported")`,非静默失败 |
| Markdown 后端 recall | 实现为线性扫描前 N 条(无相关度排序) |

### 2.7 Memory 配置项 vs 硬编码

**可配置(`[memory]` 段,`schema.rs:514-557`)**:

| 字段 | 类型 | 默认 | 含义 |
|---|---|---|---|
| `backend` | String | ""(运行时默认 sqlite) | 后端选择 |
| `auto_save` | bool | false | 对话结束自动落库 |
| `hygiene_enabled` | bool | false | 启用治理(归档/清理) |
| `archive_after_days` | u32 | 0 | 归档阈值(0=禁用) |
| `purge_after_days` | u32 | 0 | 清理阈值(0=禁用) |
| `conversation_retention_days` | u32 | 0 | 会话类保留天数 |
| `core_retention_days` | u32 | 0 | 核心类保留天数 |
| `daily_retention_days` | u32 | 0 | 日常类保留天数 |
| `embedding_provider` | String | "" | Embedding 模型 provider |
| `embedding_model` | String | "" | Embedding 模型名(支持 `"hint:xxx"` 路由) |
| `embedding_dimensions` | usize | 0 | 向量维度(0=使用模型默认) |
| `embedding_api_key` | Option<String> | None | Embedding 专用 key |
| `vector_weight` | f64 | 0.0(运行时填) | 向量权重 |
| `keyword_weight` | f64 | 0.0(运行时填) | 关键词权重 |
| `min_relevance_score` | f64 | 0.0 | 召回最低相关度 |
| `embedding_cache_size` | usize | 0 | Embedding 缓存大小 |
| `search_mode` | SearchMode | Hubrid | 检索模式 |

**可配置(`[storage.sqlite.<alias>]`,`schema.rs:438-443`)**:
- `path: Option<String>` — 自定义 sqlite 路径
- `open_timeout_secs: Option<u64>` — 打开超时

**可配置(`[[embedding_routes]]`,`schema.rs:573-585`)**:按 hint 映射到具体 provider/model/dimensions/api_key。

**硬编码常量**:
- 默认 namespace 字符串:`"default"` (`memory.rs:137`)
- `AgentScopedMemory` allowlist 上限:无显式上限,跟随配置
- `recall_namespaced` 倍数:`limit * 2` (`memory.rs:349`)

---

## 3. Runtime / AgentRuntime 子系统需求

### 3.1 运行环境适配(RuntimePlatformAdapter)

`shadow-core/src/runtime.rs:6`:

| 方法 | 默认 | 含义 |
|---|---|---|
| `name()` | — | 环境标识(native/docker/cloudflare) |
| `has_shell_access()` | — | 是否可执行 shell |
| `has_filesystem_access()` | — | 文件系统读写 |
| `storage_path()` | — | 持久化目录 |
| `supports_long_running()` | — | 长任务(否则 serverless) |
| `memory_budget()` | 0(无限制) | 最大可用内存(字节) |
| `build_shell_command()` | — | 构造 shell 子进程 |

**实现**(`crates/shadow-config/src/platform/`):

| 实现 | 状态 | 关键行为 |
|---|---|---|
| `NativeRuntime` (`native.rs:45`) | ✅ | shell=true, fs=true, long_running=true, 存储在 `~/.shadow`,Unix 用 `sh -c`,Android 固定 `/system/bin/sh`,Windows 用 `cmd.exe /C` |
| `DockerRuntime` (`docker.rs:8`) | ✅ | shell=true, fs=`mount_workspace`, long_running=false, 存储在 `/workspace/.shadow` 或 `/tmp/.shadow` |
| `CloudflareRuntime` | ❌ | `create_runtime` 直接 `bail!("not implemented")` |

### 3.2 Agent 主循环(run 函数)

**入口**:`shadow-runtime/src/agent/loop_.rs:20`

```rust
pub async fn run(
    config: Config,
    agent_alias: &str,
    message: Option<String>,
    temperature: Option<f64>,
    interactive: bool,
    session_state_file: Option<PathBuf>,
    allowed_tools: Option<Vec<String>>,
    overrides: AgentRuntimeOverrides,
) -> anyhow::Result<String>
```

**主循环阶段**(基于 `agent_bak.rs` 注释代码与 `loop_.rs` 脚手架归纳,阶段已定稿):

| 阶段 | 状态 | 说明 |
|---|---|---|
| 1. Agent 配置解析 | ✅ | `resolved_agent_config(agent_alias)` |
| 2. Risk profile 解析 | ✅ | `risk_profile_for_agent` |
| 3. Memory 后端判定 | ✅ | 根据 MemoryBackendKind 分支 |
| 4. 平台适配器构造 | ✅ | `create_runtime(config)` |
| 5. Provider 解析 | ✅ | `create_model_provider` |
| 6. Memory 装配 / overrides 应用 | ✅ | overrides 优先 |
| 7. 工具调用循环 | ⚠️ | `loop_.rs` 当前提前返回 `"exit"`,完整循环逻辑在 `agent_bak.rs` 注释中 |
| 8. 历史截断 + Token 预算 | 🚧 | 默认 max_history=50, context_token_budget=100_000 |
| 9. Memory 召回注入 system prompt | 🚧 | `MemoryStrategy::before_chat` |
| 10. LLM 调用(上下文溢出恢复) | 🚧 | — |
| 11. 工具执行(并行/串行) | 🚧 | 需审批的工具串行,其余 `futures::join_all` 并行 |
| 12. 循环检测 | ✅ | `LoopDetector` 已完整实现 |
| 13. 收尾:历史/session 落盘 + `after_chat` + skill review | 🚧 | — |

**`AgentRuntimeOverrides`**(`loop_.rs:14`):
```rust
pub struct AgentRuntimeOverrides {
    pub security: Option<Arc<SecurityPolicy>>,
    pub memory: Option<Arc<dyn Memory>>,
    pub is_subagent: bool,
}
```

### 3.3 循环检测(LoopDetector)

`shadow-runtime/src/agent/loop_detector.rs`,滑动窗口默认 20 条。

| 检测模式 | Warning | Block | Break |
|---|---|---|---|
| 精确重复(同工具+同参数+同结果) | 连续 3 次 | 4 次 | 5 次 |
| 乒乓(两工具 A-B-A-B 交替) | 4 个周期(8 次) | 5 周期 | 6+ 周期 |
| 无进展(同工具+同结果,参数可变) | 窗口内 5 次 | 6 次 | 7+ 次 |

输出:`LoopDetectionResult::{Ok, Warning(String), Block(String), Break(String)}`。

### 3.4 工具系统

**Tool trait**(`shadow-core/src/kennel/tool.rs:75`):
```rust
async fn execute(&self, args: serde_json::Value) -> Result<ToolResult>;
fn name(&self) -> &str;
fn description(&self) -> &str;
fn parameters_schema(&self) -> serde_json::Value;
fn spec(&self) -> ToolSpec;  // 默认实现
```

`ToolResult { success, output, error }`。

**ToolRegistry**(`tools/registry.rs`):`register / extend / find / specs / len / is_empty / iter`。

**内置工具现状**:

| 工具 | 状态 | 说明 |
|---|---|---|
| `cron_add` | ✅ | `CronAddTool`,添加定时任务(shell/agent) |
| `cron_list` / `cron_run` / `cron_runs` / `cron_remove` | 🚧 | 文件存在但为空 |
| `shell` / `file_read` / `file_write` | 🚧 | README 声称已实现,实际 `default_tools_with_workspace` 返回空 `ToolRegistry` |
| `memory_*` / `spawn_subagent` / `sop_*` | 🚧 | `ToolKind` 枚举已声明,无实现 |

**ToolKind 枚举**(`kennel/attribution.rs:86`):Shell / HttpRequest / HttpServer / FetchUrl / Search / Memory / SpawnSubAgent / SopList / SopExecute / SopApprove / SopAdvance / SopStatus / SopHistory / Wait / Plugin。

### 3.5 工具协议派发(ToolDispatcher)

`shadow-runtime/src/dispatcher.rs:10`:
```rust
fn parse_response(&self, response: &ChatResponse) -> (String, Vec<ToolCall>);
fn format_results(&self, results: &[ToolResult]) -> ChatMessage;
fn should_send_tool_specs(&self) -> bool;
```

| 实现 | 用途 | parse | format_results role | should_send_specs |
|---|---|---|---|---|
| `NativeToolDispatcher` | OpenAI/Anthropic 原生函数调用 | 读 `response.tool_calls` | `"tool"` | true |
| `XmlToolDispatcher` | 无原生工具的 Provider | 解析 `<tool_call>JSON</tool_call>` | `"user"` + `[tool_result]...[/tool_result]` | false(已在 system prompt 描述) |

### 3.6 安全(SecurityPolicy + Sandbox)

**SecurityPolicy**(`security/mod.rs:139`):

| 字段 | 含义 | 默认 |
|---|---|---|
| `autonomy` | 自主级别 | Supervised |
| `blocked_patterns` | 危险命令黑名单(子串或正则) | 见下表 |
| `allowed_env_vars` | 环境变量白名单 | 见下表 |
| `workspace` | 工作区路径限定 | None |
| `allowed_commands` | 命令白名单(空=全部允许) | 空 |
| `forbidden_paths` | 禁止访问路径 | 见下表 |
| `block_high_risk_commands` | 阻断高风险 | true |
| `require_approval_for_medium_risk` | 中风险需审批 | true |

**硬编码默认黑名单**(15 条,`security/mod.rs:21`):
`rm -rf /`、`rm -rf ~`、`rm -rf *`、`mkfs`、`dd if=`、`> /dev/sd`、`> /dev/nvme`、fork bomb、`curl.*|.*sh`、`wget.*|.*sh`、`chmod 777 /`、`shutdown`、`reboot`、`init 0`、`init 6`。

**硬编码默认 forbidden_paths**(6 条):`/etc`、`/root`、`/boot`、`/sys`、`/proc`、`/dev`。

**硬编码默认 env 白名单**(10 条):`PATH`、`HOME`、`TERM`、`LANG`、`LC_ALL`、`LC_CTYPE`、`USER`、`SHELL`、`TMPDIR`、`PWD`。

**Sandbox**:`Sandbox` trait 只有 `NoopSandbox`(透传)实现。Firejail / Namespace 为未来计划。

**AutonomyLevel**(`shadow-core/src/lib.rs:43`):
- `Full` — 完全自主
- `Supervised`(默认)— 敏感操作需审批
- `ReadOnly` — 拒绝写操作

### 3.7 Prompt 工程(SystemPromptBuilder)

`shadow-runtime/src/prompt/mod.rs:82`,基于分段(`PromptSection`)按 `priority()` 降序拼接。

| Section | 优先级 | 作用 | 状态 |
|---|---|---|---|
| `IdentitySection` | 100 | "You are {alias}..." | ✅ |
| `PersonaSection` | 99 | 10 个内置 persona,配置可覆盖 | ✅ |
| `BootstrapSection` | 95 | 载入 AGENTS.md / SOUL.md / IDENTITY.md / USER.md(各 20KB 上限) | ✅ |
| `DateTimeSection` | 90 | 当前时间戳 | ✅ |
| `WorkspaceSection` | 80 | 当前工作目录 | ✅ |
| `SafetyInjectionSection` | 75 | 把 SecurityPolicy 渲染成具体约束 | ✅ |
| `SafetySection` | 70 | 通用安全 + autonomy 描述 | ✅ |
| `ToolHonestySection` | 60 | "禁止伪造工具结果" | ✅ |

**子模块状态**:

| 子模块 | 作用 | 状态 |
|---|---|---|
| `bootstrap.rs` | 身份文件加载 + 注入守卫 | ✅ |
| `caching.rs` | Anthropic prompt caching(system + 末 3 条消息,4 个断点,1h TTL) | ✅ |
| `context_compressor.rs` | 1 token ≈ 4 字符;裁剪老工具结果 | ✅ |
| `injection_guard.rs` | 10 条注入正则 + 10 条不可见 Unicode 检测 | ✅ |
| `persona.rs` | 10 个内置 persona + 配置覆盖 | ✅ |
| `prompt_guided.rs` | 无原生工具时的 `<tool_call>` 注入与解析 | ✅ |
| `safety_injection.rs` | SecurityPolicy → prompt | ✅ |
| `tools_payload.rs` | OpenAI / Anthropic / PromptGuided 三格式 | ✅ |
| `truncation.rs` | 工具输出截断(头 1/3 + 尾 1/3,UTF-8 安全) | ✅ |

### 3.8 Skills 子系统

`shadow-runtime/src/skills/mod.rs`,从 `SKILL.md`(YAML frontmatter + Markdown)加载。

| 功能 | 状态 |
|---|---|
| `parse_skill_md` / `split_frontmatter` | ✅ |
| `load_skills_from_dir` | ✅ |
| 目录扫描加载 | ❌(扫描路径被注释) |
| `all_tools` 工具注册 | ❌(shell/http/builtin 分支全注释) |
| SkillBundle 过滤(include/exclude) | ✅(`SkillBundleConfig::admits_skill`) |

### 3.9 Cron 子系统

`shadow-runtime/src/cron/`:

| 功能 | 状态 |
|---|---|
| 表达式校验(5/6/7 字段 + 时区) | ✅ `schedule.rs` |
| `At` 一次性 / `Every` 重复 | ✅ |
| `next_run_for_schedule` | ✅ |
| `add_shell_job_with_approval` | ❌(stub,直接 bail) |
| `add_agent_job` | ⚠️(校验通过但不落库) |
| 持久化执行 | ❌ |
| 投递渠道 | 仅 "feishu" |

`CronRun.output` 截断:10KB。

### 3.10 Runtime 配置项 vs 硬编码

**可配置(`[runtime]` 段,`schema.rs:614-664`)**:

| 字段 | 类型 | 默认 | 含义 |
|---|---|---|---|
| `kind` | RuntimeKind | Native | native / docker / cloudflare |
| `shell` | Option<String> | None(用 "sh") | 仅 native 生效 |
| `reasoning_enabled` | Option<bool> | None | 全局 reasoning 开关 |
| `reasoning_effort` | Option<String> | None | minimal/low/medium/high/xhigh |
| `[runtime.docker]` | 子表 | 见下 | Docker 运行时配置 |

**`[runtime.docker]` 默认**:

| 字段 | 默认 |
|---|---|
| `image` | "alpine:3.20" |
| `network` | "none" |
| `memory_limit_mb` | 512 |
| `cpu_limit` | 1.0 |
| `read_only_rootfs` | true |
| `mount_workspace` | true |
| `allowed_workspace_roots` | [] |

**可配置(`[[risk_profiles.<name>]]`,`multi/risk_profile.rs:6`)**:

| 字段 | 默认 |
|---|---|
| `level` (AutonomyLevel) | Supervised |
| `workspace_only` | true |
| `allowed_commands` | [] |
| `forbidden_paths` | [] |
| `require_approval_for_medium_risk` | true |
| `block_high_risk_commands` | true |
| `shell_env_passthrough` | [] |
| `auto_approve` | [] |
| `always_ask` | [] |
| `allowed_roots` | [] |
| `delegation_policy` (DelegationMode) | Forbidden |
| `approval_route` | None |
| `allow_tools` / `excluded_tools` | [] |
| `sandbox_enabled` / `sandbox_backend` | None |
| `firejail_args` | [] |

**可配置(`[[runtime_profiles.<name>]]`,`multi/runtime_profile.rs:5`)**:

| 字段 | 默认 | 含义 |
|---|---|---|
| `agentic` | false | 是否 Agent 模式 |
| `max_tool_iterations` | 0(运行时填 10) | 工具调用上限 |
| `max_actions_per_hour` | 30 | 限流 |
| `max_cost_per_day_cents` | 500 | 每日成本上限 |
| `shell_timeout_secs` | 60 | shell 超时 |
| `max_delegation_depth` | 0 | 委派深度 |
| `delegation_timeout_secs` | None | 委派超时 |
| `agentic_timeout_secs` | None | Agent 超时 |
| `max_history_messages` | None | 历史条数 |
| `max_context_tokens` | None | 上下文 token |
| `compact_context` | None | 压缩 |
| `parallel_tools` | None | 并行工具 |
| `tool_dispatcher` | None | 派发器 |
| `tool_call_dedup_exempt` | [] | 去重豁免 |
| `max_system_prompt_chars` | None | system prompt 上限 |
| `max_tool_result_chars` | None | 工具结果上限 |
| `keep_tool_context_turns` | None | 保留工具上下文轮次 |
| `memory_recall_limit` | None | 召回条数 |
| `strict_tool_parsing` | false | 严格解析 |

**硬编码常量**:
- LoopDetector 窗口:20 条
- LoopDetector 阈值:见 §3.3
- `context_token_budget`:100_000(0 = 无限制)
- `max_history`:50 条
- Bootstrap 文件单文件上限:20_000 字符
- ToolResult 输出截断:头/尾各 1/3
- CronRun 输出截断:10KB
- 默认 shell:"sh"(Unix)/"cmd.exe"(Windows)/"/system/bin/sh"(Android)
- ToolKind 枚举:14 种 + Plugin

---

## 4. Provider 子系统需求

### 4.1 功能性需求

| ID | 需求 | 状态 | 来源 |
|---|---|---|---|
| PRV-01 | simple_chat(单轮) | ✅ | `provider.rs:259` |
| PRV-02 | chat_with_system(带 system prompt) | ✅ | `provider.rs:260` |
| PRV-03 | chat_with_history(多轮) | ✅ | `provider.rs:262` |
| PRV-04 | chat(ChatRequest 完整) | ✅ | `provider.rs:263` |
| PRV-05 | chat_with_tools(工具调用) | ✅ | `provider.rs:264` |
| PRV-06 | 流式(stream_chat 等) | ✅ | `provider.rs:266-268` |
| PRV-07 | list_models | ✅(默认空) |
| PRV-08 | warmup | ✅(默认空) |
| PRV-09 | Provider 能力声明(capabilities) | ✅ | `provider.rs:202` |
| PRV-10 | 工具格式转换(OpenAI/Anthropic/PromptGuided) | ✅ | `tools_payload.rs` |
| PRV-11 | 无原生工具时自动注入 prompt | ✅ | `provider.rs:341` 默认实现 |

### 4.2 多 Provider 路由(三层架构)

| 层 | 状态 | 说明 |
|---|---|---|
| Router(顶层,按 hint 路由 + 跨 provider fallback) | ❌ | `router.rs` 全文注释 |
| Reliable(中层,重试 + 退避 + key 轮换 + 限流) | ❌ | `reliable.rs` 全文注释 |
| Compat(底层,OpenAI 兼容协议适配) | ✅ | `openai.rs` + `factory.rs` + `dispatch.rs` |

> ⚠️ **当前生产路径**: `create_model_provider` 直接构造 `OpenAiCompatibleModelProvider`,**不经过 Router/Reliable 层**。所有重试 / fallback / 限流 / key 轮换能力均未生效。

### 4.3 Provider 降级与缺省

| 场景 | 降级/缺省逻辑 | 状态 |
|---|---|---|
| `ProviderRef::parse("openai")` 缺省 alias | 默认 `"default"` | ✅ |
| `default_base_url(family)` | 10 个家族内置 URL,custom 返回 None | ✅ |
| `ResolvedProvider::effective_model` | entry.model 或调用方 fallback | ✅ |
| `ResolvedProvider::effective_temperature` | entry.temperature 或 **0.7** | ✅ |
| 无原生工具支持 | 注入 PromptGuided 文本到 system prompt | ✅ |
| 模型级 fallback(遍历 fallback_models) | 设计完备 | ❌(注释) |
| 跨 provider fallback(fallback_chains) | 设计完备 | ❌(注释) |
| Key 轮换(Auth 错误立即切换) | 设计完备 | ❌(注释) |
| 重试退避(指数 + jitter) | 设计完备 | ❌(注释) |
| RPM 限流(TokenBucket) | 实现完备 | ❌(未接入) |
| ChatError 分类(5xx/429/401/4xx/网络) | 实现完备 | ❌(未接入) |

**已设计的 ReliableConfig 默认**(`providers.rs:328`,生效需要解封 reliable.rs):

| 字段 | 默认 |
|---|---|
| `max_retries` | 3 |
| `initial_backoff_ms` | 1000 |
| `max_backoff_ms` | 60000 |
| `jitter_pct` | 25 |
| `requests_per_minute` | 0(不限) |

### 4.4 Provider 配置项 vs 硬编码

**可配置(`[providers.models.custom.<alias>]`,`model_provider/mod.rs:10`)**:

| 字段 | 类型 | 默认 |
|---|---|---|
| `api_key` / `api_keys` | Option<String> / Vec<String> | None |
| `kind` | Option<String> | None |
| `uri` | Option<String> | None(用 family 默认) |
| `model` | Option<String> | None |
| `temperature` | Option<f64> | None(用 0.7) |
| `timeout_secs` | Option<u64> | None |
| `extra_headers` | HashMap | {} |
| `response_max_tokens` | Option<u32> | None |
| `native_tools` | Option<bool> | None |
| `think` | Option<bool> | None |
| `context_window` | Option<usize> | None |

**硬编码常量**(`kennel/provider.rs:236-239`):
- `BASE_TEMPERATURE = 0.7`
- `BASE_MAX_TOKEN = 4096`
- `BASE_TIMEOUT_SECS = 120`
- `BASE_WIRE_API = "chat_completions"`
- OpenAI 兼容 Provider `timeout_secs`:60(`openai.rs`)
- AuthStyle:Bearer(默认)
- ProviderCapabilities 默认:全 false

---

## 5. 其它子系统需求

### 5.1 Config 加载与迁移

| 需求 | 状态 | 说明 |
|---|---|---|
| TOML/JSON 加载(`load_or_init`) | ✅ | 支持 SHADOW_CONFIG_DIR / SHADOW_DATA_DIR / SHADOW_WORKSPACE 环境变量 |
| macOS Homebrew 路径识别 | ✅ | `<prefix>/var/shadow` |
| schema 迁移 | ✅ | v1→v2:`api_key` → `api_keys` 数组;`CURRENT_SCHEMA_VERSION = 2` |
| Secret 加密 | ✅ | ChaCha20-Poly1305 AEAD,密钥 `<shadow_dir>/.secret_key`(0600) |
| Config Default | ✅ | 内置默认 agent="assistant",MiniMax-M2.7 |

**路径优先级**:`SHADOW_CONFIG_DIR` > `~/.shadow` > Homebrew prefix。

### 5.2 Session 存储

`shadow-core/src/session_store.rs`,`SessionStore` trait 6 方法:`load / append_message / save / delete / list / list_with_metadata`。

**JsonlSessionStore**:
- `{workspace}/sessions/{id}.jsonl` — 一行一条消息
- `{workspace}/sessions/{id}.meta.json` — 元数据 sidecar
- `append_message`:追加一行 + 更新 sidecar
- `list`:按 mtime 倒序
- `list_with_metadata`:无 sidecar 时从文件 mtime + 行数推导(向后兼容)

### 5.3 Channel

`shadow-core/src/channel.rs:29`,`Channel` trait:
- `name()` / `send(&SendMessage)` / `supports_approval()`(默认 false)
- 当前无具体实现(仅 trait + 数据结构)

### 5.4 Workspace 目录布局

`shadow-core/src/workspace.rs`:

| 路径 | 用途 |
|---|---|
| `{root}/sessions/` | 会话 JSONL |
| `{root}/memory/` | 记忆 |
| `{root}/memory/brain.db` | SQLite 记忆库 |
| `{root}/memory/MEMORY.md` | Markdown 记忆 |
| `{root}/skills/` | 技能 |
| `{root}/logs/runtime-trace.jsonl` | 追踪日志 |
| `{root}/workspace/` | 工作区根 |
| `{root}/cron.db` | 定时任务 |
| `{root}/config.toml` | 配置 |
| `{root}/SOUL.md` | 身份文件 |

`ensure_layout()` 幂等创建 sessions/memory/skills/logs/workspace。

### 5.5 Observability

`shadow-config/src/observability.rs`:`ObservabilityBackend::{None(默认), Prometheus}`。Prometheus 后端未实现。

### 5.6 Scheduler

`shadow-config/src/schema.rs:446-458`:

| 字段 | 默认 |
|---|---|
| `enabled` | true |
| `max_tasks` | 64 |
| `max_concurrent` | 4 |
| `catch_up_on_startup` | false |
| `max_run_history` | 50 |

### 5.7 Agent(别名)配置

`shadow-config/src/multi/alias_agent.rs:22`:

| 字段 | 默认 |
|---|---|
| `enabled` | true |
| `workspace.path` | None(用默认) |
| `workspace.access` | BTreeMap<AgentAlias, AccessMode> |
| `workspace.unrestricted_filesystem` | false |
| `workspace.read_memory_from` | [](其他 agent 的 peer 列表) |
| `memory.backend` | MemoryBackendKind::Sqlite |
| `model_provider` | 必填(ModelProviderRef) |
| `risk_profile` | RiskProfileRef |
| `runtime_profile` | RuntimeProfileRef |

**AccessMode**:`Read` / `Write` / `ReadWrite`。

---

## 6. 跨模块:配置 vs 硬编码总表

### 6.1 可配置项汇总

| 模块 | 配置入口 | 字段数 | 关键默认 |
|---|---|---|---|
| Agent | `[agents.<alias>]` | 5+ | backend=Sqlite, enabled=true |
| Memory | `[memory]` | 17 | backend="sqlite"(运行时) |
| Storage | `[storage.sqlite.<alias>]` | 2 | — |
| Embedding | `[memory.embedding_*]` + `[[embedding_routes]]` | 4+4 | — |
| Provider | `[providers.models.custom.<alias>]` | 11 | temperature=0.7 |
| Runtime | `[runtime]` + `[runtime.docker]` | 4+7 | kind=Native, image=alpine:3.20 |
| Risk | `[[risk_profiles.<name>]]` | 17 | level=Supervised |
| RuntimeProfile | `[[runtime_profiles.<name>]]` | 19 | agentic=false |
| Scheduler | `[scheduler]` | 5 | max_tasks=64 |
| Observability | `[observability]` | 1 | backend=None |
| SkillBundle | `[[skill_bundles.<name>]]` | 3 | include=[] 表示全部 |

### 6.2 硬编码常量汇总

| 常量 | 值 | 位置 |
|---|---|---|
| BASE_TEMPERATURE | 0.7 | `provider.rs:236` |
| BASE_MAX_TOKEN | 4096 | `provider.rs:237` |
| BASE_TIMEOUT_SECS | 120 | `provider.rs:238` |
| Provider HTTP timeout | 60s | `openai.rs` |
| LoopDetector 窗口 | 20 | `loop_detector.rs` |
| max_history | 50 条 | agent_bak.rs |
| context_token_budget | 100_000 tokens | agent_bak.rs |
| Bootstrap 文件上限 | 20_000 字符/文件 | `prompt/bootstrap.rs` |
| Token 估算 | 1 token ≈ 4 字符 | `prompt/context_compressor.rs` |
| 工具结果截断 | 头 1/3 + 尾 1/3 | `prompt/truncation.rs` |
| Cron 输出截断 | 10KB | `cron/mod.rs:147` |
| 默认 shell(Unix) | "sh" | `platform/native.rs` |
| 默认 shell(Android) | "/system/bin/sh" | `platform/native.rs` |
| 默认 shell(Windows) | "cmd.exe" | `platform/native.rs` |
| Docker 默认镜像 | "alpine:3.20" | `schema.rs:698` |
| Docker 默认网络 | "none" | `schema.rs:702` |
| Docker 默认内存 | 512MB | `schema.rs:706` |
| Docker 默认 CPU | 1.0 | `schema.rs:710` |
| 密钥文件权限(Unix) | 0600 | `secrets.rs` |
| 默认 namespace | "default" | `memory.rs:137` |
| 密文前缀 | "enc2:" | `secrets.rs` |
| 危险命令黑名单 | 15 条 | `security/mod.rs:21` |
| 禁止路径 | 6 条 | `security/mod.rs:43` |
| ENV 白名单 | 10 条 | `security/mod.rs:56` |
| Schema 版本 | 2 | `migration.rs` |

---

## 7. 跨模块:降级与缺省总表

### 7.1 静默降级(无错继续)

| 场景 | 降级到 |
|---|---|
| Memory 后端字符串识别失败 | Markdown |
| MemoryBackendKind 未在 builder 中匹配 | Markdown |
| recall_namespaced / recall_for_agents 未实现 | 默认 trait 实现兼容 |
| purge_xxx / rename_agent 不支持 | 默认 `bail!`(显式失败) |
| Embedding hint 路由未命中 | fallback 到 `[memory.embedding_*]` |
| Embedding 模型字段为空 | 用 NoopEmbeddingProvider |
| ChatRequest 有 tools 但 Provider 无原生工具 | PromptGuided 注入文本 |
| Provider `list_models`/`warmup` 缺省 | 返回空 Vec / Ok(()) |
| Bootstrap 身份文件缺失 | 跳过该 section |
| SkillBundle include 为空 | 包含全部 |
| runtime_profiles 字段为 0 | 运行时填默认(如 max_tool_iterations=10) |
| `[runtime] kind` 解析失败 | 默认 Native |
| Config `schema_version` 缺失 | 视为 v0,迁移至 v2 |
| Secret 非加密前缀 | 透传(向后兼容) |

### 7.2 显式失败(不降级)

| 场景 | 行为 |
|---|---|
| `cloudflare` runtime | `bail!("not implemented")` |
| Provider family 未识别 | `dispatch_family_factory` 报错 |
| Memory `purge_*` 默认实现 | `bail!("not supported")` |
| Postgres backend 走 builder 入口 | `bail!("requires storage config")` |
| Docker workspace mount 根 `/` | 拒绝 |
| Shell 配置非法(空/相对多段/不存在) | 启动时报错 |
| `cron add announce` 模式缺 `to` | `bail!` |
| 注入守卫命中不安全内容 | 替换为 `[BLOCKED: ...]` |

### 7.3 设计存在但未启用的降级

以下能力**代码已写完但被注释**,启用前不应视为可用:

| 能力 | 文件 | 状态 |
|---|---|---|
| RouterModelProvider(跨 provider fallback) | `shadow-providers/src/router.rs` | 全文注释 |
| ReliableModelProvider(重试/key 轮换/限流) | `shadow-providers/src/reliable.rs` | 全文注释 |
| Agent struct + 完整工具循环 | `shadow-runtime/src/agent_bak.rs` | 全文注释(1018 行) |
| Postgres / Lucid / Qdrant 后端 | `shadow-memory/src/lib.rs:223-235` | 注释 |
| Skills `all_tools` 注册 | `shadow-runtime/src/skills/mod.rs` | 分支注释 |
| Skills 目录扫描加载 | `shadow-runtime/src/skills/mod.rs` | 路径注释 |
| Cron `list/run/runs/remove` 工具 | `shadow-runtime/src/tools/cron/` | 空文件 |
| Cron 任务持久化执行 | `shadow-runtime/src/cron/` | stub |
| FirejailSandbox / NamespaceSandbox | `shadow-runtime/src/security/mod.rs` | 仅声明 |
| Prometheus Observability | `shadow-config/src/observability.rs` | 仅枚举 |
| CloudflareRuntime | `shadow-config/src/platform/mod.rs` | `bail!` |
| shadow-proxy / shadow-gateway / shadow-channels / shadow-spawn crate | `crates/shadow-*/src/lib.rs` | 空壳 |

---

## 8. 需求覆盖度自检

### 8.1 子系统成熟度

| 子系统 | trait/数据 | 实现 | 测试 | 评级 |
|---|---|---|---|---|
| Memory(SQLite) | ✅ | ✅ | ✅(25 项集成测试,见 `shadow-memory/tests/`) | A |
| Memory(Markdown/None/Scoped) | ✅ | ✅ | ⚠️ | B |
| Provider(OpenAI 兼容) | ✅ | ✅ | ⚠️ | B |
| Provider(Router/Reliable) | ✅ | ❌(注释) | — | D |
| RuntimePlatformAdapter(Native/Docker) | ✅ | ✅ | ⚠️ | B |
| Agent loop | ⚠️ | ❌(注释) | — | D |
| LoopDetector | ✅ | ✅ | ⚠️ | B |
| ToolDispatcher | ✅ | ✅ | ⚠️ | B |
| Tool 内置集 | ✅ | ❌(仅 cron_add) | — | D |
| Security(Policy) | ✅ | ✅ | ⚠️ | B |
| Security(Sandbox) | ⚠️ | ❌(Noop) | — | D |
| Prompt(9 子模块) | ✅ | ✅ | ⚠️ | A |
| Skills | ✅ | ⚠️(仅解析) | — | C |
| Cron | ✅ | ⚠️(仅校验+add) | — | C |
| Session(JSONL) | ✅ | ✅ | ⚠️ | B |
| Config(加载/迁移/Secret) | ✅ | ✅ | ⚠️ | A |
| Observability | ⚠️ | ❌ | — | D |
| Channel | ⚠️ | ❌(仅 trait) | — | D |

### 8.2 关键缺口(按优先级)

**P0(阻塞主流程)**:
1. **Agent loop 启用**:`agent_bak.rs` → `loop_.rs` 的工具调用循环当前是 stub,run() 提前返回
2. **内置工具回归**:`shell` / `file_read` / `file_write` README 声明已实现,实际 `default_tools_with_workspace` 返回空
3. **Cron 任务持久化与执行**:当前 add 不落库

**P1(影响可靠性)**:
4. **Reliable/Router 启用**:重试、key 轮换、fallback、限流均未生效
5. **Skills 工具注册**:目录扫描与 shell/http/builtin 注册全注释
6. **Cron list/run/runs/remove**:空文件

**P2(完善生态)**:
7. Sandbox 非 Noop 实现(Firejail/Namespace)
8. Prometheus Observability
9. Postgres / Lucid / Qdrant Memory 后端
10. CloudflareRuntime
11. Channel 具体实现
12. shadow-proxy / shadow-gateway / shadow-channels / shadow-spawn 填充

### 8.3 命名一致性修正建议

- README 与多处文档称 `AgentRuntime` trait,实际不存在 → 建议统一为 `RuntimePlatformAdapter` + `run()` + `AgentRuntimeOverrides`
- `SearchMode::Hubrid` 拼写 → 建议改为 `Hybrid`(需同步配置反序列化)
- `StreamEvent::TextDelte` 拼写 → 建议改为 `TextDelta`

---

## 附录 A:文档来源映射

本需求文档的每一条需求可直接溯源到代码:

- Memory: `crates/shadow-core/src/kennel/memory.rs`、`crates/shadow-memory/src/{lib.rs,sqlite/*.rs,markdown.rs,none.rs,agent_scoped*.rs,strategy.rs,vector.rs,embedding.rs,conflict.rs}`
- Runtime: `crates/shadow-core/src/runtime.rs`、`crates/shadow-runtime/src/{agent/*,dispatcher.rs,security/*,prompt/*,skills/*,cron/*,tools/*}`、`crates/shadow-config/src/platform/*`
- Provider: `crates/shadow-core/src/kennel/provider.rs`、`crates/shadow-providers/src/{lib.rs,factory.rs,dispatch.rs,router.rs,reliable.rs,rate_limit.rs,error.rs,openai.rs}`、`crates/shadow-config/src/providers.rs`
- Config: `crates/shadow-config/src/{lib.rs,schema.rs,providers.rs,migration.rs,secrets.rs,multi/*,model_provider/*,platform/*,autonomy.rs,observability.rs,proxy_client.rs}`
- 基础设施: `crates/shadow-core/src/{session_store.rs,workspace.rs,channel.rs,kennel/attribution.rs,kennel/observer.rs,kennel/tool.rs}`

