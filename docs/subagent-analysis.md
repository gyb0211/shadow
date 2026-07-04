# ZeroClaw SubAgent / Delegate 深度分析报告

## 概览

ZeroClaw 有两套 subagent 机制，各自独立但共享安全基座:

| 维度 | SubAgent (spawn_subagent) | Delegate (delegate) |
|------|---------------------------|----------------------|
| 本质 | 同身份子进程 (inherit parent) | 跨 agent 委派 (different agent) |
| 模型/Provider | 继承父 agent | 目标 agent 自己的 |
| 工具集 | 父 agent 工具 (但 spawn_subagent 被禁) | 目标 agent 工具 (Bounded) 或独立构建 (Independent) |
| 记忆 | 共享父 agent 记忆 | 目标 agent 记忆 + 跨 agent allowlist |
| 安全策略 | 继承父 (可收窄不可升) | 目标 agent 的 SecurityPolicy |
| 深度限制 | depth-1 cap (不能递归) | 可配置 max_depth (default 3) |
| 执行模式 | 同步 only | sync / background / parallel |
| 行数 | 458 + 629 = 1087 行 | 7316 行 |
| 配置 | 无 (运行时继承) | `[agents.<alias>]` + `delegates` |

---

## 1. SubAgent 模块 (subagent/mod.rs, 458行)

### 1.1 核心设计: 权限继承 + 只能收窄

```
父 Agent (alias="researcher", policy=P, memory_allowlist={researcher, coder})
    │
    ├── SubAgentSpawn::for_agent(config, "researcher")
    │       → 解析父 policy + 父 memory_allowlist
    │
    ├── SubAgentOverrides { policy: None, allowed_agent_aliases: None }
    │       → 继承父 verbatim (Arc clone, 零拷贝)
    │
    └── SubAgentOverrides { policy: Some(narrower), allowlist: Some(subset) }
            → ensure_no_escalation_beyond() 验证
            → 失败则 bail + 记录 Reject 日志
```

### 1.2 三层结构

```rust
// 1. Overrides -- 调用方提供的收窄参数 (None = 继承)
pub struct SubAgentOverrides {
    pub policy: Option<SecurityPolicy>,           // 安全策略收窄
    pub allowed_agent_aliases: Option<HashSet<String>>, // 记忆允许列表收窄
}

// 2. Context -- build() 后的已验证上下文
pub struct SubAgentContext {
    pub parent_alias: String,                     // 父 agent 别名
    pub policy: Arc<SecurityPolicy>,              // 已验证的子策略
    pub allowed_agent_aliases: HashSet<String>,   // 已验证的允许列表
}

// 3. Spawn -- 构建器, 从 config 解析父身份
pub struct SubAgentSpawn {
    pub parent_alias: String,
    pub parent_policy: Arc<SecurityPolicy>,
    pub parent_allowed_agent_aliases: HashSet<String>,
}
```

### 1.3 两个入口点

- `for_agent(config, alias)` -- 从 config 重建 policy, 用于 cron 调度 (无 live session)
- `for_agent_with_policy(config, alias, live_policy)` -- 用调用方的 live policy, 用于交互式 (ACP/IDE)
  - 关键: 保留 session-scoped 的 `workspace_dir` (issue #7263)

### 1.4 安全验证 (build)

1. Policy 收窄验证: `ensure_no_escalation_beyond()` -- 子不能有父没有的权限
2. Allowlist 子集验证: 子的每个 alias 必须在父的 allowlist 中
3. Action budget 共享: 子继承父的 `PerSenderTracker` (防止通过 spawn 绕过 max_actions_per_hour)

### 1.5 测试覆盖 (7个)

- for_agent 解析成功 / 未知 alias 报错
- 默认继承 = verbatim Arc 指针相等
- Policy 升级 (/secrets) 被拒
- Allowlist 外来 alias 被拒
- Allowlist 空子集合法 (但 parent_alias 总是被加回)
- Action budget 耗尽后子也被限 (防绕过)
- session workspace_dir 保留 (regression #7263)

---

## 2. SpawnSubagentTool (tools/spawn_subagent.rs, 629行)

### 2.1 工具签名

```
name: "spawn_subagent"
parameters: { "prompt": string (required) }
description: "Spawn an ephemeral SubAgent that inherits this agent's identity..."
```

### 2.2 执行流程

```
1. 深度检查: is_subagent_caller == true → 拒绝 (depth-1 cap)
2. Risk-profile 门控: allowed_tools 不含 spawn_subagent → 拒绝
3. 参数验证: prompt 非空
4. Action budget: enforce_tool_operation(Act, "spawn_subagent") → 消耗一个配额
5. SubAgentSpawn::for_agent_with_policy() → build(Overrides::default())
6. 生成 run_id (UUID)
7. 构建 AgentRunOverrides { security: Some(ctx.policy), is_subagent: true }
8. Control-plane 注册 (TaskRecord: Subagent/Running)
9. crate::agent::run(config, alias, prompt, ..., run_overrides) → 等待完成
10. Control-plane 更新状态 (Completed/Failed)
11. 返回 ToolResult
```

### 2.3 递归保护

- `is_subagent_caller: bool` -- 在 tool registry 构建时从 `AgentRunOverrides.is_subagent` 设置
- 子 agent 的 SpawnSubagentTool 实例带 `with_subagent_caller(true)`
- execute() 第一行检查: 如果是 subagent caller → 立即返回错误

### 2.4 REENTRANT_AGENT_TOOLS

```rust
// tools/mod.rs
pub const REENTRANT_AGENT_TOOLS: &[&str] = &["spawn_subagent", "delegate"];
```

- 这两个工具免于 per-turn 重复调用去重
- 允许同一 turn 内多次 spawn/delegate (扇出)
- 但 action budget 仍限制扇出总量

---

## 3. DelegateTool (tools/delegate.rs, 7316行)

### 3.1 核心结构 (17个字段!)

```rust
pub struct DelegateTool {
    agents: Arc<HashMap<String, AliasedAgentConfig>>,  // 所有 agent 配置
    security: Arc<SecurityPolicy>,                      // 调用方安全策略
    global_credential: Option<String>,                  // 全局 API key
    provider_runtime_options: ModelProviderRuntimeOptions,
    depth: u32,                                         // 委派深度
    parent_tools: Arc<RwLock<Vec<Arc<dyn Tool>>>>,      // 父工具列表
    runtime: Option<Arc<dyn RuntimeAdapter>>,            // 运行时适配器
    multimodal_config: MultimodalConfig,
    delegate_config: DelegateToolConfig,                // 超时配置
    workspace_dir: PathBuf,
    cancellation_token: CancellationToken,               // 级联取消
    memory: Option<Arc<dyn Memory>>,                    // 命名空间隔离
    providers_models: Arc<HashMap<...>>,                // 模型解析
    risk_profiles: Arc<HashMap<...>>,                   // 风险配置
    runtime_profiles: Arc<HashMap<...>>,                // 运行时配置
    skill_bundles: Arc<HashMap<...>>,                   // 技能包
    root_config: Option<Arc<Config>>,                   // 完整配置
    caller_alias: String,                               // 调用方别名 (排除自委派)
}
```

### 3.2 四种 Action

```rust
match action {
    "delegate"     => execute_sync / execute_background / execute_parallel
    "check_result" => handle_check_result(task_id)       // 查后台任务结果
    "list_results" => handle_list_results()              // 列所有后台任务
    "cancel_task"  => handle_cancel_task(task_id)        // 取消后台任务
}
```

### 3.3 三种执行模式

#### Sync (默认)
```
execute_sync(agent_name, prompt, args)
  → resolve_brain() → 构建 provider → 单次 LLM 调用 (带 timeout)
  → 或 execute_agentic() (如果 runtime_profile.agentic=true)
```

#### Background (background: true)
```
execute_background(agent_name, prompt, args)
  → 验证 + 深度检查
  → tokio::spawn → 结果写入 workspace/delegate_results/{task_id}.json
  → 立即返回 task_id
```

#### Parallel (parallel: ["agent_a", "agent_b"])
```
execute_parallel(agents, args)
  → futures::join_all: 多个 agent 并发执行同一 prompt
  → 返回所有结果
```

### 3.4 委派模式 (DelegateExecutionMode)

```rust
pub enum DelegateExecutionMode {
    Bounded,      // 父限界: 共享父 budget, agentic 工具受父 envelope 限制
    Independent,  // 独立: 目标 agent 自己的 policy/tool envelope, 像新开一个 chat
}
```

- Bounded (默认): 子 agent 用父的工具 allowlist 子集, 共享父的 action budget
- Independent: 子 agent 用自己的完整工具集 + 自己的 SecurityPolicy

### 3.5 深度控制

```rust
let max_depth = self.resolve_max_depth(&agent_config.runtime_profile);
if self.depth >= max_depth {
    return Err("Delegation depth limit reached ({depth}/{max})");
}
// 子 DelegateTool 构建时 depth = parent.depth + 1
```

### 3.6 Agentic 模式

当目标 agent 的 runtime_profile.agentic = true:
1. 解析目标 agent 的 tool policy (allowed_tools/excluded_tools)
2. 从父工具列表中筛选 allowlisted 工具
3. 构建 ToolLoop 并运行完整工具调用循环
4. 子 agent 有自己的 max_tool_iterations

### 3.7 Admission 模式

```rust
enum DelegateAdmission {
    Required,     // 公共入口: 运行完整授权 + 可达性检查
    Prevalidated, // 后台 worker: 父已授权, 跳过重复检查
}
```

### 3.8 工具参数 schema (动态)

```json
{
    "action": "delegate|check_result|list_results|cancel_task",
    "agent": "目标 agent 名称 (从 config 动态列出可用 agents)",
    "prompt": "任务描述",
    "context": "可选上下文",
    "background": false,
    "parallel": ["agent_a", "agent_b"],
    "task_id": "后台任务 ID"
}
```

注意: `agent` 字段的 description 动态列出可用 agent 列表 (排除 caller_alias)

---

## 4. 配置层 (AliasedAgentConfig)

### 4.1 Agent 配置结构 (20+ 字段)

```toml
[agents.researcher]
enabled = true
model_provider = "openai.gpt4"          # 点号引用
risk_profile = "default"
runtime_profile = "default"
skill_bundles = ["coding"]
knowledge_bundles = ["docs"]
mcp_bundles = ["tools"]
cron_jobs = ["daily-summary"]
delegate_same_risk_profile = true        # 自动允许同 profile 互委
delegates = [
    { agent = "coder", mode = "bounded" },
    { agent = "summarizer", mode = "independent" },
]

[agents.researcher.workspace]
path = "/path/to/workspace"              # None = 自动推导
access = { coder = "read", summarizer = "read_write" }
unrestricted_filesystem = false
read_memory_from = ["coder"]             # 可读 coder 的记忆

[agents.researcher.memory]
backend = "sqlite"                       # sqlite|postgres|qdrant|markdown|none
```

### 4.2 跨 Agent 访问控制

- **文件系统**: `workspace.access` map (alias → Read/Write/ReadWrite)
- **记忆**: `workspace.read_memory_from` 列表 (可读哪些 sibling 的记忆)
- **委派**: `delegates` 列表 + `delegate_same_risk_profile` 开关

### 4.3 可达性解析

```rust
// Config::reachable_delegate_targets(caller_alias)
// 返回 caller 可委派的所有目标 alias 列表
fn reachable_delegate_targets(&self, caller: &str) -> Vec<String> {
    // 1. delegate_same_risk_profile=true → 同 risk_profile 的所有 agent
    // 2. + delegates 列表中的显式目标
    // 3. - caller 自己 (不能自委派)
}
```

---

## 5. AgentRunOverrides

```rust
pub struct AgentRunOverrides {
    pub security: Option<Arc<SecurityPolicy>>,  // 覆盖安全策略
    pub memory: Option<Arc<dyn Memory>>,        // 覆盖记忆后端
    pub is_subagent: bool,                       // 标记为 SubAgent 运行
}
```

- spawn_subagent: `security=Some(child_policy), memory=None, is_subagent=true`
- delegate: 不使用 AgentRunOverrides (直接调 provider.chat())
- cron JobType::Agent: `security=Some(policy), memory=None, is_subagent=false`

---

## 6. 安全模型总结

### 6.1 权限继承原则

- SubAgent: **继承父 verbatim, 只能收窄**
  - Policy: `ensure_no_escalation_beyond()` 验证
  - Allowlist: 子集验证
  - Action budget: 共享父的 tracker (防绕过)

- Delegate: **使用目标 agent 自己的 policy**
  - 但 caller 必须有委派权限 (`delegation_policy.permits()`)
  - 目标必须在 caller 的可达列表中
  - 深度不能超过 max_depth

### 6.2 递归保护

| 机制 | SubAgent | Delegate |
|------|----------|----------|
| 深度限制 | depth-1 cap (硬编码) | max_depth (可配置, default 3) |
| 实现方式 | `is_subagent_caller` flag | `self.depth >= max_depth` |
| 自委派 | N/A (同身份) | caller_alias 排除 |

### 6.3 扇出控制

- REENTRANT_AGENT_TOOLS: spawn_subagent + delegate 免去重
- 但 action budget 限制: 每次 spawn/delegate 消耗一个 action 配额
- max_actions_per_hour 限制总扇出量

---

## 7. Shadow 对比与差距

### Shadow 当前状态

- 无 subagent/delegate 实现
- 有 multi-agent 设计文档 (docs/multi-agent-design.md) 但未实现
- 有 SecurityPolicy (15 个危险命令模式 + env 白名单)
- 有 ToolRegistry (动态注册/注销)
- 无 depth 控制
- 无 action budget
- 无 REENTRANT_AGENT_TOOLS 机制

### 精简实现路线 (3级)

#### L1: SpawnSubagentTool (最小可用)
- 继承父 provider + tools + security
- depth-1 cap (硬编码)
- 同步执行: 调用 agent::run() 等待结果
- 不需要 config.agents (同身份)
- ~200行

#### L2: DelegateTool (基本版)
- 需要 config.agents HashMap
- 目标 agent 用自己的 provider/model
- 同步 + background 两种模式
- depth 控制 (可配置)
- ~400行

#### L3: 完整版
- Parallel 模式
- Agentic 模式 (子 agent 有自己的工具循环)
- Bounded vs Independent 模式
- 跨 agent 文件/记忆访问控制
- Action budget 共享
- ~800行

### 关键设计决策

1. SubAgent vs Delegate 分离: ZeroClaw 的设计正确, 不应合并
   - SubAgent = 同身份快速子任务 (不改 provider)
   - Delegate = 跨 agent 专业委派 (不同 provider/model)

2. 安全验证前置: build() 时验证, 不在 execute() 时
3. REENTRANT_AGENT_TOOLS: 必须实现, 否则同 turn 多次 spawn 会被去重
4. CancellationToken: background 模式必须有级联取消
5. DelegateAdmission: Prevalidated 模式避免后台 worker 重复授权
