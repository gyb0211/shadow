# Shadow Tool 设计文档

> 参考 ZeroClaw tool.rs (150行) + tool_execution.rs (910行) + tools/mod.rs (2527行)

## 一、ZeroClaw 的 Tool 架构

### 1.1 Tool Trait (zeroclaw-api/src/tool.rs)

```rust
trait Tool: Send + Sync + Attributable {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters_schema(&self) -> serde_json::Value;
    async fn execute(&self, args: serde_json::Value) -> Result<ToolResult>;
    fn spec(&self) -> ToolSpec { ... }  // 默认实现
}
```

ZeroClaw 的 Tool trait 很简洁 -- 和 Shadow 当前的几乎一样。
关键差异在 **执行层** 和 **注册层**。

### 1.2 工具执行 (tool_execution.rs, 910行)

ZeroClaw 的工具执行不是简单的 `tool.execute(args)`，而是一个完整管道：

```
LLM 返回 tool_calls
    ↓
should_execute_tools_in_parallel() -- 决策: 并行还是串行
    ↓
execute_one_tool() -- 单个工具执行管道:
    1. excluded_tools 检查 -- 排除列表
    2. find_tool() -- 在 registry 中查找
    3. activated_tools 查找 -- MCP 动态激活的工具
    4. 未找到 → 返回 "Unknown tool" 错误
    5. observer.record_event(ToolCallStart) -- 通知观察者开始
    6. record!(Invoke, tool=xxx, input=xxx) -- 日志记录
    7. TurnEvent::ToolCall -- 推送给前端
    8. tool.execute(args) -- 实际执行
       ├─ cancellation_token 支持 (tokio::select!)
       └─ instrument(tool_span) -- 归因 span
    9. record!(Complete/Fail, output=xxx) -- 结果日志
    10. observer.record_event(ToolCall) -- 通知观察者完成
    11. TurnEvent::ToolResult -- 推送结果给前端
    12. receipt_generator -- HMAC 回执 (可选)
    ↓
execute_tools_parallel() -- join_all 并行
execute_tools_sequential() -- 逐个串行
```

关键设计：
- **并行 vs 串行决策**: >1 个工具调用时默认并行, 但 tool_search 或需审批的工具强制串行
- **取消支持**: CancellationToken, 串行模式下每个工具前检查
- **凭证脱敏**: scrub_credentials() 在输出给 observer/log 前脱敏
- **工具 I/O 自动记录**: execute() 实现内零日志, 由执行管道统一记录 input/output
- **HMAC 回执**: 证明工具确实执行了 (防篡改审计)

### 1.3 工具注册 (tools/mod.rs, 2527行)

```
default_tools(security) -- 基础工具集:
  ShellTool → RateLimitedTool → PathGuardedTool 包装
  FileReadTool → RateLimitedTool → PathGuardedTool
  FileWriteTool → RateLimitedTool → PathGuardedTool
  FileEditTool → RateLimitedTool → PathGuardedTool
  GlobSearchTool → RateLimitedTool → PathGuardedTool
  ContentSearchTool → RateLimitedTool → PathGuardedTool

all_tools_with_runtime() -- 完整工具集 (50+):
  + MemoryRecallTool / MemoryStoreTool / MemoryForgetTool
  + HttpRequestTool / BrowserTool
  + CronAddTool / CronListTool / CronRemoveTool
  + DelegateTool / SpawnSubagentTool
  + SkillTool / SkillHttpTool / SkillManageTool
  + MCP 工具 (动态激活)
  + 硬件工具 / Google 工具 / Jira / ...
```

装饰器模式 (从外到内):
```
RateLimitedTool (速率限制)
  └─ PathGuardedTool (路径安全)
       └─ ShellTool (实际工具)
```

### 1.4 Shadow 当前状态

```rust
// shadow-core/src/tool.rs (111行)
trait Tool: Attributable {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters_schema(&self) -> Value;
    async fn execute(&self, args: Value) -> Result<ToolResult>;
    fn timeout(&self) -> Option<Duration>;      // ← ZeroClaw 没有
    fn requires_approval(&self) -> bool;         // ← ZeroClaw 在 ApprovalManager 里
    fn spec(&self) -> ToolSpec;
}

// shadow-runtime/src/tools/mod.rs (25行)
fn default_tools() -> Vec<Box<dyn Tool>> {
    vec![Box::new(ShellTool), Box::new(FileReadTool), Box::new(FileWriteTool)]
}
```

Shadow 的 Agent.execute_tool_call() 已经有:
- 只读模式拒绝
- Supervised 审批检查
- 超时控制 (tokio::time::timeout)
- 工具事件回调
- Observer 事件记录

### 1.5 差距分析

| # | 问题 | 严重度 | ZeroClaw 做法 | Shadow 现状 |
|---|------|--------|--------------|-------------|
| 1 | 无并行执行 | P1 | join_all 并行, 审批/tool_search 串行 | 串行 for 循环 |
| 2 | 无取消支持 | P1 | CancellationToken | 无 |
| 3 | 无凭证脱敏 | P2 | scrub_credentials() | 无 |
| 4 | 无装饰器模式 | P1 | RateLimited + PathGuarded 包装 | 裸工具 |
| 5 | 无工具排除 | P2 | excluded_tools 列表 | 无 |
| 6 | 无 MCP 动态激活 | P2 | ActivatedToolSet | 无 |
| 7 | 无 HMAC 回执 | P3 | ReceiptGenerator | 无 |
| 8 | 工具数量少 | P1 | 50+ 工具 | 3 个 (Shell/FileRead/FileWrite) |
| 9 | 无 Memory 工具 | P1 | MemoryRecall/Store/Forget | 无 |
| 10 | 无 HTTP 工具 | P2 | HttpRequestTool | 无 |
| 11 | 无 GlobSearch | P2 | 文件名搜索 | 无 |
| 12 | 无 ContentSearch | P2 | 文件内容搜索 | 无 |

### 1.6 Shadow 已有的优势 (不需要改)

- Tool trait 设计已对齐 ZeroClaw (name/description/schema/execute/spec)
- timeout() 比 ZeroClaw 更好 (ZeroClaw 没有, 靠装饰器实现)
- requires_approval() 在 Tool trait 里, ZeroClaw 在 ApprovalManager 里
- Attributable 继承已对齐
- tool_attribution! 宏已对齐

## 二、Shadow Tool 改进设计

### 2.1 核心原则

不照搬 ZeroClaw 的 910 行 tool_execution.rs (太复杂),
而是提取关键设计模式, 在 Shadow 的 Agent.execute_tool_call() 基础上增量改进。

### 2.2 改进项 (按优先级)

#### P0: 补齐基础工具 (Memory + Search)
- MemoryRecallTool: 让 LLM 主动检索记忆
- MemoryStoreTool: 让 LLM 主动存储记忆
- GlobSearchTool: 文件名搜索 (find/glob)
- ContentSearchTool: 文件内容搜索 (grep/rg)

#### P0: 工具注册表
- ToolRegistry: 动态注册/注销工具
- default_tools() 保持不变
- 支持运行时添加工具 (Skills 系统)

#### P1: 并行执行
- Agent 工具循环改为: 多个 tool_calls 并行执行
- 审批需要的工具仍串行

#### P1: 装饰器模式
- ToolWrapper trait: 包装 Tool, 添加横切逻辑
- RateLimitedTool: 速率限制 (每秒最多 N 次)
- PathGuardedTool: 路径安全检查 (防止访问工作目录外)

#### P1: 凭证脱敏
- scrub_credentials(): 输出前脱敏 API key 等

#### P2: 取消支持
- Agent 加 CancellationToken
- 工具循环支持取消

#### P2: HTTP 工具
- HttpRequestTool: 让 LLM 发 HTTP 请求

### 2.3 设计细节

#### ToolRegistry

```rust
pub struct ToolRegistry {
    tools: Vec<Box<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self;
    pub fn register(&mut self, tool: Box<dyn Tool>);
    pub fn find(&self, name: &str) -> Option<&dyn Tool>;
    pub fn specs(&self) -> Vec<ToolSpec>;
    pub fn execute(&self, name: &str, args: Value) -> Result<ToolResult>;
}
```

Agent 从 Vec<Box<dyn Tool>> 改为持有 ToolRegistry。

#### MemoryRecallTool

```rust
pub struct MemoryRecallTool {
    memory: Arc<dyn Memory>,
}

// name: "memory_recall"
// description: "检索相关记忆"
// parameters: { query: string, limit?: number }
// execute: memory.recall(query, limit, None) → 格式化结果
```

#### 并行执行

```rust
// Agent 工具循环中:
let futures: Vec<_> = response.tool_calls.iter()
    .map(|tc| self.execute_tool_call(tc))
    .collect();
let results = futures::future::join_all(futures).await;
```

但审批需要的工具要先检查再决定是否并行。

#### 装饰器

```rust
pub trait ToolWrapper: Tool {
    fn inner(&self) -> &dyn Tool;
}

pub struct RateLimitedTool {
    inner: Box<dyn Tool>,
    max_per_sec: usize,
    last_calls: Mutex<Instant>,
}

// execute() 先检查速率, 再调 inner.execute()
```
