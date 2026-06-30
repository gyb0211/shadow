# Shadow Coding Plan — 下一步开发计划

> 基于当前 Tool trait 和工具实现的不足分析, 制定后续开发路线图

## 一、当前状态评估

### 已完成

- ✅ Agent trait 驱动架构 (Tool / ModelProvider / Memory / Observer / Channel)
- ✅ Shell / FileRead / FileWrite 三个基本工具
- ✅ Agent::chat() 工具调用循环 (解析 tool_calls → 执行 → 回传结果 → 循环)
- ✅ OpenAI 兼容 Provider (支持 function calling)
- ✅ Markdown 记忆后端 + None 后端
- ✅ shadow-log (record! 宏 + JSONL 持久化 + 广播)
- ✅ 双模式构建 (完整版 / kernel-only)
- ✅ 多轮对话历史 (内存)
- ✅ LogObserver (Observer 事件转发到 JSONL)

### 测试覆盖

- agent-core: 1 test (Attributable blanket impl)
- shadow-config: 15 tests (Provider 解析 + TOML 序列化)
- shadow-runtime: 6 tests (Shell + FileRead + FileWrite)
- 总计: 22 tests, 全部通过

---

## 二、不足分析

### 2.1 Tool Trait 设计不足

| 问题 | 严重度 | 说明 |
|------|--------|------|
| 无参数校验 | 高 | `execute()` 接收 `Value`, 无 JSON Schema 校验, 工具自行解析容易出错 |
| 无超时机制 | 高 | `execute()` 无超时参数, 长时间运行的工具会阻塞 Agent |
| 无流式输出 | 中 | 返回单一 `ToolResult`, 无法流式返回中间进度 |
| 无权限声明 | 中 | 工具无法声明所需权限 (如文件系统访问、网络访问) |
| 无生命周期 | 低 | 无 init/cleanup 钩子, 工具无法做初始化或资源清理 |
| 无工具组合 | 低 | 无法声明工具间依赖或管道式组合 |
| 无版本管理 | 低 | ToolSpec 无 version 字段, 无法做兼容性检查 |
| 参数类型弱 | 低 | `parameters_schema() -> Value` 非强类型, 可考虑泛型参数 |

### 2.2 工具实现不足

#### ShellTool

| 问题 | 严重度 | 说明 |
|------|--------|------|
| 无命令白名单/黑名单 | 高 | 任意命令可执行, 安全风险大 |
| 无超时 | 高 | `sleep 9999` 会永久阻塞 |
| 无工作目录控制 | 中 | 默认在进程目录, 无法指定工作目录 |
| 无环境变量控制 | 中 | 继承所有环境变量, 可能泄露敏感信息 |
| 无输出流式返回 | 中 | 一次性收集所有 stdout/stderr, 长输出占内存 |
| 无退出码区分 | 低 | 失败时只返回 error, 不区分非零退出码 vs 执行异常 |

#### FileReadTool

| 问题 | 严重度 | 说明 |
|------|--------|------|
| 无二进制文件检测 | 中 | 读取二进制文件会产生乱码 |
| 无行范围支持 | 中 | 无法指定读取第 N-M 行 |
| 无编码检测 | 低 | 假设 UTF-8, 非 UTF-8 文件会报错 |
| 截断阈值硬编码 | 低 | 100KB 限制不可配置 |

#### FileWriteTool

| 问题 | 严重度 | 说明 |
|------|--------|------|
| 无追加模式 | 中 | 只能覆盖, 无法 append |
| 无原子写入 | 中 | 直接写入, 异常中断可能产生半截文件 |
| 无备份 | 低 | 覆盖前不备份原文件 |
| 无权限检查 | 低 | 不检查文件权限, 可能覆盖只读文件 |

### 2.3 Agent Tool Loop 不足

| 问题 | 严重度 | 说明 |
|------|--------|------|
| 无并行工具执行 | 中 | 多个 tool_calls 串行执行, 可并行 |
| 无审批机制 | 高 | Supervised 模式下不请求用户确认敏感操作 |
| 无中间结果显示 | 中 | 工具执行过程对用户不可见 (无 CLI 实时输出) |
| 历史不保存中间工具调用 | 中 | 只保存最终 user+assistant, 丢失工具调用上下文 |
| 无错误恢复策略 | 中 | 工具失败只返回错误文本, 不自动重试 |
| 最大迭代数硬编码 | 低 | MAX_TOOL_ITERATIONS=10 不可配置 |
| 无 token 预算追踪 | 中 | 多轮工具调用可能超出 context window |

### 2.4 Provider 工具调用不足

| 问题 | 严重度 | 说明 |
|------|--------|------|
| 仅 OpenAI function calling 格式 | 高 | 不支持 Anthropic原生 tool_use 格式 |
| 无降级方案 | 中 | 不支持原生工具的 Provider 无文本解析降级 |
| 无 tool_choice 参数 | 中 | 无法强制使用/禁止使用特定工具 |
| 响应解析容错性差 | 低 | tool_calls arguments 解析失败直接返回 Null |

### 2.5 Memory 集成不足

| 问题 | 严重度 | 说明 |
|------|--------|------|
| 对话历史不持久化 | 高 | Agent history 仅在内存, 进程退出即丢失 |
| 无自动记忆召回 | 高 | chat() 前不从 Memory 检索相关记忆注入上下文 |
| 无自动记忆存储 | 中 | chat() 后不自动提取重要事实存入 Memory |
| recall 为关键词匹配 | 中 | 无语义搜索 (embedding) |
| MarkdownMemory 无 frontmatter 解析 | 低 | get() 方法不解析 frontmatter, 丢失元数据 |

### 2.6 其他不足

| 问题 | 严重度 | 说明 |
|------|--------|------|
| 无 SSE 流式响应 | 高 | chat() 等待完整响应, 无实时流式输出 |
| 无上下文窗口管理 | 高 | history 无限增长, 最终超出 LLM context window |
| 无 system prompt 自定义 | 中 | system prompt 硬编码 |
| config set 未实现 | 中 | 只能手动编辑配置文件 |
| 无多 Agent 支持 | 低 | 单 Agent, 无法多 Agent 协作 |

---

## 三、开发路线图

### P0 — 核心可用性 (近期)

1. **对话历史持久化到 Memory**
   - Agent::chat() 结束后自动保存对话摘要到 Memory
   - 启动时从 Memory 加载历史上下文
   - 文件: `crates/shadow-runtime/src/agent.rs`

2. **上下文窗口管理**
   - 设置最大 history 条数 / token 估算
   - 超限时自动截断旧消息 (保留 system + 最近 N 条)
   - 文件: `crates/shadow-runtime/src/agent.rs`

3. **SSE 流式响应**
   - Provider::chat_stream() 返回 Stream
   - CLI 逐字输出
   - 文件: `agent-core/src/provider.rs`, `crates/shadow-providers/src/openai.rs`

4. **Supervised 模式审批**
   - 敏感工具 (Shell/FileWrite) 执行前请求用户确认
   - 通过 Channel trait 的 supports_approval() 实现
   - 文件: `crates/shadow-runtime/src/agent.rs`

5. **工具超时机制**
   - Tool trait 添加 `timeout() -> Option<Duration>` 方法
   - Agent 用 tokio::time::timeout 包装 execute()
   - 文件: `agent-core/src/tool.rs`, `crates/shadow-runtime/src/agent.rs`

### P1 — 安全与健壮性 (中期)

6. **参数校验框架**
   - 在 Tool::execute() 前自动校验 args 符合 parameters_schema
   - 使用 jsonschema crate 做运行时校验
   - 文件: `agent-core/src/tool.rs`

7. **ShellTool 安全增强**
   - 命令白名单/黑名单 (可配置)
   - 超时 (默认 30s)
   - 工作目录隔离
   - 文件: `crates/shadow-runtime/src/tools/shell.rs`

8. **FileWrite 原子写入**
   - 先写临时文件, 再 rename
   - 支持 append 模式
   - 文件: `crates/shadow-runtime/src/tools/file_write.rs`

9. **工具执行中间结果显示**
   - CLI 模式下实时显示工具调用: `[工具] shell: echo hello`
   - 文件: `src/main.rs`

10. **config set 实现**
    - 支持 `shadow config set agent.model gpt-4o`
    - 支持 `shadow config set providers.openai.default.api_key sk-xxx`
    - 文件: `src/main.rs`, `crates/shadow-config/src/schema.rs`

### P2 — 功能扩展 (远期)

11. **Anthropic Provider**
    - 原生 tool_use 格式支持
    - Claude 系列模型
    - 文件: `crates/shadow-providers/src/anthropic.rs`

12. **文本解析降级**
    - 不支持原生工具的 Provider, 解析 XML/JSON 格式的工具调用
    - 文件: `crates/shadow-providers/src/`

13. **语义搜索记忆**
    - 基于 embedding 的 Memory recall
    - 文件: `crates/shadow-memory/src/`

14. **并行工具执行**
    - 多个 tool_calls 用 futures::join_all 并行执行
    - 文件: `crates/shadow-runtime/src/agent.rs`

15. **工具注册表**
    - 动态注册/注销工具
    - 工具发现机制 (类似 MCP)
    - 文件: `crates/shadow-runtime/src/tools/registry.rs`

16. **多 Agent 协作**
    - Agent 间消息传递
    - 任务委派
    - 文件: `crates/shadow-runtime/src/`

17. **System Prompt 自定义**
    - 配置文件支持自定义 system prompt
    - 支持 prompt 模板变量
    - 文件: `crates/shadow-config/src/schema.rs`

---

## 四、技术决策待定项

### 4.1 参数校验方案

**选项 A**: 运行时 JSON Schema 校验 (jsonschema crate)
- 优点: 灵活, 标准
- 缺点: 运行时开销, 依赖额外 crate

**选项 B**: 编译期类型化参数 (关联类型)
- 优点: 零开销, 类型安全
- 缺点: 失去动态性, trait object 不兼容

**推荐**: A, 保持 trait object 兼容性

### 4.2 流式响应方案

**选项 A**: `async fn chat_stream() -> impl Stream<Item = String>`
- 优点: 简单
- 缺点: trait object 不兼容 (async fn in trait)

**选项 B**: `fn chat_stream() -> BoxStream<String>`
- 优点: trait object 兼容
- 缺点: 额外 Box

**选项 C**: Channel-based (tokio::mpsc)
- 优点: 灵活, 支持背压
- 缺点: 更复杂

**推荐**: B, 使用 BoxStream

### 4.3 历史持久化方案

**选项 A**: 每轮对话存一条 MemoryEntry
- 优点: 简单, 可搜索
- 缺点: 对话上下文丢失

**选项 B**: 对话摘要存 MemoryEntry + 原始消息存 JSONL
- 优点: 上下文完整, 可搜索
- 缺点: 双重存储

**选项 C**: 仅存 JSONL, 启动时加载最近 N 条
- 优点: 简单, 单一存储
- 缺点: 不可搜索

**推荐**: C, 与日志系统复用 JSONL

---

## 五、优先级排序

```
P0 (核心可用性):
  1. 历史持久化        → 没有持久化, 多轮对话无意义
  2. 上下文窗口管理     → 没有管理, 长对话会崩
  3. SSE 流式响应      → 用户体验关键
  4. Supervised 审批   → 安全关键
  5. 工具超时          → 防止卡死

P1 (安全与健壮性):
  6. 参数校验
  7. Shell 安全增强
  8. FileWrite 原子写入
  9. 工具执行可视化
  10. config set

P2 (功能扩展):
  11. Anthropic Provider
  12. 文本解析降级
  13. 语义搜索记忆
  14. 并行工具执行
  15. 工具注册表
  16. 多 Agent
  17. System Prompt 自定义
```

---

## 六、总结

当前 Shadow 已具备基本的 Agent 工具调用能力, 但在安全性、持久化、流式输出方面有明显不足。P0 项是使其真正可用的必要条件, P1 项是生产环境就绪的必要条件, P2 项是功能丰富的远期目标。

核心设计原则保持不变: trait 驱动、微内核、归因系统、双模式构建。
