# ZeroClaw Trait 体系 -- 完整总结

> 本文档总结 ZeroClaw 全部 trait 定义和实现, 作为 shadow 开发的参考蓝图。

## 一、Crate 对照表

| ZeroClaw crate           | 行数    | Shadow crate          | 状态   |
|--------------------------|---------|-----------------------|--------|
| zeroclaw-api             | 6,293   | agent-core            | ✅ 已有 |
| zeroclaw-config          | 65,171  | shadow-config         | ✅ 已有 |
| zeroclaw-log             | 5,079   | shadow-log            | ✅ 已有 |
| zeroclaw-providers       | 49,664  | shadow-providers      | ✅ 已有 |
| zeroclaw-memory          | 17,442  | shadow-memory         | ✅ 已有 |
| zeroclaw-runtime         | 157,684 | shadow-runtime        | ✅ 已有 |
| zeroclaw-infra           | 4,679   | shadow-infra          | ❌ 缺失 |
| zeroclaw-spawn           | 231     | shadow-spawn          | ❌ 缺失 |
| zeroclaw-channels        | 122,086 | shadow-channels       | ❌ 缺失 |
| zeroclaw-tools           | 62,235  | shadow-tools          | ❌ 缺失 |
| zeroclaw-tool-call-parser| 3,869   | shadow-tool-call-parser| ❌ 缺失 |
| zeroclaw-eval            | 1,430   | shadow-eval           | ❌ 缺失 |
| zeroclaw-plugins         | 3,929   | shadow-plugins        | ❌ 缺失 |
| zeroclaw-gateway         | 29,789  | shadow-gateway        | ❌ 缺失 |
| zeroclaw-hardware        | 10,641  | shadow-hardware       | ❌ 缺失 |
| zeroclaw-macros          | 2,917   | shadow-macros         | ❌ 缺失 |

## 二、Trait 总览 (12 个 trait, 41+ 实现)

### Layer 0: 核心 trait (zeroclaw-api, agent-core 已有)

| Trait           | 方法数 | 实现数 | Shadow 状态 |
|-----------------|--------|--------|-------------|
| Attributable     | 2      | blanket | ✅ 已有     |
| ModelProvider    | 6      | 18+     | ✅ 已有 (1) |
| Channel          | 30+    | 30+     | ✅ 已有 (1) |
| Tool             | 4      | 30+     | ✅ 已有 (3) |
| Memory           | 25+    | 9       | ✅ 已有 (2) |
| Observer         | 5      | 8       | ✅ 已有 (1) |
| RuntimeAdapter   | 7      | 3       | ❌ 缺失     |
| Peripheral       | 6      | 3       | ❌ 缺失     |
| HookHandler      | 15     | 2       | ❌ 缺失     |

### Layer 1: 运行时 trait (zeroclaw-runtime, 需要在 shadow-runtime 实现)

| Trait            | 定义位置              | 方法数 | 实现数 | Shadow 状态 | 优先级 |
|------------------|----------------------|--------|--------|-------------|--------|
| ToolDispatcher   | agent/dispatcher.rs  | 5      | 2      | ❌ 缺失     | P0     |
| MemoryStrategy   | agent/memory_strategy| 3      | 1      | ❌ 缺失     | P0     |
| PromptSection    | agent/prompt.rs      | 3      | 9      | ❌ 缺失     | P0     |
| Sandbox          | security/traits.rs   | 4      | 6      | ❌ 缺失     | P1     |
| SopRunStore      | sop/store/mod.rs     | 13     | 2      | ❌ 缺失     | P2     |
| RpcTransport     | rpc/transport.rs     | 3      | 2      | ❌ 缺失     | P2     |
| TaskRegistry     | control_plane/       | 7      | 1      | ❌ 缺失     | P2     |
| Tunnel           | tunnel/mod.rs        | 5      | 7      | ❌ 缺失     | P2     |
| Scout            | skillforge/scout.rs  | 1      | 1      | ❌ 缺失     | P2     |

## 三、各 Trait 详细定义和实现

### 1. ToolDispatcher (P0 -- 工具协议分发)

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
- NativeToolDispatcher: 原生 API (OpenAI/Anthropic), role=tool, should_send_tool_specs=true
- XmlToolDispatcher: <tool_call> 标签, <tool_result> 文本, should_send_tool_specs=false

选择逻辑: config 指定 > provider.supports_native_tools() > XML

### 2. MemoryStrategy (P0 -- 记忆加载/合并/治理)

```rust
pub trait MemoryStrategy: Send + Sync {
    fn load_context(&self, user_message: &str, session_id: &str) -> String;
    fn consolidate_turn(&self, messages: &[ConversationMessage]) -> Result<()>;
    fn run_governance(&self) -> Result<()>;
}
```

实现:
- DefaultMemoryStrategy:
  - load_context: recall(limit=5) -> 时间衰减 -> 过滤(score<0.4) -> [memory_context]格式化
  - consolidate_turn: LLM 提取记忆 -> store
  - run_governance: hygiene::run_if_due (定期清理)

### 3. PromptSection (P0 -- 系统提示可插拔段)

```rust
pub trait PromptSection: Send + Sync {
    fn name(&self) -> &str;
    fn render(&self, ctx: &PromptContext) -> String;
    fn priority(&self) -> i32 { 0 }
}
```

实现 (9 个 section, 按 priority 排序拼接):
1. DateTimeSection -- 日期时间
2. IdentitySection -- 身份 (AIEOS/personality/SOUL.md)
3. ToolHonestySection -- 工具诚实性约束
4. ToolsSection -- 工具列表 (native跳过, xml列出)
5. SafetySection -- 安全约束 + autonomy_level 指令
6. SkillsSection -- 技能注入
7. WorkspaceSection -- 工作目录
8. RuntimeSection -- 运行时环境 (OS/host)
9. ChannelMediaSection -- channel 媒体支持

SystemPromptBuilder 持有 Vec<Box<dyn PromptSection>>, 按 priority 排序后拼接。

### 4. Sandbox (P1 -- OS 级进程隔离)

```rust
pub trait Sandbox: Send + Sync {
    fn wrap_command(&self, cmd: &mut Command) -> io::Result<()>;
    fn is_available(&self) -> bool;
    fn name(&self) -> &str;
    fn description(&self) -> &str;
}
```

实现 (6 个):
- NoopSandbox: 无操作回退
- DockerSandbox: --network none --memory 512m
- FirejailSandbox: SUID, --caps.drop=all
- BubblewrapSandbox: 用户命名空间, --unshare-all
- LandlockSandbox: Linux 5.13+ LSM
- SeatbeltSandbox: macOS sandbox-exec

detect.rs 选择梯: Landlock > Firejail > Bubblewrap > Seatbelt > None

### 5. SopRunStore (P2 -- SOP 状态+并发+审计+提案)

```rust
#[async_trait]
pub trait SopRunStore: Send + Sync {
    async fn save_run(&self, run: &SopRun) -> Result<()>;
    async fn finish_run(&self, run: &SopRun) -> Result<()>;
    async fn load_active_runs(&self) -> Result<Vec<SopRun>>;
    async fn try_claim_run(&self, sop: &str, run_id: &str, max_conc: usize, max_global: usize) -> Result<bool>;
    async fn heartbeat_claim(&self, sop: &str, run_id: &str) -> Result<()>;
    async fn release_claim(&self, sop: &str, run_id: &str) -> Result<()>;
    async fn append_event(&self, event: &SopEventRecord) -> Result<()>;
    async fn list_events(&self, run_id: &str) -> Result<Vec<SopEventRecord>>;
    // ... 程序记忆提案
}
```

四合一: 运行状态 + CAS声明(并发) + 事件日志(审计) + 程序记忆(提案)

### 6. RpcTransport (P2 -- 传输无关帧读写)

```rust
#[async_trait]
pub trait RpcTransport: Send + 'static {
    fn writer(&self) -> mpsc::Sender<String>;
    async fn next_frame(&mut self) -> Option<String>;
    fn peer_label(&self) -> String;
}
```

实现:
- LocalTransport: Unix socket / Named pipe
- WssTransport: TLS WebSocket, 20s ping

### 7. TaskRegistry (P2 -- 任务注册/调度/回收)

```rust
#[async_trait]
pub trait TaskRegistry: Send + Sync {
    async fn create(&self, rec: TaskRecord) -> Result<()>;
    async fn heartbeat(&self, id: &str, owner_boot_id: &str) -> Result<()>;
    async fn update_status(&self, id: &str, status: TaskStatus, ...) -> Result<()>;
    async fn get(&self, id: &str) -> Result<Option<TaskRecord>>;
    async fn list_running(&self) -> Result<Vec<TaskRecord>>;
    async fn reconcile_lost(&self, id: &str, now_boot_id: &str) -> Result<bool>;
}
```

### 8. Tunnel (P2 -- 隧道抽象)

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

### 9. HookHandler (P1 -- 生命周期钩子, 定义在 zeroclaw-api)

```rust
#[async_trait]
pub trait HookHandler: Send + Sync {
    fn name(&self) -> &str;
    fn priority(&self) -> i32 { 0 }

    // Void hooks (并行, 9个)
    async fn on_gateway_start(&self, host: &str, port: u16) {}
    async fn on_gateway_stop(&self) {}
    async fn on_session_start(&self, session_id: &str, channel: &str) {}
    async fn on_session_end(&self, session_id: &str, channel: &str) {}
    async fn on_llm_input(&self, messages: &[ChatMessage], model: &str) {}
    async fn on_llm_output(&self, response: &ChatResponse) {}
    async fn on_after_tool_call(&self, tool: &str, result: &ToolResult, duration: Duration) {}
    async fn on_message_sent(&self, channel: &str, recipient: &str, content: &str) {}
    async fn on_heartbeat_tick(&self) {}

    // Modifying hooks (顺序, 6个, 返回 HookResult<T>)
    async fn before_model_resolve(&self, provider: String, model: String) -> HookResult<(String, String)> { Continue(...) }
    async fn before_prompt_build(&self, prompt: String) -> HookResult<String> { Continue(prompt) }
    async fn before_llm_call(&self, messages: &mut Vec<ChatMessage>, model: &mut String) -> HookResult<()> { Continue(()) }
    async fn before_tool_call(&self, name: String, args: Value) -> HookResult<(String, Value)> { Continue(...) }
    async fn on_message_received(&self, message: ChannelMessage) -> HookResult<ChannelMessage> { Continue(message) }
    async fn on_message_sending(&self, channel: String, recipient: String, content: String) -> HookResult<(String, String, String)> { Continue(...) }
}

pub enum HookResult<T> { Continue(T), Cancel(String) }
```

HookRunner:
- Void: join_all 并行
- Modifying: priority 降序串行管道, Cancel 短路
- catch_unwind panic 隔离

### 10. RuntimeAdapter (P1 -- 执行环境适配, 定义在 zeroclaw-api)

```rust
pub trait RuntimeAdapter: Send + Sync {
    fn name(&self) -> &str;
    fn has_shell_access(&self) -> bool;
    fn has_filesystem_access(&self) -> bool;
    fn storage_path(&self) -> PathBuf;
    fn supports_long_running(&self) -> bool;
    fn memory_budget(&self) -> u64 { 0 }
    fn build_shell_command(&self, command: &str, workspace_dir: &Path) -> Result<Command>;
}
```

实现 (3 个):
- NativeRuntime: shell=true, fs=true, long_running=true
- DockerRuntime: shell=true (docker exec)
- WasmRuntime: shell=false, wasmi 引擎, 燃料计量

### 11. Observer (已有, 需扩展实现)

```rust
pub trait Observer: Send + Sync + 'static {
    fn record_event(&self, event: &ObserverEvent);
    fn record_metric(&self, metric: &ObserverMetric);
    fn flush(&self) {}
    fn name(&self) -> &str;
    fn as_any(&self) -> dyn Any;
}
```

ZeroClaw 实现 (8 个): Noop/Log/Verbose/Prometheus/Otel/Dora/Multi/Tee
Shadow 实现 (1 个): NoopObserver

### 12. Scout (P2 -- 技能发现)

```rust
#[async_trait]
pub trait Scout: Send + Sync {
    async fn discover(&self) -> Result<Vec<ScoutResult>>;
}
```

## 四、实现优先级

### P0 -- 核心可用性 (本次实现)
1. ToolDispatcher -- Native/XML 双协议, Agent 依赖
2. MemoryStrategy -- 记忆加载/合并/治理, Agent 依赖
3. PromptSection -- 可插拔系统提示, Agent 依赖

### P1 -- 安全/扩展
4. Sandbox -- 进程隔离 (先 NoopSandbox)
5. HookHandler -- 生命周期钩子
6. RuntimeAdapter -- 执行环境抽象 (先 NativeRuntime)
7. Observer 扩展 -- LogObserver + MultiObserver

### P2 -- 高级功能
8. SopRunStore -- SOP 持久化
9. RpcTransport -- RPC 通信
10. TaskRegistry -- 任务控制平面
11. Tunnel -- 隧道抽象
12. Scout -- 技能发现
