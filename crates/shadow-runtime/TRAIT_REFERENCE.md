# ZeroClaw Runtime Trait 体系 -- 完整总结

> 基于 ZeroClaw v0.8.2 (157,684 行, 246 文件) 的深度分析
> Shadow 参照此体系实现精简版

## 一、Trait 全景 (12 个 Trait, 41+ 实现)

### Layer 1: 核心 trait (zeroclaw-api 定义, runtime re-export)

| Trait | 文件 | 方法数 | 实现数 | 语义 |
|-------|------|--------|--------|------|
| ModelProvider | api/model_provider.rs | 8 | 18+ | LLM 推理后端 |
| Channel | api/channel.rs | 30+ | 30+ | 消息平台集成 |
| Tool | api/tool.rs | 4 | 30+ | Agent 可调用能力 |
| Memory | api/memory_traits.rs | 25+ | 7+ | 对话记忆后端 |
| Observer | api/observability_traits.rs | 5 | 8 | 指标和追踪 |
| RuntimeAdapter | api/runtime_traits.rs | 7 | 3 | 执行环境适配 |
| Peripheral | api/peripherals_traits.rs | 6 | 3 | 硬件外设 |
| HookHandler | api/hook.rs | 15 | 2 | 生命周期钩子 |

### Layer 2: runtime 内部 trait (10 个)

| Trait | 文件 | 方法数 | 实现数 | 语义 |
|-------|------|--------|--------|------|
| ToolDispatcher | agent/dispatcher.rs | 5 | 2 | 工具协议分发 |
| MemoryStrategy | agent/memory_strategy.rs | 3 | 1 | 记忆加载/合并/治理 |
| PromptSection | agent/prompt.rs | 3 | 9 | 系统提示段 |
| Sandbox | security/traits.rs | 4 | 6 | OS 级进程隔离 |
| SopRunStore | sop/store/mod.rs | 13 | 2 | SOP 运行状态+并发+审计 |
| RpcTransport | rpc/transport.rs | 3 | 2 | 传输无关帧读写 |
| TaskRegistry | control_plane/task_registry.rs | 7 | 1 | 任务注册/调度/回收 |
| Tunnel | tunnel/mod.rs | 5 | 7 | 隧道抽象 |
| Scout | skillforge/scout.rs | 1 | 1 | 技能发现 |

### 依赖层次

```
agent-core (trait 定义层, 零内部依赖)
  │
  ├── ModelProvider  ──→ shadow-providers
  ├── Channel        ──→ shadow-channels (待创建)
  ├── Tool           ──→ shadow-tools (待创建)
  ├── Memory         ──→ shadow-memory
  ├── Observer       ──→ shadow-runtime/observability
  ├── RuntimeAdapter ──→ shadow-runtime/platform
  ├── Peripheral     ──→ shadow-hardware (待创建)
  └── HookHandler    ──→ shadow-runtime/hooks

shadow-runtime 内部 trait:
  ToolDispatcher   ──→ agent/dispatcher
  MemoryStrategy   ──→ agent/memory_strategy
  PromptSection    ──→ agent/prompt
  Sandbox          ──→ security/
  SopRunStore      ──→ sop/store/
  RpcTransport     ──→ rpc/
  TaskRegistry     ──→ control_plane/
  Tunnel           ──→ tunnel/
  Scout            ──→ skillforge/
```

## 二、各 Trait 详细定义

### 1. ToolDispatcher (工具协议分发)

```rust
pub trait ToolDispatcher: Send + Sync {
    fn parse_response(&self, response: &str) -> (String, Vec<ParsedToolCall>);
    fn format_results(&self, results: &[ToolResult]) -> ConversationMessage;
    fn prompt_instructions(&self, tools: &[ToolSpec]) -> String;
    fn to_provider_messages(&self, history: &[ConversationMessage]) -> Vec<ChatMessage>;
    fn should_send_tool_specs(&self) -> bool;
}
```

实现:
- NativeToolDispatcher: 原生 API (OpenAI/Anthropic), role=tool
- XmlToolDispatcher: <tool_call> 标签, 用于不支持原生工具的模型

### 2. MemoryStrategy (记忆策略)

```rust
pub trait MemoryStrategy: Send + Sync {
    fn load_context(&self, user_message: &str, session_id: &str) -> String;
    fn consolidate_turn(&self, messages: &[ConversationMessage]) -> Result<()>;
    fn run_governance(&self) -> Result<()>;
}
```

实现:
- DefaultMemoryStrategy: recall -> 时间衰减 -> 过滤(score<0.4) -> 格式化

### 3. PromptSection (系统提示段)

```rust
pub trait PromptSection: Send + Sync {
    fn name(&self) -> &str;
    fn render(&self, ctx: &PromptContext) -> String;
    fn priority(&self) -> i32 { 0 }
}
```

实现 (9 个 section):
1. DateTimeSection  2. IdentitySection  3. ToolHonestySection
4. ToolsSection     5. SafetySection   6. SkillsSection
7. WorkspaceSection 8. RuntimeSection  9. ChannelMediaSection

### 4. Sandbox (沙箱)

```rust
pub trait Sandbox: Send + Sync {
    fn wrap_command(&self, cmd: &mut Command) -> io::Result<()>;
    fn is_available(&self) -> bool;
    fn name(&self) -> &str;
    fn description(&self) -> &str;
}
```

实现 (6 个): NoopSandbox / Docker / Firejail / Bubblewrap / Landlock / Seatbelt

### 5. SopRunStore (SOP 持久化)

```rust
#[async_trait]
pub trait SopRunStore: Send + Sync {
    async fn save_run(&self, run: &SopRun) -> Result<()>;
    async fn finish_run(&self, run: &SopRun) -> Result<()>;
    async fn load_active_runs(&self) -> Result<Vec<SopRun>>;
    async fn load_run(&self, run_id: &str) -> Result<Option<SopRun>>;
    async fn try_claim_run(&self, sop: &str, run_id: &str, max_conc: usize, max_global: usize) -> Result<bool>;
    async fn heartbeat_claim(&self, sop: &str, run_id: &str) -> Result<()>;
    async fn release_claim(&self, sop: &str, run_id: &str) -> Result<()>;
    async fn expired_claims(&self, now: DateTime<Utc>) -> Result<Vec<(String, String)>>;
    async fn append_event(&self, event: &SopEventRecord) -> Result<()>;
    async fn list_events(&self, run_id: &str) -> Result<Vec<SopEventRecord>>;
    async fn save_proposal(&self, proposal: &ProceduralProposal) -> Result<()>;
    async fn load_proposal(&self, run_id: &str) -> Result<Option<ProceduralProposal>>;
    async fn list_proposals(&self) -> Result<Vec<ProceduralProposal>>;
}
```

实现: InMemoryRunStore / SqliteRunStore

### 6. RpcTransport (RPC 传输)

```rust
#[async_trait]
pub trait RpcTransport: Send + 'static {
    fn writer(&self) -> mpsc::Sender<String>;
    async fn next_frame(&mut self) -> Option<String>;
    fn peer_label(&self) -> String;
}
```

实现: LocalTransport (Unix socket) / WssTransport (TLS WebSocket)

### 7. TaskRegistry (任务控制平面)

```rust
#[async_trait]
pub trait TaskRegistry: Send + Sync {
    async fn create(&self, rec: TaskRecord) -> Result<()>;
    async fn heartbeat(&self, id: &str, owner_boot_id: &str) -> Result<()>;
    async fn update_status(&self, id: &str, status: TaskStatus, output: Option<String>, error: Option<String>) -> Result<()>;
    async fn get(&self, id: &str) -> Result<Option<TaskRecord>>;
    async fn list_running(&self) -> Result<Vec<TaskRecord>>;
    async fn list_by_agent(&self, agent: &str) -> Result<Vec<TaskRecord>>;
    async fn reconcile_lost(&self, id: &str, now_boot_id: &str) -> Result<bool>;
}
```

实现: SqliteTaskStore

### 8. HookHandler (生命周期钩子)

```rust
#[async_trait]
pub trait HookHandler: Send + Sync {
    fn name(&self) -> &str;
    fn priority(&self) -> i32 { 0 }
    // Void hooks (并行, 9 个)
    async fn on_gateway_start(&self, _host: &str, _port: u16) {}
    async fn on_gateway_stop(&self) {}
    async fn on_session_start(&self, _session_id: &str, _channel: &str) {}
    async fn on_session_end(&self, _session_id: &str, _channel: &str) {}
    async fn on_llm_input(&self, _messages: &[ChatMessage], _model: &str) {}
    async fn on_llm_output(&self, _response: &ChatResponse) {}
    async fn on_after_tool_call(&self, _tool: &str, _result: &ToolResult, _duration: Duration) {}
    async fn on_message_sent(&self, _channel: &str, _recipient: &str, _content: &str) {}
    async fn on_heartbeat_tick(&self) {}
    // Modifying hooks (顺序, 6 个)
    async fn before_model_resolve(&self, ...) -> HookResult<(String, String)> { Continue(...) }
    async fn before_prompt_build(&self, prompt: String) -> HookResult<String> { Continue(prompt) }
    async fn before_llm_call(&self, ...) -> HookResult<()> { Continue(()) }
    async fn before_tool_call(&self, name: String, args: Value) -> HookResult<(String, Value)> { Continue(...) }
    async fn on_message_received(&self, message: ChannelMessage) -> HookResult<ChannelMessage> { Continue(message) }
    async fn on_message_sending(&self, ...) -> HookResult<(String, String, String)> { Continue(...) }
}
```

实现: CommandLoggerHook / WebhookAuditHook

### 9. Tunnel (隧道)

```rust
#[async_trait]
pub trait Tunnel: Send + Sync {
    fn name(&self) -> &str;
    async fn start(&self, local_host: &str, local_port: u16) -> Result<String>;
    async fn stop(&self) -> Result<()>;
    async fn health_check(&self) -> bool;
    fn public_url(&self) -> Option<String>;
}
```

实现 (7 个): None/Cloudflare/Tailscale/Ngrok/OpenVpn/Pinggy/Custom

### 10. Scout (技能发现)

```rust
#[async_trait]
pub trait Scout: Send + Sync {
    async fn discover(&self) -> Result<Vec<ScoutResult>>;
}
```

实现: GitHubScout

## 三、Shadow 实现优先级

### P0 -- 立即实现 (agent 基础)
- ToolDispatcher: Native + XML 双协议
- PromptSection: 可插拔系统提示
- MemoryStrategy: 记忆加载/治理

### P1 -- 近期实现 (安全+扩展)
- Sandbox: NoopSandbox + 基础沙箱
- HookHandler: 生命周期钩子框架
- SopRunStore: InMemory (SQLite 已有 Cron)

### P2 -- 远期实现 (通信+平台)
- RpcTransport: Unix socket
- TaskRegistry: SQLite
- Tunnel: 基础隧道
- Scout: 技能发现

## 四、ZeroClaw crate 对照表

| ZeroClaw crate | 行数 | Shadow crate | 状态 |
|----------------|------|-------------|------|
| zeroclaw-api | 6,293 | agent-core | ✅ 已有 (精简) |
| zeroclaw-config | 65,171 | shadow-config | ✅ 已有 |
| zeroclaw-log | 5,079 | shadow-log | ✅ 已有 |
| zeroclaw-providers | 49,664 | shadow-providers | ✅ 已有 |
| zeroclaw-memory | 17,442 | shadow-memory | ✅ 已有 |
| zeroclaw-runtime | 157,684 | shadow-runtime | ✅ 已有 |
| zeroclaw-infra | 4,679 | shadow-infra | ❌ 待创建 |
| zeroclaw-spawn | 231 | shadow-spawn | ❌ 待创建 |
| zeroclaw-channels | 122,086 | shadow-channels | ❌ 待创建 |
| zeroclaw-tools | 62,235 | shadow-tools | ❌ 待创建 |
| zeroclaw-tool-call-parser | 3,869 | shadow-tool-call-parser | ❌ 待创建 |
| zeroclaw-eval | 1,430 | shadow-eval | ❌ 待创建 |
| zeroclaw-plugins | 3,929 | shadow-plugins | ❌ 待创建 |
| zeroclaw-gateway | 29,789 | shadow-gateway | ❌ 待创建 |
| zeroclaw-hardware | 10,641 | shadow-hardware | ❌ 待创建 |
| zeroclaw-macros | 2,917 | shadow-macros | ❌ 待创建 |
