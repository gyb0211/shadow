# Shadow 多用户/多 Agent 设计文档

> 参考 ZeroClaw multi_agent.rs (472行) + schema.rs AliasedAgentConfig + tool_execution.rs

## 一、ZeroClaw 的多 Agent 架构

### 1.1 配置层

```toml
# ZeroClaw config.toml
[agents.researcher]
enabled = true
model_provider = "openai.default"
channels = ["telegram.main"]
skill_bundles = ["research"]
risk_profile = "default"
runtime_profile = "default"

[agents.researcher.workspace]
path = "/data/agents/researcher/workspace"
# 跨 agent 文件访问白名单 (默认 jailed)
[agents.researcher.workspace.access]
coder = "read"       # researcher 可以读 coder 的文件

[agents.researcher.memory]
backend = "sqlite"   # 创建后不可更改

[agents.coder]
model_provider = "anthropic.claude"
channels = ["discord.dev"]
skill_bundles = ["coding"]

[agents.coder.workspace.access]
researcher = "read_write"  # coder 可以读写 researcher 的文件

[agents.coder.workspace]
unrestricted_filesystem = false  # 默认 jailed
```

```rust
// Config 结构
struct Config {
    agents: HashMap<String, AliasedAgentConfig>,  // 多 agent
    channels: ChannelsConfig,                      // 多渠道
    // ...
}

struct AliasedAgentConfig {
    enabled: bool,
    channels: Vec<ChannelRef>,
    model_provider: ModelProviderRef,
    risk_profile: RiskProfileRef,
    runtime_profile: RuntimeProfileRef,
    skill_bundles: Vec<String>,
    workspace: AgentWorkspaceConfig,
    memory: AgentMemoryConfig,
    a2a: AgentA2aConfig,  // Agent-to-Agent 发现
}
```

### 1.2 隔离层

#### 工作空间隔离

```
<install>/
├── agents/
│   ├── researcher/
│   │   └── workspace/     # researcher 的工作目录 (jailed)
│   │       ├── memory.db
│   │       ├── sessions/
│   │       └── skills/
│   ├── coder/
│   │   └── workspace/     # coder 的工作目录 (jailed)
│   │       ├── memory.db
│   │       └── ...
│   └── reviewer/
│       └── workspace/
└── shared/
    └── skills/            # 共享技能目录
```

```rust
struct AgentWorkspaceConfig {
    path: Option<PathBuf>,                          // 自定义路径 (None = 默认)
    access: BTreeMap<AgentAlias, AccessMode>,       // 跨 agent 文件访问白名单
    unrestricted_filesystem: bool,                  // 逃生舱 (审计标记)
    read_memory_from: Vec<AgentAlias>,              // 跨 agent 记忆读取白名单
}

enum AccessMode {
    Read,       // 只读
    Write,      // 只写
    ReadWrite,  // 读写
}
```

- 默认 jailed: agent 只能访问自己的 workspace
- 跨 agent 访问需要显式声明 (双向各自声明)
- `unrestricted_filesystem = true` 解除限制 (审计标记)

#### 记忆隔离

```rust
struct AgentMemoryConfig {
    backend: MemoryBackendKind,  // sqlite / postgres / qdrant / markdown / none
    // backend 创建后不可更改 (Config::validate() 强制)
}

enum MemoryBackendKind {
    None,
    Sqlite,      // 默认
    Postgres,    // + pgvector
    Qdrant,      // 向量数据库
    Markdown,    // 文件
    Lucid,       // 混合
}
```

- 每个 agent 有独立的 memory.db (或独立 postgres schema)
- `read_memory_from` 白名单: agent A 可以读 agent B 的记忆
- 跨 backend 的记忆共享被拒绝 (验证时检查)

### 1.3 通信层

#### DelegateTool (任务委派)

```rust
// agent A 调用工具委托任务给 agent B
// LLM 调用: delegate(target="coder", task="写一个排序函数")
// → Agent A 的 DelegateTool 查找 agents.coder 配置
// → 创建临时 Agent B 实例
// → Agent B 执行任务
// → 结果返回给 Agent A
```

#### SpawnSubagentTool (子 agent 生成)

```rust
// 生成一个一次性子 agent (不持久化配置)
// 子 agent 继承父 agent 的 provider/memory/workspace
// 适合: 并行任务拆分
```

#### PeerGroup (多 agent 群聊)

```toml
[peer_groups.dev_team]
channel = "telegram.dev"    # 渠道
agents = ["researcher", "coder", "reviewer"]
external_peers = ["@alice", "@bob"]
output_modality = "mirror"  # mirror / voice / text
```

- 多个 agent + 人类用户在同一个渠道
- 消息路由: @researcher 的问题路由给 researcher agent
- agent 之间可以互相看到消息

#### A2A 协议 (Agent-to-Agent)

```toml
[a2a.server]
enabled = true
# agent 发布为可发现的 A2A 服务

[agents.researcher.a2a]
published = true
exposed_skills = ["search", "summarize"]
```

- Agent 发布为可发现的服务 (类似 microservice)
- 其他系统可以通过 HTTP 调用 agent 的技能
- 双层门控: server.enabled + agent.a2a.published

### 1.4 渠道层

```toml
[channels.telegram.main]
token = "..."
# Telegram 渠道

[channels.discord.dev]
token = "..."
# Discord 渠道
```

```rust
struct Config {
    agents: HashMap<String, AliasedAgentConfig>,
    channels: ChannelsConfig,  // Telegram / Discord / Slack / WebSocket / SMS
}
```

- 每个 agent 绑定到特定渠道: `agent.channels = ["telegram.main"]`
- 一个渠道可以服务多个 agent (通过 peer_groups)
- 渠道负责消息路由 (用户消息 → 正确的 agent)

## 二、Shadow 当前状态

### 2.1 配置结构

```rust
// Shadow config.toml
struct Config {
    agent: AgentSection,        // 单 agent (不是 HashMap)
    providers: ProvidersConfig,
    memory: MemorySection,
}

struct AgentSection {
    alias: String,              // "shadow"
    model_provider: String,     // "openai.minimax"
    model: String,
    autonomy: String,
    max_iterations: usize,
    max_history: usize,
    system_prompt: Option<String>,
}
```

### 2.2 与 ZeroClaw 的差距

| 维度 | ZeroClaw | Shadow | 差距 |
|------|----------|--------|------|
| Agent 数量 | HashMap (多 agent) | 单 AgentSection | P0 |
| 工作空间隔离 | 每 agent 独立目录 + jailed | 共享进程目录 | P1 |
| 跨 agent 文件访问 | access 白名单 | 不需要 (单 agent) | P2 |
| 记忆隔离 | 每 agent 独立 memory.db | 单 memory.db | P1 |
| 跨 agent 记忆 | read_memory_from 白名单 | 不需要 | P2 |
| 任务委派 | DelegateTool | 无 | P2 |
| 子 agent | SpawnSubagentTool | 无 | P2 |
| Peer 组 | PeerGroupConfig | 无 | P3 |
| A2A 协议 | A2A server + published | 无 | P3 |
| 渠道集成 | Telegram/Discord/Slack | 无 | P2 |
| Memory backend 选择 | 每 agent 选 backend | 全局一个 | P1 |
| risk_profile | 每 agent 选风险等级 | 全局一个 autonomy | P2 |

## 三、Shadow 多用户演进路线

### 层级 1: 多 Agent 配置 (最小改动, P0)

改动范围: shadow-config + TUI/CLI 启动逻辑

```toml
# ~/.shadow/config.toml

[agents.default]
alias = "shadow"
model_provider = "openai.minimax"
model = "MiniMax-M2.7"
autonomy = "full"
max_iterations = 10

[agents.coder]
alias = "coder"
model_provider = "openai.glm"
model = "GLM-4.7"
autonomy = "supervised"
system_prompt = "你是一个编程助手"

[agents.writer]
alias = "writer"
model_provider = "openai.minimax"
model = "MiniMax-M2.7"
system_prompt = "你是一个写作助手"
```

```rust
// shadow-config/src/schema.rs
struct Config {
    // 旧: agent: AgentSection
    // 新: agents: HashMap<String, AgentSection>
    agents: HashMap<String, AgentSection>,
    // 向后兼容: 如果 config.toml 只有 [agent] 段, 迁移为 agents.default
}

// 启动时选择 agent
// shadow chat -m "hello" --agent coder
// shadow chat  # 默认用 agents.default
```

改动点:
- Config.agent → Config.agents: HashMap<String, AgentSection>
- 迁移逻辑: 旧 [agent] 段自动迁移为 [agents.default]
- CLI: --agent <name> 参数选择 agent
- TUI: 启动时选择 agent (或默认 default)
- build_agent() 接收 AgentSection 参数

不需要改:
- Agent 结构 (已经是通用设计)
- Provider/Memory/Tool/Observer (都是 trait, 不绑定具体 agent)
- SkillsService (从 agent 的 workspace 加载)

### 层级 2: 工作空间隔离 (P1)

改动范围: shadow-config + shadow-runtime (ShellTool/FileRead/FileWrite)

```rust
struct AgentSection {
    alias: String,
    model_provider: String,
    // 新增:
    workspace: Option<PathBuf>,  // None = ~/.shadow/agents/<alias>/
}

// 工作目录结构:
// ~/.shadow/agents/<alias>/
//   ├── memory.db
//   ├── sessions/
//   ├── skills/
//   └── logs/
```

改动点:
- AgentSection 加 workspace 字段
- ShellTool/FileReadTool/FileWriteTool 限制在 workspace 内 (PathGuardedTool)
- Memory/SessionStore/SkillsService 从 agent 的 workspace 加载
- config_dir() 改为 agent_workspace_dir(alias)

不需要:
- 跨 agent 文件访问 (access 白名单) -- 单用户场景不需要
- unrestricted_filesystem -- Shadow 信任用户

### 层级 3: 跨 Agent 通信 (P2)

改动范围: shadow-runtime (DelegateTool)

```rust
// DelegateTool: agent A 委托任务给 agent B
// 1. Agent A 的 LLM 调用 delegate(target="coder", task="写排序函数")
// 2. DelegateTool 查找 config.agents["coder"]
// 3. 创建临时 Agent B (用 coder 的 provider/memory/tools)
// 4. Agent B 执行 task
// 5. 结果返回给 Agent A
```

改动点:
- DelegateTool 实现 (持有 Config 引用, 按名字创建临时 Agent)
- Agent 构建支持 "临时模式" (不持久化 session, 不加载全部技能)

不需要:
- SpawnSubagentTool (太复杂, DelegateTool 够用)
- PeerGroup (需要渠道集成)
- A2A 协议 (需要 HTTP 服务)

### 层级 4: 渠道集成 (P2/P3)

改动范围: 新建 shadow-channels crate

```
shadow-channels/
├── telegram.rs    # Telegram Bot API
├── discord.rs     # Discord Bot API
└── webhook.rs     # HTTP Webhook
```

- 消息路由: 用户消息 → 正确的 agent
- 每个 agent 绑定渠道: `agent.channels = ["telegram.main"]`
- 需要长连接 (tokio + telegram-bot-api)

### 不做的 (刻意精简)

| 功能 | 原因 |
|------|------|
| AccessMode 白名单 | 单用户, 不需要跨 agent 文件控制 |
| read_memory_from | 单用户, 不需要跨 agent 记忆共享 |
| unrestricted_filesystem | Shadow 信任用户, 不需要审计 |
| SpawnSubagentTool | DelegateTool 够用 |
| PeerGroup | 需要渠道, 太复杂 |
| A2A 协议 | 需要 HTTP 服务, 太复杂 |
| risk_profile | 用 autonomy 字段够了 |
| MemoryBackendKind 选择 | 全局 sqlite 够用, 不需要每 agent 选 |
| Skill bundle 系统 | 直接从 workspace/skills/ 加载 |

## 四、建议的实施顺序

```
现在 (main 分支):
  单 agent, 共享目录, 单 memory

Step 1 (层级 1): 多 Agent 配置
  Config.agents: HashMap
  --agent 参数选择
  向后兼容迁移

Step 2 (层级 2): 工作空间隔离
  每 agent 独立目录
  PathGuardedTool 限制路径
  独立 memory/sessions/skills

Step 3 (层级 3): 跨 Agent 通信
  DelegateTool
  临时 Agent 创建

Step 4 (层级 4): 渠道集成 (如有需求)
  Telegram/Discord
  消息路由
```

每一步都是增量改进, 不破坏现有功能。
层级 1 的改动最小 (只改 config + 启动逻辑), 可以先做。
