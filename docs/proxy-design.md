# Shadow Proxy / Broker 设计文档

## 概述

shadow-proxy 是一个 agent 中间件，作为主 agent 的 Tool 运行，同时是：
- 对本地 agent：ACP broker (spawn + stdio JSON-RPC)
- 对远程 agent：A2A broker (HTTP JSON-RPC)
- 对所有 agent：Registry (注册 + 发现)
- 对主 agent：一个 Tool (delegate)

## 架构

```
                ┌─────────────────────┐
                │   Shadow Agent      │
                │   (主 agent, CLI)   │
                └──────────┬──────────┘
                           │ tool 调用
                ┌──────────┴──────────┐
                │   shadow-proxy      │
                │   (broker)          │
                │                     │
                │  Registry (HashMap) │  ← agent 上报 AgentCard
                │  Router (按名/能力) │  ← 路由任务
                │  Translator         │  ← A2A ↔ ACP 翻译
                └──┬──────┬───────┬───┘
                   │      │       │
            ACP    │  A2A │  A2A  │
          (stdio)  │(HTTP)│ (HTTP)│
           ┌───────┴┐ ┌───┴──┐ ┌──┴─────────┐
           │Claude  │ │Shadow│ │Shadow on    │
           │Code    │ │#2    │ │192.168.1.100│
           └────────┘ └──────┘ └────────────┘
```

## 核心 Trait

```rust
/// Agent 传输层 -- 所有 agent 通信的统一抽象
#[async_trait]
pub trait AgentTransport: Send + Sync {
    /// 发送任务, 等待结果
    async fn chat(&self, prompt: &str) -> Result<String>;
    
    /// 发送任务, 流式返回
    async fn chat_stream(&self, prompt: &str) 
        -> BoxStream<'_, Result<ChatChunk>>;
    
    /// agent 能力声明
    fn card(&self) -> &AgentCard;
}

/// 进程内 (同 Shadow 实例, 不同配置)
pub struct LocalAgent { ... }

/// ACP 子进程 (spawn claude/codex, stdio JSON-RPC)
pub struct AcpClient { 
    child: tokio::process::Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

/// A2A 远程 (HTTP JSON-RPC)
pub struct A2aClient {
    url: String,
    auth_token: Option<String>,
    http: reqwest::Client,
}
```

## 数据结构

### AgentCard (注册/发现)

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentCard {
    /// agent 名称 (唯一标识)
    pub name: String,
    /// 描述
    pub description: String,
    /// 传输协议: "local" | "acp" | "a2a"
    pub transport: TransportKind,
    /// 能力标签 (用于路由): ["coding", "review", "research"]
    pub capabilities: Vec<String>,
    /// 技能列表
    pub skills: Vec<AgentSkill>,
    /// A2A endpoint (仅 transport=a2a)
    pub endpoint: Option<String>,
    /// ACP 命令 (仅 transport=acp)
    pub command: Option<String>,
    /// 最后心跳时间
    pub last_heartbeat: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TransportKind {
    Local,
    Acp,
    A2a,
}
```

### Task (任务)

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: String,           // UUID
    pub from: String,         // 发起方 agent 名称
    pub to: String,           // 目标 agent 名称
    pub prompt: String,       // 任务描述
    pub status: TaskStatus,
    pub result: Option<String>,
    pub error: Option<String>,
    pub created_at: String,   // RFC3339
    pub finished_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TaskStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}
```

## Proxy Server API (HTTP)

### 注册/发现

POST /agents/register
  Body: AgentCard
  Response: { "registered": true, "name": "..." }

GET /agents
  Response: AgentCard[]

GET /agents/{name}
  Response: AgentCard | 404

DELETE /agents/{name}
  Response: { "deregistered": true }

### 任务管理

POST /tasks
  Body: { "to": "claude-code", "prompt": "review src/main.rs" }
  Response: Task (status=Running, 异步) 或 Task (status=Completed, 同步)

GET /tasks/{id}
  Response: Task

GET /tasks
  Query: ?status=running&from=main
  Response: Task[]

POST /tasks/{id}/cancel
  Response: Task (status=Cancelled)

### 健康检查

GET /health
  Response: { "status": "ok", "agents": N }

## 配置

```toml
# ~/.shadow/config.toml

# Proxy Server (A2A Server 端)
[proxy]
enabled = true
bind = "127.0.0.1"
port = 9090

# 注册的本地 agent (进程内)
[agents.researcher]
model_provider = "openai.minimax"
model = "MiniMax-M2.7"
capabilities = ["research", "summarization"]

# ACP 子进程 agent
[acp_agents.claude]
command = "claude"
args = ["--acp", "--stdio"]
workdir = "."
capabilities = ["coding", "code_review"]
register_on_start = true

# A2A 远程 agent
[remote_agents.coder]
url = "http://192.168.1.100:8080/a2a/coder"
auth_token = "bearer xxx"
capabilities = ["coding"]
auto_discover = true  # 启动时 GET /.well-known/agent-card.json

# 发现种子节点
[discovery]
seeds = ["http://192.168.1.100:9090", "http://192.168.1.200:9090"]
refresh_interval_secs = 300
```

## 文件结构

```
crates/shadow-proxy/
├── Cargo.toml
├── src/
│   ├── lib.rs              # 模块导出
│   ├── transport.rs         # AgentTransport trait
│   ├── local.rs             # LocalAgent (进程内)
│   ├── acp_client.rs        # AcpClient (stdio JSON-RPC)
│   ├── a2a_client.rs        # A2aClient (HTTP JSON-RPC)
│   ├── card.rs              # AgentCard, AgentSkill, TransportKind
│   ├── task.rs              # Task, TaskStatus
│   ├── registry.rs          # AgentRegistry (HashMap + CRUD)
│   ├── router.rs            # TaskRouter (按名/能力路由)
│   ├── server.rs            # ProxyServer (axum HTTP)
│   ├── proxy_tool.rs        # ProxyTool (impl Tool, 给主 agent 用)
│   └── config.rs            # ProxyConfig (from config.toml)
```

## 实现阶段

### 阶段1: 进程内 Delegate (~200行)
- LocalAgent: 包装现有 Agent, 实现 AgentTransport
- 不需要网络, 直接函数调用
- 配置 [agents.xxx] 多 agent 支持

### 阶段2: ACP Client (~250行)
- AcpClient: spawn 子进程, stdio JSON-RPC
- ACP 协议: JSON-RPC 2.0 over stdio
- 方法: initialize, tools/call, chat
- 支持: claude --acp --stdio, codex --acp --stdio

### 阶段3: A2A Client (~200行)
- A2aClient: HTTP JSON-RPC (message/send)
- GET /.well-known/agent-card.json 发现
- Bearer token 认证
- 基于 reqwest (已有依赖)

### 阶段4: Proxy Server (~300行)
- ProxyServer: axum HTTP server
- POST /agents/register, GET /agents
- POST /tasks, GET /tasks/{id}
- TaskRouter: 按名称或能力路由到 transport
- 任务持久化: JSON 文件 (workspace/proxy_tasks/)

### 阶段5: 自注册 + 发现 (~150行)
- Agent 启动时向 proxy 注册 AgentCard
- 种子节点发现: GET seed/.well-known/agents-card.json
- 心跳: 每 60s POST /agents/register 刷新

### 阶段6: ProxyTool (~150行)
- impl Tool for ProxyTool
- action: delegate | list_agents | check_status | register
- 主 agent 通过 tool 调用 proxy 的全部功能

总计: ~1250行

## ACP 协议简述

ACP (Agent Communication Protocol) 是 stdio 上的 JSON-RPC 2.0:

请求 (proxy → agent, 写入 stdin):
```json
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"1.0"}}
```

响应 (agent → proxy, 从 stdout 读):
```json
{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"1.0","capabilities":{"tools":true}}}
```

发任务:
```json
{"jsonrpc":"2.0","id":2,"method":"chat","params":{"prompt":"review src/main.rs"}}
```

收结果:
```json
{"jsonrpc":"2.0","id":2,"result":{"content":"代码审查结果...","role":"assistant"}}
```

## A2A 协议简述

A2A 是 HTTP 上的 JSON-RPC 2.0:

请求 (proxy → 远程 agent):
```
POST http://host:8080/a2a/coder
Authorization: Bearer xxx
Content-Type: application/json

{"jsonrpc":"2.0","id":1,"method":"message/send","params":{
  "message":{"parts":[{"kind":"text","text":"review src/main.rs"}]}
}}
```

响应:
```json
{"jsonrpc":"2.0","id":1,"result":{
  "id":"task-uuid",
  "status":{"state":"completed"},
  "artifacts":[{"parts":[{"kind":"text","text":"审查结果..."}]}]
}}
```
