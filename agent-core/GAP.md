# agent-core 差距分析 -- 对照 ZeroClaw

## 当前状态 (Shadow)
- 6 trait: Attributable, ModelProvider, Tool, Memory, Observer, Channel
- Attributable blanket impl (Arc/Box/&)
- 6 Role (Agent/Channel/Tool/Provider/Memory/System)
- AutonomyLevel (Full/Supervised/ReadOnly)
- 共 ~500 行

## ZeroClaw 对应 (zeroclaw-api: 6293行, 20文件)
- 7 trait: ModelProvider, Channel, Tool, Memory, Observer, RuntimeAdapter, Peripheral
- Attributable + 14 Role + 72 ModelProviderKind + 37 ChannelKind + 11 MemoryKind
- SchemaCleanr (跨提供商 JSON Schema 清洗)
- jsonrpc 模块 (441行)
- elicitation 模块 (454行)
- principal 模块 (350行)
- HookHandler trait (15方法: 9 Void + 6 Modifying)
- task_local 上下文 (TOOL_LOOP_THREAD_ID 等 4 个)
- tool_attribution! / mock_tool_attribution! 宏

## 缺失项
| 功能 | 严重度 | ZeroClaw 实现 | Shadow 状态 |
|------|--------|--------------|-------------|
| RuntimeAdapter trait | P1 | shell/fs/storage/long_running 抽象 | 缺失 |
| Peripheral trait | P2 | 硬件外设 connect/disconnect/tools | 缺失 |
| HookHandler trait | P1 | 15 方法, Void+Modifying 二分 | 缺失 |
| SchemaCleanr | P1 | Gemini/Anthropic/OpenAI 三策略 | 缺失 |
| Role 子枚举 | P2 | 72 provider kind + 37 channel kind | 用字符串替代 |
| jsonrpc 模块 | P2 | JSON-RPC 2.0 类型 | 缺失 |
| elicitation | P2 | 用户交互请求 | 缺失 |
| task_local 上下文 | P1 | 4 个线程本地变量 | 缺失 |

## 开发建议
1. P1: 加 HookHandler trait (agent 生命周期钩子)
2. P1: 加 RuntimeAdapter trait (执行环境抽象)
3. P1: 加 task_local 上下文 (tool loop 状态)
4. P2: 加 SchemaCleanr (多 provider 工具规格清洗)
5. P2: 加 Peripheral trait (硬件扩展)
