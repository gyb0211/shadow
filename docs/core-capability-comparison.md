# Shadow vs Hermes vs Claude Code vs ZeroClaw 核心能力技术实现详细对比

> 生成时间: 2026-07-05
> Shadow: Rust 23887行 17crate 486测试 | Hermes: TypeScript ~100000行 | Claude Code: TypeScript ~50000行 | ZeroClaw: Rust 608951行 23crate v0.8.2

本文档从 14 个核心能力维度出发，对 Shadow、Hermes、Claude Code、ZeroClaw 四个 AI Agent 系统的技术实现方式进行深度对比分析。每个维度涵盖四个系统的实现方式、使用的第三方库、核心设计原理、优势与劣势。

---

## 目录

1. 架构与核心 Trait 设计
2. Tool 系统与工具调用
3. Provider 模型接入层
4. Memory 记忆系统
5. Agent 循环与推理控制
6. Proxy 多智能体协作
7. 日志与可观测性
8. 配置系统
9. 安全机制
10. TUI 终端界面
11. Skills 技能系统
12. Channel 渠道集成
13. 插件与扩展系统
14. Gateway 与基础设施

---

## 1. 架构与核心 Trait 设计

### Shadow

**实现方式**: Rust trait 驱动微内核架构，17 个 crate 分层组织。核心层 shadow-core 定义 6 个基础 trait: Attributable（归因超 trait，提供 role/alias）、Provider（模型接入）、Tool（工具调用）、Memory（记忆存储）、Channel（消息渠道）、Observer（事件观察）。所有 trait 继承 Attributable，形成统一的归因体系。运行时层 shadow-runtime 组装各组件并驱动 Agent 循环。采用双模式构建: 默认完整模式包含所有功能，kernel-only 模式仅保留核心 trait 定义，支持交叉编译到 ARM 等目标平台。

**依赖库**: async-trait（trait 中 async 方法）、serde/serde_json（序列化）、anyhow（错误处理）、parking_lot（同步原语）。

**原理**: trait 对象多态（Arc<dyn Tool> / Arc<dyn Provider>）。每个 trait 定义最小的行为契约，实现方通过工厂函数按字符串 key 注册。Attributable 超 trait 为所有组件提供统一的角色标识（Role 枚举: Provider/Tool/Memory/Channel/Observer/Proxy/Agent），支持归因传播和审计。Crate 之间通过 trait 定义解耦，shadow-core 不依赖任何 HTTP 库（保持 HTTP-agnostic），由 shadow-providers 在调用时注入 reqwest。

**优势**: 编译期类型安全，零运行时开销（除 trait 对象 vtable 外）；Crate 级隔离实现最小权限依赖；双模式构建适配嵌入式场景；交叉编译 ARM 支持。

**劣势**: 仅 6 个 trait，相比 ZeroClaw 的 7 核心 trait + 5 辅助 trait 仍有差距；缺少 RuntimeAdapter（执行环境抽象）、Peripheral（硬件外设）、HookHandler（生命周期钩子）等 trait；trait 对象有动态分发开销。

### Hermes

**实现方式**: TypeScript 插件式 Agent 架构，约 100000 行代码。核心是 Agent 类，通过组合模式注入 Provider、Memory、Channel、Tool 等组件。使用 TypeScript interface 定义插件契约，运行时动态加载。没有 crate 级隔离，所有代码在一个大仓库中通过目录组织: core/（引擎）、channels/（消息渠道）、skills/（技能）、plugins/（插件）、browser/（浏览器工具）等。

**依赖库**: Node.js 生态。zod（运行时类型校验）、fastify/express（HTTP 服务）、better-sqlite3（SQLite）、puppeteer/playwright（浏览器自动化）等。

**原理**: 基于 interface 的鸭子类型多态。Agent 在初始化时组装各组件，通过依赖注入方式传递。插件通过 register() 方法动态注册，Agent 维护一个插件列表并在事件发生时遍历调用。没有编译期类型保证（interface 运行时擦除），但开发速度快。

**优势**: 生态丰富，可利用整个 npm 生态；开发迭代速度快；插件式架构扩展灵活；支持 30+ 种消息渠道集成。

**劣势**: 无编译期类型保证（interface 运行时擦除）；动态类型导致运行时错误风险高；无 crate 级隔离，依赖管理容易膨胀；单仓库缺乏严格的依赖边界。

### Claude Code

**实现方式**: TypeScript 单体 CLI Agent 架构，约 50000 行代码。专注代码生成场景，深度集成 Anthropic API。不采用 trait/插件式架构，而是以函数组合方式构建。核心是 prompt 工程 + tool calling 循环，通过 Anthropic SDK 直接调用 Messages API。工具集固定（Read/Write/Edit/Bash/Search 等），不支持动态注册第三方工具。

**依赖库**: @anthropic-ai/sdk（Anthropic 官方 SDK）、Node.js 内置模块。

**原理**: 深度绑定 Anthropic Messages API，利用 Anthropic 原生 tool_use 机制实现工具调用。prompt 模板针对代码场景优化（codebase 上下文注入、diff 生成、文件操作）。没有抽象层，Agent 直接调用 SDK 方法，工具定义硬编码在 Agent 内部。

**优势**: 与 Anthropic API 深度集成，利用原生能力（extended_thinking、prompt_caching）；工具集精炼，针对代码场景高度优化；无多余抽象层开销；启动快。

**劣势**: 强绑定 Anthropic，不支持其他模型提供商；无插件系统，不可扩展第三方工具；无 trait/接口抽象，难以替换组件；不支持多 Agent 协作。

### ZeroClaw

**实现方式**: Rust trait 驱动微内核架构，23 个 crate 分层组织，608951 行代码。定义 7 个核心 trait: ModelProvider、Channel、Tool、Memory、Observer、RuntimeAdapter、Peripheral。Attributable 超 trait 拥有 Role 枚举体系（37 种 Channel、60 种 Provider、11 种 Memory 等角色）。采用宏驱动设计: Configurable derive 宏（14 个属性）自动生成配置代码，tool_attribution! 宏简化归因实现。双模式构建: --no-default-features 编译为微内核。

**依赖库**: async-trait、serde/serde_json、tokio（异步运行时）、ratatui（TUI）、reqwest（HTTP）、rusqlite（SQLite）、wasmtime（WASM 运行时）、ring（密码学）、axum（HTTP 服务）、toml_edit（保留注释的 TOML 编辑）、schemars（JSON Schema 导出）。

**原理**: 与 Shadow 类似的 trait 对象多态，但规模远超。Crate 分层: zeroclaw-api（trait 定义）到 zeroclaw-config/zeroclaw-log（基础设施）到 zeroclaw-providers/zeroclaw-memory/zeroclaw-channels/zeroclaw-tools（实现）到 zeroclaw-runtime（引擎）到 zeroclaw-gateway（HTTP API）到 zeroclaw-plugins（WASM）到 zeroclaw-hardware（硬件）到 zerocode（TUI）到 robot-kit/aardvark-sys（机器人）。Attributable 的 Role 枚举体系支持 100+ 种角色，归因传播贯穿整个调用链。

**优势**: 类型安全 + 全栈覆盖（从 trait 到硬件）；Crate 级严格隔离；宏驱动减少样板代码；7 核心 trait 覆盖所有能力域；Attributable 归因体系支持完整审计链。

**劣势**: 608951 行代码量庞大，编译时间长；学习曲线陡峭；部分 crate 过度工程化；trait 定义复杂（如 Memory trait 有 25+ 方法）。

---

## 2. Tool 系统与工具调用

### Shadow

**实现方式**: Tool trait 定义 7 个方法: name()、description()、parameters_schema()、execute()、timeout()、requires_approval()、validate_args()。ToolRegistry 管理动态注册的工具集合。工具装饰器模式: PathGuardedTool 包装任意 Tool 限制文件访问路径，RateLimitedTool 包装任意 Tool 实现令牌桶限流。Agent 在工具调用循环中: (1) 从 LLM 响应解析 tool_calls；(2) 在 registry 中查找工具；(3) 调用 validate_args() 校验参数；(4) 用 tokio::time::timeout 包装 execute()；(5) 返回 ToolResult。内置工具: ShellTool（命令执行，黑名单拦截）、FileReadTool、FileWriteTool（原子写入）、FileSearchTool、MemoryStoreTool、SkillTool 等。

**依赖库**: async-trait、jsonschema（运行时参数校验，自动检测 draft 版本）、tokio::time（超时控制）、serde_json（参数序列化）、regex（黑名单模式匹配）。

**原理**: Tool trait 继承 Attributable，所有工具的 Role 都是 Tool。validate_args() 默认实现使用 jsonschema crate 编译 parameters_schema() 返回的 JSON Schema 为校验器，然后验证传入参数。timeout() 默认 30 秒，敏感工具可覆盖。requires_approval() 默认 false，ShellTool/FileWriteTool 覆盖为 true。装饰器模式通过内部持有 Arc<dyn Tool> 实现，不修改被包装工具的代码。

**优势**: jsonschema 运行时参数校验是四者中独有特性，在工具执行前拦截非法参数；工具装饰器模式实现安全策略与工具逻辑解耦（PathGuardedTool 可包装任意工具限制路径）；动态注册支持运行时扩展；超时和审批机制完善。

**劣势**: 工具种类较少（约 10 种），缺少浏览器工具、HTTP 服务器工具、语音工具等；不支持工具并行执行；不支持流式工具输出；不支持 WASM 插件工具；缺少 MCP（Model Context Protocol）支持（仅有设计）。

### Hermes

**实现方式**: 工具以 TypeScript class 实现 ToolProvider 接口，通过 register_tool() 动态注册。Agent 维护工具列表，在 LLM 返回工具调用请求后查找并执行。支持子代理委派（delegate_task）: 主 Agent 可以将子任务分配给子 Agent 执行。工具集丰富: Shell、File、Browser（基于 Puppeteer/Playwright）、GitHub（gh CLI）、Email（himalaya）、Memory、Skill 等。支持流式工具输出，适合长时间运行的任务。

**依赖库**: zod（参数校验）、puppeteer/playwright（浏览器自动化）、child_process（Shell 执行）、better-sqlite3（SQLite 操作）。

**原理**: 基于 interface 的工具注册和调用。Agent 在收到 LLM 的 tool_use 请求后，通过工具名查找对应实例，调用 execute() 方法。delegate_task 允许 Agent 将复杂任务分解给子 Agent，子 Agent 在隔离的上下文中执行，结果返回给主 Agent。

**优势**: 工具种类丰富（浏览器自动化、GitHub、Email 等）；支持子代理委派和隔离；流式工具输出支持长任务；MCP（Model Context Protocol）原生支持；浏览器工具基于 Puppeteer/Playwright 功能强大。

**劣势**: 无运行时参数校验（依赖 TypeScript 类型，运行时擦除）；无工具超时机制（依赖手动设置）；无工具审批机制；无工具装饰器模式（安全策略需手动在每个工具中实现）。

### Claude Code

**实现方式**: 工具集固定硬编码，不支持动态注册。核心工具: Read（文件读取，支持行号、分页）、Write（文件写入）、Edit（精确替换编辑）、Bash（Shell 执行）、Search（文件搜索，支持正则）、Glob（文件模式匹配）。工具定义直接传给 Anthropic API 的 tools 参数。工具执行结果通过 tool_result 格式回传给 LLM。

**依赖库**: @anthropic-ai/sdk（Anthropic SDK 内置工具调用支持）、Node.js fs（文件操作）、child_process（Shell 执行）。

**原理**: 利用 Anthropic Messages API 原生 tool_use 机制。工具定义以 JSON Schema 格式传给 API，LLM 返回 tool_use 块后 Agent 执行对应工具，结果以 tool_result 格式回传。Edit 工具使用精确的 old_string/new_string 替换模式，而非 diff，减少 LLM 出错概率。

**优势**: 工具集精炼，针对代码场景高度优化；Edit 工具的精确替换模式减少 diff 冲突；与 Anthropic API 原生集成，无适配层开销；Read 工具支持行号和分页，适合大文件。

**劣势**: 工具集固定，不支持扩展；不支持其他 LLM 提供商；无参数校验；无工具超时（依赖 Bash 自身的 timeout）；无工具审批机制；无装饰器模式。

### ZeroClaw

**实现方式**: Tool trait 定义核心方法，ToolsPayload 实现多态分发（一次调用可路由到多个工具实例）。内置工具: ShellTool（安全策略）、FileTool、MemoryTool、BrowserTool、FetchUrlTool、SearchTool、SpawnSubagentTool、HttpRequestTool、HttpServerTool、SOPTool、VoiceTool 等。WASM 插件工具: WasmTool 实现 Tool trait，通过 wasmtime 执行 WASM 模块。PromptGuided 降级: 当 LLM 不支持原生工具调用时，自动降级为 prompt 模板注入方式。tool_attribution! 宏简化归因实现。

**依赖库**: async-trait、wasmtime（WASM 运行时，43 个组件）、ring（Ed25519 签名验证）、reqwest（HTTP 请求工具）、rusqlite（文件/记忆工具）。

**原理**: Tool trait 继承 Attributable，所有工具的 Role 为 Tool。ToolsPayload 使用 enum 包装多种工具类型，实现多态分发: 一次工具调用可以路由到 Native 工具、WASM 工具或 Prompt 引导工具。WasmTool 通过 wasmtime 的 component model（WIT 接口定义）加载和执行 WASM 模块，每个 WASM 工具在独立的 fuel 限制沙箱中运行。Ed25519 签名验证确保插件来源可信。

**优势**: WASM 插件支持无限扩展（安全沙箱隔离）；ToolsPayload 多态分发支持混合工具类型；PromptGuided 降级支持不支持原生工具调用的 LLM；工具种类最丰富（Shell/File/Memory/Browser/FetchUrl/Search/SpawnSubagent/HttpRequest/HttpServer/SOP/Voice）；Ed25519 签名验证确保插件安全。

**劣势**: WASM 运行时增加二进制大小和启动时间；工具调用链路复杂（Native/WASM/Prompt 三种路径）；学习曲线陡峭；部分工具依赖外部系统（如 VoiceTool 依赖语音服务）。

---

## 3. Provider 模型接入层

### Shadow

**实现方式**: Provider trait 定义 chat()、chat_stream()、list_models()、supports_native_tools()、default_temperature() 等方法。chat() 返回完整 ChatResponse，chat_stream() 返回 BoxStream<ChatChunk> 支持流式输出。ChatChunk 枚举: ContentDelta（文本增量）、ReasoningDelta（思考增量）、ToolCallDelta（工具调用增量）、Done（流结束）。三个实现: OpenAiProvider（OpenAI 兼容 API，function calling 格式）、AnthropicProvider（Anthropic Messages API，tool_use 格式）、ReliableModelProvider（重试 + key 轮换装饰器）。RouterModelProvider 支持 hint 路由: 根据请求中的 hint 字段路由到不同的底层 provider。ModelProviderRuntimeOptions 传递 HTTP 层细节（超时、推理强度、自定义 API path、额外 headers、认证方式）。AuthStyle 枚举: Bearer / XApiKey / Query(String)。

**依赖库**: reqwest（HTTP 客户端）、serde_json（请求/响应序列化）、futures::stream::BoxStream（流式输出）、async-trait。chat_stream 默认实现包装 chat() 为单个 Done chunk，支持 SSE 的 provider 覆写此方法。

**原理**: Provider 工厂函数按字符串 key（如 "openai"、"anthropic"）创建实例。ReliableModelProvider 作为装饰器包装底层 provider，实现: (1) 重试逻辑（可配置重试次数和退避策略）；(2) API key 轮换（多个 key 循环使用，遇到 429/5xx 自动切换）。RouterModelProvider 维护 hint -> provider 的映射表，根据 ChatRequest 中的 hint 字段选择 provider。ChatMessage 支持 reasoning_content 字段（DeepSeek-R1、GLM-4.7 等思考模型），从 API 响应解析后回传给 API。

**优势**: 支持 OpenAI 和 Anthropic 两大主流 API 格式；ReliableModelProvider 装饰器实现重试和 key 轮换不侵入底层 provider；流式输出支持思考内容分离；hint 路由支持多模型智能选择；AuthStyle 灵活支持不同 API key 注入方式；HTTP-agnostic 设计（shadow-core 不依赖 reqwest）。

**劣势**: 仅 2 个原生 provider 实现（OpenAI + Anthropic），远少于 ZeroClaw 的 50+ family；不支持 Gemini、Azure OpenAI、AWS Bedrock、OpenRouter 等；不支持 OAuth 认证；不支持多模态（vision）；不支持 TTS/语音转写；不支持流式工具事件；无原生 thinking 支持。

### Hermes

**实现方式**: Provider 以 TypeScript class 实现 LLMProvider 接口，提供 chat()、streamChat()、listModels() 方法。支持多种 provider: OpenAI 兼容、Anthropic、Gemini、Ollama 本地模型等。通过配置文件指定 provider 类型和 API key。支持 provider 降级（fallback）: 主 provider 失败时自动切换到备用 provider。支持 TTS 语音合成。

**依赖库**: openai（OpenAI Node SDK）、@anthropic-ai/sdk（Anthropic SDK）、@google/generative-ai（Gemini SDK）、ollama（本地模型客户端）。

**原理**: Agent 在初始化时根据配置创建 provider 实例并注入。流式输出通过 Node.js 的 AsyncIterator 实现。provider 降级通过 try-catch 包装: 主 provider 抛出异常后，自动尝试备用 provider。

**优势**: 支持主流 provider（OpenAI/Anthropic/Gemini/Ollama）；provider 降级提高可用性；TTS 语音合成支持；利用各官方 SDK 减少适配工作；开发迭代快。

**劣势**: 无 key 轮换机制（单个 key 用尽即失败）；无重试退避策略（依赖 SDK 默认行为）；无多模型路由（不支持根据任务复杂度选择不同模型）；无速率限制管理；不支持多模态；无成本追踪。

### Claude Code

**实现方式**: 直接使用 @anthropic-ai/sdk 调用 Anthropic Messages API。不支持其他模型提供商。利用 Anthropic 原生能力: extended_thinking（扩展思考）、prompt_caching（提示缓存，减少重复 token 消耗）。流式输出通过 SDK 的 stream 方法实现。

**依赖库**: @anthropic-ai/sdk（Anthropic 官方 SDK，唯一 provider）。

**原理**: 所有请求直接通过 Anthropic SDK 发送。工具定义作为 tools 参数传入 Messages API。利用 prompt_caching 缓存 system prompt 和历史消息，减少 token 消耗。extended_thinking 允许模型在回答前进行更深入的推理。

**优势**: 利用 Anthropic 原生能力（extended_thinking、prompt_caching）获得最佳性能；无需适配层，调用链路最短；prompt_caching 显著降低成本；extended_thinking 提升复杂推理质量。

**劣势**: 强绑定 Anthropic，无法切换其他模型；无 key 轮换；无重试退避；无 provider 降级；无多模型路由；不支持 Ollama 等本地模型。

### ZeroClaw

**实现方式**: ModelProvider trait 定义 chat/chat_stream/list_models/pricing/capabilities/default_base_url 等方法。50+ provider family 通过 CompatFamilySpec 声明式定义（每个 family 声明 API 端点、认证方式、工具格式等）。ReliableModelProvider 装饰器: 重试 + 退避 + key 轮换。RouterModelProvider: CostOptimized 路由（根据成本选择最优模型）、hint 路由。OAuth 认证: 多 provider 支持 OAuth 2.0 流程。多模态支持: vision（图片输入）、multimodal（音频/视频）。TTS 语音合成（7 种引擎）。语音转写（7 种引擎）。流式工具事件: 在流式响应中携带工具调用增量。

**依赖库**: reqwest（HTTP）、serde_json（序列化）、ring（OAuth 签名）、async-trait、futures（流式）。CompatFamilySpec 使用声明式宏定义 provider family 规格。

**原理**: CompatFamilySpec 是声明式 provider 定义: 每种 provider family 通过一个 struct 声明 API base URL、认证方式（Bearer/XApiKey/Query/OAuth）、工具调用格式（function calling/tool_use/prompt）、流式格式等。ReliableModelProvider 包装底层 provider，在遇到 429/5xx 时自动重试并轮换 key。RouterModelProvider 的 CostOptimized 模式根据各模型的定价和任务复杂度选择性价比最高的模型。

**优势**: 50+ provider family 覆盖几乎所有主流 LLM（OpenAI/Anthropic/Gemini/Azure/Bedrock/Ollama/OpenRouter/Copilot 等）；声明式 provider 定义减少代码量；OAuth 认证支持 GitHub Copilot 等需要 OAuth 的 provider；多模态支持（vision/multimodal）；TTS 和语音转写（各 7 种引擎）；CostOptimized 路由自动优化成本；流式工具事件支持实时工具调用增量；模型定价信息内置。

**劣势**: 50+ family 的维护成本高；部分 family 可能缺乏充分测试；CompatFamilySpec 声明式定义灵活性有限（复杂 provider 需要自定义实现）；二进制体积大（包含所有 provider 代码）。

---

## 4. Memory 记忆系统

### Shadow

**实现方式**: Memory trait 定义 store()、recall()、get()、list()、forget()、count()、health_check() 等方法。MemoryCategory 枚举: Core（长期事实/偏好）、Daily（日常会话）、Conversation（对话上下文）、Custom(String)（自定义分类）。三个实现: NoneMemory（空实现，占位）、MarkdownMemory（Markdown 文件持久化）、SqliteMemory（SQLite + FTS5 trigram 全文搜索 + 向量检索）。EmbeddingProvider trait 抽象 embedding API。hybrid_merge 函数融合 FTS5 和向量检索结果。MemoryStrategy trait: before_chat()（对话前检索相关记忆）、after_chat()（对话后提取并存储重要事实）。

**依赖库**: rusqlite（SQLite + FTS5 trigram tokenizer）、reqwest（embedding API 调用）、async-trait、serde（序列化）。FTS5 trigram tokenizer 适用于中文/日文等非空格分隔语言。

**原理**: SqliteMemory 双路检索: (1) FTS5 trigram 全文搜索（关键词匹配，适合精确查询）；(2) embedding 向量余弦相似度检索（语义匹配，适合模糊查询）。hybrid_merge 融合两路结果: 按 score 归一化后加权排序。MemoryStrategy 在 Agent 循环中调用: before_chat 检索记忆并注入 system prompt，after_chat 提取重要事实并存储。MemoryEntry 包含 id、key、content、category、timestamp、session_id、score、agent_alias 字段。

**优势**: FTS5 trigram 支持中文全文搜索（无需分词器）；embedding 向量检索支持语义匹配；hybrid_merge 融合关键词和语义两路结果；MemoryCategory 分类管理记忆；MemoryStrategy 抽象记忆加载/存储策略；HTTP-agnostic 设计（shadow-core 不依赖 reqwest）；NoneMemory 空实现支持无记忆模式。

**劣势**: 仅 3 种后端（None/Markdown/SQLite），远少于 ZeroClaw 的 7 种；无 Postgres/Qdrant 等远程后端；无记忆巩固（consolidation）；无冲突检测；无时间衰减（decay）；无重要性评分（importance）；无知识图谱；无响应缓存；无审计日志；无策略执行；无文本分块（chunker）。

### Hermes

**实现方式**: 三层记忆架构: user 层（用户偏好/个人信息）、memory 层（对话历史/事实）、skills 层（技能记忆）。SQLite 持久化存储。支持跨会话搜索。记忆巩固: 通过 LLM 驱动的方式对旧记忆进行总结和压缩。

**依赖库**: better-sqlite3（SQLite 客户端）、zod（数据验证）。

**原理**: 三层记忆分别存储不同粒度的信息。user 层在每次对话开始时加载，提供个性化上下文。memory 层按时间顺序存储对话片段。skills 层存储技能执行结果和经验。记忆巩固通过 LLM 总结旧记忆，提取关键信息后丢弃原始对话。

**优势**: 三层记忆架构清晰分离不同信息类型；LLM 驱动的记忆巩固减少存储压力；跨会话搜索支持历史回顾；与 Obsidian/Vault 集成支持知识管理。

**劣势**: 无语义搜索（仅关键词匹配）；无向量检索；无 embedding 支持；无记忆分类体系；无冲突检测；无时间衰减；无重要性评分；无知识图谱；单后端（仅 SQLite）。

### Claude Code

**实现方式**: 对话上下文自动压缩。当对话历史超过 token 限制时，自动总结早期对话并压缩。无独立记忆系统，记忆依赖对话上下文和文件系统（CLAUDE.md 文件存储项目偏好）。

**依赖库**: 无独立记忆库，依赖 Anthropic SDK 的上下文管理。

**原理**: 利用 LLM 自身的上下文窗口管理记忆。当上下文溢出时，调用 LLM 总结早期对话，保留关键信息丢弃细节。CLAUDE.md 文件作为项目级记忆: 存储 coding conventions、项目结构、常见任务等。

**优势**: 实现简单，无需额外存储系统；上下文压缩由 LLM 自动完成，质量高；CLAUDE.md 文件方案轻量且版本可控。

**劣势**: 无持久化记忆（对话结束即丢失）；无跨会话搜索；无语义搜索；无向量检索；无记忆分类；无记忆巩固；无冲突检测；无重要性评分；完全依赖 LLM 上下文窗口，受 token 限制。

### ZeroClaw

**实现方式**: Memory trait 定义 25+ 方法，覆盖存储/检索/巩固/冲突/重要性/衰减/清理/知识图谱/响应缓存/分块/审计/策略等。7 种后端: SQLite（FTS5 + 向量）、Markdown（文件持久化）、Postgres（远程关系数据库）、Qdrant（向量数据库）、Lucid（图数据库）、None（空实现）、AgentScoped（按 Agent 隔离）。embedding + hybrid_merge 融合检索。consolidation: LLM 驱动的记忆巩固。conflict: Jaccard 相似度冲突检测。importance: 重要性评分。decay: 时间衰减。hygiene: 记忆清理（去重、归档）。knowledge_graph: 知识图谱。response_cache: 响应缓存。chunker: 文本分块。audit: 审计日志。policy: 策略执行。

**依赖库**: rusqlite（SQLite + FTS5）、reqwest（embedding API）、qdrant-client（Qdrant 向量数据库）、tokio-postgres（Postgres）、ring（加密）、serde（序列化）。

**原理**: 7 种后端实现同一 Memory trait，通过工厂函数按字符串 key 创建。SQLite 后端与 Shadow 类似（FTS5 trigram + 向量）。Postgres 后端支持远程存储和大规模数据。Qdrant 后端专注于高维向量检索。Lucid 后端支持知识图谱存储和图查询。AgentScoped 后端为每个 Agent 创建独立的记忆空间。consolidation 通过 LLM 总结旧记忆。conflict 使用 Jaccard 相似度检测重复/矛盾记忆。importance 根据访问频率、时间衰减、来源可信度等计算重要性评分。decay 按时间衰减记忆权重。hygiene 定期清理低重要性、过时、重复的记忆。knowledge_graph 构建实体-关系图谱。

**优势**: 7 种后端覆盖所有存储需求（本地/远程/向量/图/空/隔离）；25+ 方法覆盖完整记忆生命周期；知识图谱支持复杂关系查询；Qdrant 向量数据库支持大规模语义检索；冲突检测避免重复/矛盾记忆；重要性评分 + 时间衰减自动管理记忆优先级；响应缓存减少重复 LLM 调用；文本分块支持长文档处理；审计日志支持合规需求；AgentScoped 支持多 Agent 记忆隔离。

**劣势**: 25+ 方法的 trait 定义过于复杂；7 种后端维护成本高；部分高级功能（知识图谱、consolidation）依赖 LLM 调用，增加成本；Postgres/Qdrant 后端需要额外基础设施；学习曲线陡峭。

---

## 5. Agent 循环与推理控制

### Shadow

**实现方式**: Agent 持有 provider/tool/memory/observer，驱动工具调用循环。AgentConfig 配置: max_iterations（默认 10）、max_history（默认 50）、context_token_budget（默认 100000）、system_prompt、autonomy（AutonomyLevel）。循环检测: LoopDetector 记录工具调用序列，检测重复模式（连续相同工具调用、参数循环等）。上下文溢出恢复: 当 token 超预算时，自动截断旧消息。流式回调: StreamDeltaCallback（Content/Reasoning 增量）、ToolEventCallback（tool_start/tool_success/tool_error/tool_timeout）。token 预算: 预估消息 token 数，超预算时触发截断。对话后技能审查: skill_review_enabled，工具调用次数达到阈值时触发审查。

**依赖库**: tokio（异步运行时）、parking_lot::Mutex（线程安全）、futures::StreamExt（流式消费）、shadow-log（Action 枚举日志）。

**原理**: Agent 循环: (1) 构建 ChatRequest（system prompt + memory context + history + user message + tools）；(2) 调用 provider.chat_stream()；(3) 消费流式 chunk，通过回调推送给 TUI/CLI；(4) 解析 tool_calls；(5) 对每个工具调用: validate_args -> check_approval -> execute(timeout)；(6) 将工具结果加入 history；(7) 检查 LoopDetector；(8) 检查 token 预算；(9) 重复直到无 tool_calls 或达到 max_iterations。LoopDetector 维护最近 N 次工具调用的哈希序列，检测: 完全重复、参数循环、工具交替等模式。

**优势**: 循环检测防止 Agent 陷入死循环（四者中仅 Shadow 和 ZeroClaw 有）；上下文溢出恢复防止 token 超限导致 API 错误；token 预算管理优化成本；流式回调支持实时显示（回答 + 思考分离）；技能审查触发自动改进。

**劣势**: 无生命周期 Hook（ZeroClaw 有 14 个 Hook）；无 SOP 标准操作流程；无 PromptSection 可插拔段；无 Anti-Narration 反叙述控制；无工具并行执行；max_history 截断策略简单（仅按条数，未考虑重要性）。

### Hermes

**实现方式**: Agent 循环: 接收用户输入 -> 调用 LLM -> 解析工具调用 -> 执行工具 -> 回传结果 -> 重复。支持上下文窗口管理（自动压缩历史）。支持子代理委派: delegate_task 将子任务分配给子 Agent，子 Agent 在隔离上下文中执行。

**依赖库**: Node.js AsyncIterator（流式处理）、zod（响应校验）。

**原理**: Agent 维护对话历史数组，每次调用 LLM 后解析响应。如果有工具调用，执行工具后将结果加入历史。当历史超过 token 限制时，自动总结早期对话。delegate_task 创建子 Agent 实例，传入子任务和上下文，子 Agent 独立运行后将结果返回。

**优势**: 子代理委派支持复杂任务分解；子代理隔离防止上下文污染；上下文自动压缩；开发简单。

**劣势**: 无循环检测（Agent 可能陷入死循环）；无上下文溢出恢复（依赖 LLM 自动压缩，可能丢失关键信息）；无 token 预算管理；无流式思考分离；无技能审查。

### Claude Code

**实现方式**: 专注代码生成的 Agent 循环。利用 Anthropic extended_thinking 进行深度推理。上下文窗口管理: prompt_caching 缓存 system prompt 和历史消息。自动代码审查: 在代码修改后自动检查语法和逻辑。

**依赖库**: @anthropic-ai/sdk（extended_thinking、prompt_caching）。

**原理**: Agent 循环: 构建 codebase 上下文 -> 调用 Messages API（含 extended_thinking） -> 解析 tool_use -> 执行工具（Read/Write/Edit/Bash） -> 回传 tool_result -> 重复。extended_thinking 允许模型在回答前进行更长的推理链。prompt_caching 通过缓存不变部分减少 token 消耗。

**优势**: extended_thinking 提供深度推理能力；prompt_caching 显著降低成本和延迟；针对代码场景优化；自动代码审查提升质量。

**劣势**: 无循环检测；无上下文溢出恢复；无 token 预算管理；无子代理委派；无流式思考分离（extended_thinking 内容不直接暴露）；强绑定 Anthropic。

### ZeroClaw

**实现方式**: Agent 引擎拥有 30+ 字段，7874 行代码。工具调用循环 + 14 个生命周期 Hook（Void/Modifying 两种类型）。SOP（Standard Operating Procedure）引擎: 7 种 SOP 类型支持标准化操作流程。PromptSection: 5 个可插拔段落（system/context/skills/memory/tools）。Anti-Narration: 防止 LLM 生成不必要的叙述。history_trim: 上下文窗口管理。循环检测: 与 Shadow 类似但更复杂。token 预算管理。多 Agent 配置: AliasedAgentConfig 支持多个 Agent 实例。

**依赖库**: tokio、parking_lot、futures、async-trait。

**原理**: 14 个 Hook 在 Agent 生命周期的关键点触发: before_chat、after_chat、before_tool、after_tool、before_llm、after_llm 等。Void Hook 只观察不修改（如日志、指标），Modifying Hook 可以修改数据（如注入额外上下文、过滤工具）。SOP 引擎允许定义标准操作流程: 步骤序列、条件分支、循环控制。PromptSection 以组合方式构建 system prompt: 各 section 独立管理，按顺序拼接。Anti-Narration 通过 prompt 模板和后处理过滤 LLM 的叙述性输出。

**优势**: 14 个生命周期 Hook 提供完整的扩展点（四者中独有）；SOP 引擎支持标准化操作流程；PromptSection 可插拔段落灵活组合 system prompt；Anti-Narration 减少不必要的 LLM 输出；循环检测 + 上下文溢出恢复；多 Agent 配置支持复杂场景。

**劣势**: 30+ 字段和 7874 行代码过于复杂；14 个 Hook 的执行顺序和依赖管理困难；SOP 引擎学习曲线陡峭；Anti-Narration 可能误过滤有用内容。

---

## 6. Proxy 多智能体协作

### Shadow

**实现方式**: AgentTransport trait 抽象 Agent 间通信: chat() 方法发送 prompt 并接收响应。三种实现: LocalAgent（进程内直接调用）、AcpClient（ACP 子进程协议）、A2aClient（A2A 远程协议）。TaskRouter 持有所有 transport 实例，按 agent 名称路由任务。AgentRegistry 管理 agent 元数据（名称、能力描述、在线状态）。HttpTransport（axum 实现）提供 HTTP API 接口。StdioTransport 提供 stdio JSON-RPC 接口。ProxyTool 实现 Tool trait，将远程 Agent 调用包装为工具。自动发现: 通过配置或扫描自动注册可用 Agent。

**依赖库**: axum（HTTP 服务器）、tokio（异步运行时）、serde_json（消息序列化）、parking_lot::RwLock（并发注册表）、parking_lot::Mutex（任务存储）。

**原理**: TaskRouter 维护 transport 映射表（agent_name -> Arc<dyn AgentTransport>）。dispatch(from, to, prompt) 查找目标 transport，调用 chat() 方法，返回 Task 对象（含状态: Pending/Running/Completed/Failed）。LocalAgent 直接持有 Agent 实例调用。AcpClient 通过 ACP 协议与子进程通信。A2aClient 通过 A2A 协议与远程 Agent 通信。ProxyTool 将 TaskRouter.dispatch 包装为 Tool，允许 LLM 通过工具调用委派子任务。

**优势**: 三种 transport 覆盖进程内/子进程/远程场景；TaskRouter 统一路由接口；AgentRegistry 支持 Agent 发现；ProxyTool 将多 Agent 协作包装为工具调用，对 LLM 透明；HTTP + stdio 双接口；设计精简（相比 ZeroClaw）。

**劣势**: 无子代理并行执行（dispatch 是同步等待）；无子代理隔离（LocalAgent 共享上下文）；无 Swarm 群体智能；无 Peer Groups 分组协作；无自动负载均衡。

### Hermes

**实现方式**: delegate_task 机制: 主 Agent 通过工具调用将子任务分配给子 Agent。子 Agent 在隔离的上下文中执行，结果返回给主 Agent。支持子代理并行执行: 多个 delegate_task 可以同时发起。

**依赖库**: Node.js child_process 或 worker_threads（子进程/线程隔离）。

**原理**: delegate_task 创建子 Agent 实例，传入子任务描述和必要上下文。子 Agent 独立运行工具调用循环，完成后将结果序列化返回。主 Agent 可以同时发起多个 delegate_task，通过 Promise.all 等待所有子任务完成。

**优势**: 子代理委派 + 隔离设计成熟；支持并行执行多个子任务；子 Agent 上下文隔离防止信息泄露；开发简单（基于 Promise）。

**劣势**: 无 ACP/A2A 协议支持（仅进程内）；无 Agent 注册/发现；无任务路由；无 HTTP/stdio Proxy Server；无 ProxyTool 抽象；无 Swarm 群体智能。

### Claude Code

**实现方式**: 不支持多 Agent 协作。单一 Agent 模式，所有工具调用在同一个 Agent 实例中完成。通过 ACP（Agent Communication Protocol）支持有限的子进程交互。

**依赖库**: 无多 Agent 相关库。ACP 支持基于 Node.js child_process。

**原理**: 单一 Agent 循环，不支持任务委派。ACP 支持通过子进程运行外部工具，但不是真正的多 Agent 协作。

**优势**: 实现简单；单一 Agent 上下文连贯，无需跨 Agent 通信；适合代码生成场景（单一任务流）。

**劣势**: 不支持多 Agent 协作；无任务委派；无 Agent 注册/发现；无任务路由；无 HTTP/stdio Proxy Server；无并行子任务。

### ZeroClaw

**实现方式**: 完整的多 Agent 协作平台。AgentTransport 抽象 + 多种实现（Local/ACP/A2A/RPC）。TaskRegistry: 任务注册/调度/回收。Swarm 群体智能: 多个 Agent 协作完成复杂任务。Peer Groups: Agent 分组协作。子代理并行执行 + 隔离。SpawnSubagentTool: 通过工具调用创建子 Agent。自动发现 + 负载均衡。

**依赖库**: axum（HTTP）、tokio（异步）、parking_lot（并发控制）、serde_json（序列化）。

**原理**: SpawnSubagentTool 实现 Tool trait，LLM 通过工具调用创建子 Agent。子 Agent 可以是 Local（进程内）、ACP（子进程）或 A2A（远程）。Swarm 模式: 多个 Agent 同时处理不同子任务，结果汇总。Peer Groups: 按能力或角色分组，任务路由到合适的 Agent 组。TaskRegistry 管理任务生命周期: 创建 -> 调度 -> 执行 -> 回收。

**优势**: 完整的多 Agent 平台（Swarm/Peer Groups/并行/隔离）；SpawnSubagentTool 通过工具调用创建子 Agent（对 LLM 透明）；自动发现 + 负载均衡；TaskRegistry 完整任务生命周期管理；ACP + A2A + RPC 多协议支持。

**劣势**: 系统复杂度高；Swarm 协调逻辑复杂；负载均衡策略可能不够智能；子 Agent 隔离增加资源消耗。

---

## 7. 日志与可观测性

### Shadow

**实现方式**: record! 宏统一日志入口。JSONL（JSON Lines）格式持久化到文件。broadcast channel 多消费者广播: 日志写入文件的同时，可以推送到 TUI/Observer。Observer trait: on_event(ObserverEvent) 接收事件、on_metric(name, value) 接收指标。LogObserver 桥接: 将 LogEvent 投影到 ObserverEvent，统一日志和观察者通道。LogEvent schema 版本管理。日志读取 API: 支持按时间/级别/类型过滤查询。

**依赖库**: serde_json（JSONL 序列化）、parking_lot::RwLock（Observer 注册）、tokio::sync::broadcast（多消费者广播）、OnceLock（全局 Observer 单例）。

**原理**: record! 宏在调用点生成 LogEvent，包含时间戳、级别、事件类型、消息、上下文等字段。LogEvent 通过 broadcast channel 发送给所有订阅者。JSONL writer 将事件逐行写入文件。ObserverBridge 维护一个全局 LogObserver 单例（OnceLock + RwLock），forward() 函数将 LogEvent 投影为 ObserverEvent 并转发。无 Observer 绑定时 no-op（零开销）。

**优势**: record! 宏统一入口，调用简洁；JSONL 格式便于后续处理（每行独立 JSON）；broadcast channel 支持多消费者（文件 + TUI + Observer 同时消费）；Observer trait 抽象事件和指标；LogObserver 桥接统一日志和观察者通道；无 Observer 时零开销。

**劣势**: 无成本追踪（token 费用记录）；无工具 I/O 捕获（工具输入/输出内容记录）；无归因传播 spawn（子 Agent 的日志不会自动关联到父 Agent）；无指标聚合（如 QPS、延迟统计）；无部署事件记录；事件类型较少（相比 ZeroClaw 的 20+ 事件）。

### Hermes

**实现方式**: JSONL 持久化日志。标准 console.log/winston 日志。无 Observer 模式。无宏统一入口。

**依赖库**: winston（日志库）或 console（Node.js 内置）。

**原理**: 通过 winston 或 console.log 输出日志，同时写入 JSONL 文件。无统一的事件系统，各组件自行记录日志。

**优势**: 实现简单；winston 生态成熟，支持多种 transport（文件/控制台/远程）；JSONL 格式便于处理。

**劣势**: 无统一日志入口（record! 宏）；无 broadcast channel（无法多消费者同时消费）；无 Observer trait（无事件抽象）；无 LogObserver 桥接；无日志读取 API；无 schema 版本管理；无成本追踪；无工具 I/O 捕获；无归因传播。

### Claude Code

**实现方式**: 无独立日志系统。依赖 Anthropic SDK 的日志输出。对话历史以 JSON 格式存储在本地。

**依赖库**: 无专门日志库。

**原理**: Anthropic SDK 内部记录请求/响应日志。对话历史存储为 JSON 文件。无结构化日志系统。

**优势**: 实现最简；无额外日志开销。

**劣势**: 无结构化日志；无 JSONL 持久化；无 broadcast channel；无 Observer 模式；无日志读取 API；无 schema 管理；无成本追踪；无工具 I/O 捕获；无归因传播；无指标系统。几乎无可观测性。

### ZeroClaw

**实现方式**: record! 宏统一日志入口。LogEvent schema-2（第二代 schema，支持更多字段）。JSONL 滚动持久化（按大小/时间滚动）。broadcast channel 多消费者广播。ObserverBridge 桥接。20+ 事件类型覆盖完整生命周期。成本追踪: 记录每次 LLM 调用的 token 用量和费用。工具 I/O 捕获: 记录工具调用的输入/输出内容，含泄漏扫描（检测敏感信息泄露）。归因传播 spawn: 子 Agent 的日志自动关联到父 Agent。指标系统: QPS、延迟、成功率等。

**依赖库**: serde_json（JSONL）、parking_lot（并发控制）、tokio::sync::broadcast（广播）、ring（敏感信息哈希）。

**原理**: LogEvent schema-2 在 schema-1 基础上增加: cost 字段（token 费用）、tool_input/tool_output 字段（工具 I/O）、attribution_chain 字段（归因链）、spawn_parent 字段（父 Agent 标识）。JSONL 滚动: 按文件大小（如 100MB）或时间（如每天）滚动，自动压缩旧文件。工具 I/O 捕获: 在工具执行前后记录输入/输出，通过正则扫描检测 API key、密码等敏感信息泄露。归因传播 spawn: 子 Agent 创建时继承父 Agent 的 attribution chain，所有子 Agent 的日志都关联到父 Agent。

**优势**: 20+ 事件类型覆盖最完整；成本追踪支持精确费用分析；工具 I/O 捕获 + 泄漏扫描确保安全合规；归因传播 spawn 实现完整审计链；JSONL 滚动防止日志文件过大；指标系统支持性能监控；schema-2 字段最丰富。

**劣势**: 20+ 事件类型和多个字段导致 LogEvent 结构复杂；工具 I/O 捕获增加 I/O 开销；泄漏扫描可能误报；归因传播链可能过长影响性能；滚动日志管理需要额外配置。

---

## 8. 配置系统

### Shadow

**实现方式**: TOML 格式配置文件（~/.shadow/config.toml）。dotted path config set: `shadow config set providers.openai.api_key xxx` 支持嵌套路径设置。ChaCha20-Poly1305 密钥加密: API key 等敏感信息加密存储。Provider 别名: 为 provider 起易记的名称（如 alias "fast" -> gpt-4o-mini）。配置迁移: 支持 schema 版本升级时自动迁移。环境变量覆盖: 环境变量优先级高于配置文件。

**依赖库**: toml（TOML 解析）、serde（序列化/反序列化）、ring/chacha20poly1305（加密）、anyhow（错误处理）。

**原理**: 配置文件以 TOML 格式存储，通过 serde 反序列化为 Config struct。config set 命令接受 dotted path（如 "providers.openai.api_key"），解析路径后修改对应字段并序列化回 TOML。敏感字段（如 api_key）使用 ChaCha20-Poly1305 加密: 主密钥从机器特征（如 MAC 地址）或用户密码派生。Provider 别名维护 alias -> provider_type + model 的映射。配置迁移检测 schema 版本，按版本号逐步迁移。

**优势**: TOML 格式人类可读且支持注释；dotted path config set 支持精确修改嵌套字段；ChaCha20-Poly1305 加密保护敏感信息；Provider 别名简化使用；配置迁移支持版本升级；环境变量覆盖灵活。

**劣势**: 无 Configurable derive 宏（需手写 serde 属性）；无脏路径追踪（dirty_paths）；无安全降级模式（degraded_security）；无 1Password 集成；无 JSON Schema 导出；无配置分区（ConfigTab）；无预设模板；无 SecurityPolicy 配置。

### Hermes

**实现方式**: YAML 格式配置文件（~/.hermes/config.yaml）。环境变量覆盖。config set 命令。

**依赖库**: js-yaml（YAML 解析）、zod（配置校验）。

**原理**: YAML 格式存储配置，通过 js-yaml 解析为 JavaScript 对象。zod 定义配置 schema 并进行运行时校验。环境变量以 HERMES_ 前缀覆盖配置项。

**优势**: YAML 格式支持复杂嵌套和注释；zod 运行时校验配置正确性；开发简单。

**劣势**: 无密钥加密（API key 明文存储）；无 Configurable 宏；无配置迁移；无 Provider 别名；无脏路径追踪；无安全降级；无 1Password；无 JSON Schema 导出；无配置分区；无预设模板。

### Claude Code

**实现方式**: JSON 格式配置文件（~/.claude/config.json）。CLAUDE.md 文件作为项目级配置。环境变量覆盖。

**依赖库**: Node.js 内置 JSON 解析。

**原理**: JSON 存储全局配置，CLAUDE.md 存储项目级配置（coding conventions、项目结构等）。配置项简单，主要存储 API key 和模型选择。

**优势**: JSON 格式通用；CLAUDE.md 项目级配置与代码版本控制集成；实现简单。

**劣势**: JSON 不支持注释；无密钥加密；无 config set 命令（需手动编辑文件）；无配置迁移；无 Provider 别名；无 Configurable 宏；无脏路径追踪；无安全降级；无 1Password；无 JSON Schema 导出；无配置分区；无预设模板；无 SecurityPolicy。

### ZeroClaw

**实现方式**: TOML + toml_edit 格式配置文件（~/.zeroclaw/config.toml）。Configurable derive 宏（14 个属性）自动生成配置代码: 字段可见性、加密标记、默认值、环境变量映射、校验规则等。ChaCha20 密钥加密。V1 -> V3 配置迁移（3 个版本）。dirty_paths: 追踪哪些配置路径被修改（未保存的更改）。degraded_security: 安全降级模式（当安全模块不可用时降级运行）。1Password 集成: 从 1Password vault 读取密钥。21 种 ConfigTab: 配置分区管理（providers/agents/channels/memory/security/tools/skills 等）。JSON Schema 导出: 通过 schemars 自动导出配置的 JSON Schema。SecurityPolicy: 安全策略配置（命令黑名单、路径白名单等）。预设模板: 预定义配置模板（如 "minimal"、"full"、"development"）。

**依赖库**: toml_edit（保留注释的 TOML 编辑）、serde、ring/chacha20poly1305（加密）、schemars（JSON Schema 导出）、proc-macro2/syn/quote（Configurable 宏）。

**原理**: Configurable 宏在编译时解析 #[configurable(...)] 属性，自动生成: serde Serialize/Deserialize 实现、默认值函数、环境变量覆盖逻辑、字段加密/解密逻辑、配置校验代码。toml_edit 保留 TOML 文件中的注释和格式: 修改配置后写回文件不会丢失注释。dirty_paths 维护一个 HashSet<String>，记录被修改但未保存的配置路径。degraded_security 在安全模块（如 ring）不可用时，降级为不加密模式并记录警告。1Password 集成通过 1Password CLI（op）读取 vault 中的密钥。21 种 ConfigTab 将配置按功能分区，每个 Tab 独立编辑和校验。

**优势**: Configurable 宏大幅减少样板代码（14 个属性自动处理）；toml_edit 保留注释（其他系统修改配置会丢失注释）；V1->V3 迁移支持平滑升级；dirty_paths 追踪未保存更改；degraded_security 优雅降级；1Password 集成企业级密钥管理；21 种 ConfigTab 分区管理大型配置；JSON Schema 导出支持 IDE 自动补全；SecurityPolicy 集中安全策略；预设模板快速初始化。

**劣势**: Configurable 宏增加编译时间；14 个属性学习曲线陡峭；21 种 ConfigTab 可能过度分区；toml_edit 比 toml 更慢；1Password 集成依赖外部 CLI；配置系统复杂度高。

---

## 9. 安全机制

### Shadow

**实现方式**: AutonomyLevel 枚举: ReadOnly（只读，不执行任何修改操作）、Supervised（监督，敏感操作需审批）、Auto（自动，无需审批）。Shell 黑名单: 15 个危险命令模式（rm -rf /、mkfs、dd if=、fork bomb、curl|sh 等），支持正则和子串匹配。PathGuardedTool 装饰器: 限制文件访问路径在 workspace 目录内。RateLimitedTool 装饰器: 令牌桶限流，防止工具被高频调用。jsonschema 参数校验: 工具执行前校验参数合法性。Sandbox trait: OS 级进程隔离抽象（当前只有 NoopSandbox 直通实现，预留 FirejailSandbox/NamespaceSandbox）。requires_approval(): 敏感工具（ShellTool/FileWriteTool）覆盖为 true，Supervised 模式下执行前请求用户确认。环境变量白名单: 只传递安全的环境变量给子进程（过滤 API_KEY/TOKEN/SECRET 等）。

**依赖库**: jsonschema（参数校验）、regex（黑名单模式匹配）、ring/chacha20poly1305（密钥加密）、tokio::time::timeout（超时控制）。

**原理**: AutonomyLevel 在 AgentConfig 中设置，Agent 在工具执行前检查: ReadOnly 模式下拒绝所有修改操作（FileWriteTool/ShellTool 等）；Supervised 模式下调用 requires_approval() 返回 true 的工具时暂停等待用户确认；Auto 模式下直接执行。ShellTool 在 execute() 中: (1) is_blocked() 检查命令是否命中黑名单；(2) 设置工作目录（如果 policy.workspace 有值）；(3) filter_env() 过滤环境变量。PathGuardedTool 在 execute() 前检查 args 中的文件路径是否在允许范围内。RateLimitedTool 使用令牌桶算法: 桶容量和填充速率可配置，超过速率时拒绝或等待。Sandbox trait 的 wrap_command() 在命令执行前注入沙箱参数。

**优势**: AutonomyLevel 三级自治灵活控制；Shell 黑名单拦截危险命令（四者中独有，与 ZeroClaw 共有）；jsonschema 参数校验在执行前拦截非法参数（四者中独有）；PathGuardedTool + RateLimitedTool 装饰器模式实现安全策略与工具逻辑解耦（四者中独有设计）；环境变量白名单防止敏感信息泄露；Sandbox trait 预留扩展点。

**劣势**: Sandbox 仅有 NoopSandbox（无实际沙箱隔离）；无 WASM 沙箱；无 Ed25519 签名验证；无 WebAuthn；无幂等存储；无 TLS 硬化；无配对认证（PairingGuard）；无速率限制滑动窗口（仅有令牌桶）。

### Hermes

**实现方式**: 基本的工具审批机制。无 Shell 黑名单。无参数校验。无路径守卫。无限流装饰器。无 Sandbox。

**依赖库**: 无专门安全库。

**原理**: Agent 在执行敏感工具前请求用户确认。无其他安全机制。

**优势**: 实现简单；工具审批基本满足需求。

**劣势**: 无 Shell 黑名单（危险命令可执行）；无参数校验；无路径守卫（可访问任意文件）；无限流；无 Sandbox（无进程隔离）；无密钥加密（API key 明文）；无环境变量过滤；无签名验证；无 WebAuthn；无幂等存储；无 TLS 硬化。安全性是四者中最低的。

### Claude Code

**实现方式**: 工具审批机制。利用 Anthropic API 的安全机制（如 content filtering）。Bash 工具有超时控制。

**依赖库**: @anthropic-ai/sdk（内置安全机制）。

**原理**: Agent 在执行 Bash/Edit 等敏感工具前请求用户确认。依赖 Anthropic API 的内容过滤机制过滤有害输出。

**优势**: 利用 Anthropic API 安全机制；工具审批基本满足需求；实现简单。

**劣势**: 无 Shell 黑名单；无参数校验；无路径守卫；无限流；无 Sandbox；无密钥加密；无环境变量过滤；无签名验证；无 WebAuthn；无幂等存储；无 TLS 硬化。安全性低。

### ZeroClaw

**实现方式**: AutonomyLevel 三级自治。Shell 黑名单 + 正则匹配。PathGuardedTool 路径守卫。RateLimitedTool 滑动窗口限流（比令牌桶更精确）。WASM 沙箱: wasmtime fuel 限制（计算资源限制）+ 三级安全模式（Trusted/Sandboxed/Untrusted）。Ed25519 签名验证: 插件必须签名才能加载。WebAuthn: 基于 WebAuthn 的身份认证。幂等存储: 幂等 API 支持。TLS 硬化: rustls 替代 OpenSSL。配对认证: PairingGuard 设备配对。SecurityPolicy: 集中安全策略配置。

**依赖库**: wasmtime（WASM 沙箱 + fuel 限制）、ring（Ed25519 签名验证 + TLS）、rustls（TLS 硬化）、ring/chacha20poly1305（密钥加密）。

**原理**: WASM 沙箱: 每个插件在独立的 wasmtime 实例中运行，fuel 限制计算量（防止无限循环），三级安全模式: Trusted（完全信任，无限制）、Sandboxed（WASM 沙箱隔离，fuel 限制）、Untrusted（WASM 沙箱 + 网络隔离 + 文件系统隔离）。Ed25519 签名: 插件作者用私钥签名，运行时用公钥验证。WebAuthn: 使用硬件认证器（如 YubiKey）进行身份认证。幂等存储: 每个请求分配唯一 ID，重复请求返回缓存结果。TLS 硬化: rustls 使用 Rust 实现的 TLS 栈，避免 OpenSSL 的内存安全漏洞。PairingGuard: 设备配对认证，防止未授权设备连接。

**优势**: WASM 沙箱 + fuel 限制实现计算资源隔离（四者中独有）；三级安全模式灵活控制信任级别；Ed25519 签名验证确保插件来源可信；WebAuthn 硬件认证支持企业级安全；幂等存储防止重复操作；rustls TLS 硬化避免内存安全漏洞；滑动窗口限流比令牌桶更精确；PairingGuard 设备配对认证。

**劣势**: WASM 沙箱增加运行时开销；三级安全模式配置复杂；Ed25519 签名需要密钥管理基础设施；WebAuthn 需要硬件认证器；rustls 可能不支持某些 TLS 扩展；安全功能过多增加系统复杂度。

---

## 10. TUI 终端界面

### Shadow

**实现方式**: ratatui 构建 TUI 界面。暗色/亮色主题切换。命令面板（command palette）: 快捷键打开，支持模糊搜索命令。状态栏: 显示当前 Agent 状态、模型、token 用量等。消息列表 + 输入框: 聊天界面布局。流式显示: StreamDeltaCallback 实时推送回答和思考增量。

**依赖库**: ratatui（TUI 框架）、crossterm（终端控制）。

**原理**: ratatui 使用 Buffer + Widget 渲染模型: 每个 Widget 将内容写入 Buffer，最终一次性输出到终端。命令面板维护命令列表，按模糊匹配算法排序结果。StreamDeltaCallback 通过闭包接收增量，更新消息列表。状态栏通过 Agent 的共享状态更新。

**优势**: ratatui 性能好（一次性渲染减少闪烁）；暗色/亮色主题适配不同环境；命令面板提升操作效率；流式显示支持实时反馈；状态栏信息丰富。

**劣势**: 无 Markdown 渲染（代码块、表格等不渲染）；无 i18n 多语言；无流式思考显示（reasoning 内容不单独显示）；无审批覆盖层（审批通过命令行交互）；无草稿模式；无 viewport slice 缓存（大消息列表可能卡顿）；无 Tauri 桌面应用。

### Hermes

**实现方式**: TUI 界面（可能使用 blessed 或 ink）。聊天界面。配置界面。状态栏。Markdown 渲染。

**依赖库**: blessed/ink（TUI 框架）、marked/markdown-it（Markdown 渲染）。

**原理**: 使用 React-like 的组件模型构建 TUI。Markdown 渲染将 LLM 输出的 Markdown 转换为终端彩色文本。

**优势**: Markdown 渲染支持代码块、表格等；配置界面方便设置；React-like 组件模型开发效率高。

**劣势**: 无命令面板；无流式思考显示；无审批覆盖层；无草稿模式；无 i18n 多语言；无 viewport 缓存；无 Tauri 桌面应用；无暗色/亮色主题切换。

### Claude Code

**实现方式**: 纯 CLI（无 TUI）。通过 ANSI 转义序列输出彩色文本。工具调用结果以格式化文本显示。代码 diff 以语法高亮显示。

**依赖库**: chalk（彩色输出）或 ANSI 转义序列。

**原理**: 直接输出到 stdout，使用 ANSI 转义序列控制颜色和格式。无交互式 UI 组件。

**优势**: 实现最简单；兼容性最好（任何终端都能运行）；启动最快；管道友好（输出可重定向）。

**劣势**: 无 TUI 界面（无交互式组件）；无命令面板；无配置界面；无记忆界面；无状态栏；无滚动消息区；无主题切换；无 Markdown 渲染；无流式思考显示；无审批覆盖层；无草稿模式；无 i18n；无 Tauri 桌面应用。用户体验最差。

### ZeroClaw

**实现方式**: ratatui 构建 TUI 界面（zerocode crate）。Markdown 渲染（代码块、表格、列表等）。i18n 多语言: t() 宏实现国际化。流式思考显示: reasoning content 独立区域实时显示。审批覆盖层: 审批请求弹出覆盖层，支持快捷键确认/拒绝。草稿模式: 草稿更新和编辑。viewport slice 缓存: 大消息列表虚拟滚动优化。暗色/亮色主题。Tauri 桌面应用: 跨平台桌面 GUI。onboarding 配置向导。

**依赖库**: ratatui（TUI）、crossterm（终端控制）、tauri（桌面应用）、fluent/rust-i18n（国际化）、tui-markdown（Markdown 渲染）。

**原理**: viewport slice 缓存: 只渲染可见区域的消息，通过 slice 偏移量定位。i18n t() 宏在编译时提取字符串，运行时根据语言设置加载翻译。审批覆盖层: 当 Agent 请求审批时，TUI 暂停当前渲染，弹出审批覆盖层（ratatui 的 Layer 组件）。Tauri 桌面应用: 使用 Tauri 框架将 TUI 包装为桌面 GUI，支持系统托盘、文件对话框等。流式思考显示: 在消息区域上方增加 reasoning 区域，实时显示思考增量。

**优势**: Markdown 渲染支持丰富的格式；i18n 多语言支持国际化（四者中独有）；流式思考显示独立区域；审批覆盖层交互友好；草稿模式支持编辑；viewport slice 缓存支持大消息列表；Tauri 桌面应用跨平台；onboarding 向导简化首次配置。

**劣势**: 功能复杂导致代码量大；Tauri 桌面应用增加构建依赖；i18n 翻译文件维护成本高；viewport 缓存逻辑复杂；ratatui 渲染大量内容时可能卡顿。

---

## 11. Skills 技能系统

### Shadow

**实现方式**: SKILL.md 文件解析: 技能以 Markdown 文件定义，包含名称、描述、触发条件、执行步骤等。skill_tool: 将技能包装为工具供 LLM 调用。skill_manage: 技能管理工具（创建/更新/删除/列表）。review.rs: 对话后技能审查，分析 Agent 的工具使用模式，建议新技能或改进现有技能。improver.rs: 技能自进化改进，根据使用反馈自动优化技能描述和步骤。skill_http.rs: HTTP 技能，通过 HTTP API 加载远程技能。

**依赖库**: serde（SKILL.md front matter 解析）、reqwest（HTTP 技能加载）、async-trait。

**原理**: SKILL.md 文件使用 YAML front matter + Markdown body 格式。front matter 包含元数据（name、description、triggers、tools），body 包含详细步骤。skill_tool 在 Agent 初始化时加载所有 SKILL.md，注册为可调用工具。review.rs 在对话后（如果 skill_review_enabled）分析工具调用序列，检测重复操作模式（可提取为技能）、低效操作序列（可优化为技能步骤）、缺失技能（用户频繁请求但无对应技能）。improver.rs 根据审查结果自动修改 SKILL.md 文件。

**优势**: 技能自进化反馈（review + improver）是 Shadow 的特色设计；HTTP 技能支持远程技能加载；对话后审查自动发现改进机会；与 Agent 循环深度集成。

**劣势**: 无技能搜索（无法按关键词搜索已有技能）；无技能分类（无类别管理）；无技能脚本（技能仅描述性，不可编程执行）；无技能模板（无预设模板）；无技能包（无法打包分发）；无 MCP 服务器（仅设计阶段）。

### Hermes

**实现方式**: 技能系统包含 SKILL.md 解析、技能加载、技能管理、技能搜索（按关键词搜索）、技能分类（类别管理）、技能脚本（可编程执行）、技能模板（预设模板）。自进化反馈: 根据使用反馈改进技能。MCP 服务器: 通过 MCP 加载外部技能。

**依赖库**: Node.js 生态，无专门技能库。

**原理**: 技能以文件/目录形式存储，支持 SKILL.md 格式和可编程脚本格式。技能搜索通过关键词匹配。技能分类通过目录组织。MCP 服务器通过 Model Context Protocol 加载外部技能提供者。自进化反馈记录技能使用情况，根据成功率和用户反馈调整技能优先级。

**优势**: 技能搜索和分类管理完善；技能脚本支持可编程执行；技能模板预设快速创建；MCP 服务器支持外部技能；自进化反馈成熟。

**劣势**: 无对话后技能审查（不自动发现改进机会）；无技能改进器（不自动修改技能文件）；无 HTTP 技能加载；无技能审查触发阈值。

### Claude Code

**实现方式**: SKILL.md 解析 + 技能加载 + 技能搜索。技能以文件形式存储，Agent 初始化时加载。

**依赖库**: 无专门技能库。

**原理**: SKILL.md 格式定义技能，Agent 在初始化时扫描技能目录并加载。技能搜索通过关键词匹配。

**优势**: 实现简单；与代码生成场景集成；技能搜索基本满足需求。

**劣势**: 无技能管理（创建/更新/删除）；无技能审查；无技能改进；无技能分类；无技能脚本；无技能模板；无技能包；无 MCP 服务器；无 HTTP 技能；无自进化反馈。

### ZeroClaw

**实现方式**: skillforge: 完整的技能锻造系统，支持技能的创建、改进、分发全生命周期。skill_bundles: 技能包，将多个技能打包分发。MCP 服务器: 通过 MCP 加载外部技能。技能搜索 + 分类 + 脚本 + 模板。

**依赖库**: serde、reqwest（远程技能加载）、wasmtime（WASM 技能脚本）。

**原理**: skillforge 是技能锻造工厂: 从 Agent 使用模式中自动发现新技能到生成 SKILL.md 到测试验证到分发。skill_bundles 将多个技能和依赖打包为可分发单元（类似 npm 包）。MCP 服务器通过 Model Context Protocol 标准化外部技能接口。WASM 技能脚本: 技能的可编程部分以 WASM 模块执行，在沙箱中安全运行。

**优势**: skillforge 完整的技能锻造生命周期（四者中独有）；skill_bundles 支持打包分发（四者中独有）；MCP 服务器标准化外部技能；WASM 技能脚本安全执行；技能搜索 + 分类 + 脚本 + 模板功能完整。

**劣势**: skillforge 系统复杂；skill_bundles 依赖管理复杂；WASM 技能脚本需要 WIT 接口定义；学习曲线陡峭。

---

## 12. Channel 渠道集成

### Shadow

**实现方式**: Channel trait 定义 name()、send()、supports_approval() 方法。当前仅有 CliChannel 实现（println 输出）。CliChannel 继承 Attributable，Role::Channel。

**依赖库**: 无外部渠道库（CliChannel 仅使用 println）。

**原理**: Channel trait 抽象消息平台，每个平台实现 send() 方法发送消息。supports_approval() 标记渠道是否支持审批请求（如 Telegram 支持内联按钮审批）。CliChannel 是最简实现，直接 println 输出消息。

**优势**: Channel trait 定义清晰，易于扩展；Attributable 归因支持渠道追踪；设计精简。

**劣势**: 仅 1 种渠道（CLI），远少于 Hermes 的 30+ 种和 ZeroClaw 的 37 种；无 Telegram/Discord/Slack/飞书/钉钉等渠道；无 webhook；无 WebSocket；无语音渠道；无 ACP；无消息平台协议适配。

### Hermes

**实现方式**: 30+ 种渠道集成。Telegram、Discord、Slack、飞书/Lark、Matrix、Email（himalaya）、iMessage、Twitter/X（xurl）、Notion、Webhook、Filesystem、ACP 等。Browser 渠道: 基于 Puppeteer/Playwright 的浏览器自动化渠道。

**依赖库**: telegraf（Telegram）、discord.js（Discord）、slack-sdk（Slack）、notion-client（Notion）、puppeteer/playwright（浏览器自动化）。

**原理**: 每个渠道实现统一的接口（send/receive）。Agent 在收到渠道消息后，将消息传入 Agent 循环处理，响应通过对应渠道发回。Browser 渠道通过 Puppeteer/Playwright 控制浏览器，实现网页操作和表单填写。

**优势**: 30+ 种渠道覆盖最主流的消息平台；Browser 渠道支持浏览器自动化（四者中独有，与 ZeroClaw 共有）；利用各平台官方 SDK 减少适配工作；渠道集成成熟。

**劣势**: 无钉钉/企业微信/QQ/Signal/IRC/Line/Bluesky/Reddit/Nostr/AMQP 等渠道（ZeroClaw 有）；无 WebSocket 渠道；无语音通话/语音唤醒渠道；无企业级渠道（如企业微信 WS）。

### Claude Code

**实现方式**: 仅 CLI 渠道。通过命令行交互，无其他消息平台集成。ACP 支持有限的子进程交互。

**依赖库**: 无渠道库。

**原理**: 所有交互通过 stdin/stdout 完成。无消息平台抽象。

**优势**: 实现最简单；专注代码生成场景，不需要多渠道。

**劣势**: 仅 CLI 渠道；无任何消息平台集成；无 Telegram/Discord/Slack 等；无 Browser 渠道；无 Webhook；无 WebSocket；无语音渠道；无 ACP。渠道集成是四者中最少的。

### ZeroClaw

**实现方式**: 37 种渠道。orchestrator（23846 行）统一管理所有渠道。media_pipeline: 多媒体消息处理管道（图片/音频/视频转码）。LRU 对话历史: 每个渠道维护 LRU 缓存的对话历史，避免全量加载。指数退避重连: 渠道断连后自动指数退避重连。涵盖 CLI/Telegram/Discord（含 slash 命令）/Slack/飞书/钉钉/企业微信（WeCom+WS）/微信/QQ/Matrix/Email（IMAP+SMTP）/Gmail（GmailPush）/iMessage/Signal/IRC/Line/Bluesky/Twitter-X/Reddit/Nostr/Notion/Webhook/AMQP/Filesystem/VoiceCall/VoiceWake/WebSocket/ACP 等。

**依赖库**: telegraf、serenity（Discord）、slack-sdk、matrix-sdk、imap/smtp、reqwest（HTTP 渠道）、tokio-tungstenite（WebSocket）。

**原理**: orchestrator 是渠道管理中枢: 维护所有渠道实例，负责消息路由、状态管理、重连逻辑。每个渠道实现 Channel trait，在收到消息后通过 orchestrator 路由到 Agent。media_pipeline 处理多媒体消息: 图片压缩/格式转换、音频转写、视频提取帧。LRU 对话历史按 channel_id + user_id 维护，超出容量时淘汰最旧的历史。指数退避重连: 断连后等待 1s/2s/4s/8s/16s 递增重连，最大间隔 300s。

**优势**: 37 种渠道覆盖最全面（四者中最多）；orchestrator 统一管理所有渠道；media_pipeline 处理多媒体消息；LRU 对话历史优化内存；指数退避重连提高可靠性；企业级渠道（企业微信 WS/钉钉）支持中国市场需求；语音通话和语音唤醒渠道支持语音交互。

**劣势**: orchestrator 23846 行代码过于复杂；37 种渠道维护成本极高；部分渠道可能缺乏充分测试；media_pipeline 增加依赖；LRU 缓存可能导致上下文丢失。

---

## 13. 插件与扩展系统

### Shadow

**实现方式**: 当前仅有设计阶段。规划中: WASM 插件系统（基于 wasmtime），允许第三方以 WASM 模块扩展工具和渠道。MCP（Model Context Protocol）客户端支持（设计阶段）。

**依赖库**: 设计阶段使用 wasmtime（WASM 运行时）。

**原理**: 设计中的 WASM 插件: 第三方开发者用 Rust/C/Go 等语言编写工具，编译为 WASM 模块。Shadow 在运行时通过 wasmtime 加载 WASM 模块，实现 Tool trait 的 WasmTool 适配器。WIT（Wasm Interface Type）定义插件接口。MCP 客户端通过 Model Context Protocol 连接外部 MCP 服务器，将其工具暴露为 Shadow 工具。

**优势**: WASM 插件设计安全（沙箱隔离）；MCP 支持标准化外部工具接入；与 ZeroClaw 设计一致，可复用经验。

**劣势**: 仅有设计，无实现；当前不可扩展第三方工具；无 MCP 支持；无插件签名验证；无插件市场。

### Hermes

**实现方式**: 插件式架构。插件以 npm 包形式分发，通过 require/import 动态加载。插件注册: register() 方法注册工具/渠道/provider。MCP 原生支持: 通过 MCP 协议连接外部工具提供者。

**依赖库**: Node.js require/import（动态加载）、MCP SDK。

**原理**: 插件是实现了特定接口的 JavaScript/TypeScript 模块。Agent 在初始化时扫描插件目录，require 每个插件并调用 register() 方法注册组件。MCP 客户端连接 MCP 服务器，将其工具列表同步到 Agent 的工具注册表。

**优势**: 插件开发简单（任何 npm 包都是潜在插件）；MCP 原生支持；插件生态可利用 npm 分发。

**劣势**: 无沙箱隔离（插件直接运行在 Agent 进程中，可访问所有资源）；无签名验证（恶意插件可注入）；无资源限制（插件可消耗无限 CPU/内存）；无 WASM 支持；插件质量参差不齐。

### Claude Code

**实现方式**: 不支持插件系统。工具集固定，不可扩展。

**依赖库**: 无。

**原理**: 所有工具硬编码在 Agent 中，无插件接口。

**优势**: 实现简单；无插件兼容性问题；安全（不加载第三方代码）。

**劣势**: 完全不可扩展；无法添加自定义工具；无 MCP 支持；无插件生态；无 WASM 支持。扩展性是四者中最差的。

### ZeroClaw

**实现方式**: 完整的 WASM 插件系统。wasmtime 43 个组件: WASM 运行时支持 43 个组件接口。WIT component model: 使用 Wasm Interface Type 定义插件接口。Ed25519 签名: 插件必须签名才能加载。fuel 限制: 每个 WASM 插件有计算资源限制。三级安全模式: Trusted（完全信任）/Sandboxed（WASM 沙箱）/Untrusted（WASM 沙箱 + 网络隔离 + 文件系统隔离）。WasmTool/WasmChannel/WasmMemory: WASM 插件可以实现 Tool、Channel、Memory 三种 trait。MCP 服务器: 通过 MCP 加载外部技能。

**依赖库**: wasmtime（WASM 运行时 + 43 组件）、ring（Ed25519 签名验证）、wit-bindgen（WIT 接口绑定）。

**原理**: WASM 插件以 WebAssembly 组件模型运行: (1) 插件用 Rust/C/Go 等语言编写，编译为 WASM 组件；(2) WIT 文件定义插件接口（工具方法、渠道方法、记忆方法）；(3) wit-bindgen 自动生成 Rust 绑定代码；(4) wasmtime 加载 WASM 组件，在沙箱中执行；(5) fuel 限制计算量（防止无限循环）；(6) Ed25519 验证插件签名，确保来源可信。三级安全模式: Trusted 插件无限制运行；Sandboxed 插件在 WASM 沙箱中运行（fuel 限制 + 内存限制）；Untrusted 插件在 WASM 沙箱 + 网络隔离 + 文件系统隔离中运行。

**优势**: WASM 插件安全隔离（沙箱 + fuel + 三级安全模式，四者中最完善）；WIT component model 标准化接口定义；Ed25519 签名确保插件来源可信；WasmTool/WasmChannel/WasmMemory 三种插件类型覆盖所有核心组件；43 个组件接口功能丰富；MCP 服务器支持外部技能。

**劣势**: wasmtime 增加二进制大小（约 10-15MB）；WIT 接口定义学习曲线陡峭；Ed25519 签名需要密钥管理基础设施；三级安全模式配置复杂；WASM 插件性能不如原生代码（约 10-20% 开销）；43 个组件接口维护成本高。

---


## 14. Gateway 与基础设施

### Shadow

**实现方式**: Cron 定时任务: SQLite 持久化，支持定时触发 Agent 任务。Session 管理: SessionStore 存储会话状态。Workspace 抽象: 标准化路径布局。双模式构建: kernel-only 模式。交叉编译: ARM 目标平台支持。Gateway HTTP: 设计阶段（axum）。

**依赖库**: rusqlite（Cron 任务持久化）、tokio（定时器）、axum（Gateway HTTP，设计阶段）。

**原理**: Cron 任务以 SQLite 表存储（任务名、cron 表达式、Agent 配置、上次执行时间），tokio 定时器检查到期任务并触发 Agent。SessionStore 维护会话列表和当前会话状态。Workspace 抽象定义标准目录布局: config 目录、data 目录、log 目录、skills 目录等。双模式构建通过 Cargo features 控制: kernel-only 模式排除 providers/memory/runtime 等重依赖，仅保留 core trait 定义。交叉编译通过配置 target triple 实现 ARM 平台编译。

**优势**: Cron 定时任务 SQLite 持久化（重启不丢失）；双模式构建适配嵌入式（kernel-only）；交叉编译 ARM 支持（四者中独有）；Workspace 抽象标准化路径布局；设计精简。

**劣势**: Gateway HTTP 仅有设计无实现；无评估框架；无硬件抽象（Peripheral trait）；无机器人控制；无 I2C/SPI/GPIO；无 i18n 多语言；无 Tauri 桌面应用。

### Hermes

**实现方式**: Cron 定时任务（cronjob 模块）。Session 管理。Workspace 抽象。MCP 客户端: 原生 MCP 支持。无 Gateway HTTP、无交叉编译、无双模式构建、无硬件支持。

**依赖库**: node-cron（定时任务）、better-sqlite3（Session 存储）。

**原理**: node-cron 在 Node.js 进程中运行定时器，触发 Agent 任务。Session 管理通过 SQLite 存储会话状态。MCP 客户端通过 MCP SDK 连接外部 MCP 服务器。

**优势**: MCP 客户端原生支持（四者中独有，与 ZeroClaw 共有）；Cron 定时任务；Session 管理完善；Workspace 抽象。

**劣势**: 无 Gateway HTTP；无交叉编译；无双模式构建；无硬件支持；无评估框架；无 i18n；无 Tauri 桌面应用；无机器人控制。

### Claude Code

**实现方式**: Session 管理。Workspace 抽象。ACP 支持（有限）。无 Cron、无 Gateway、无交叉编译、无双模式构建、无硬件支持、无 MCP。

**依赖库**: Node.js 内置模块。

**原理**: Session 管理通过本地文件存储会话状态。Workspace 抽象定义项目目录结构。ACP 通过子进程协议与外部工具交互。

**优势**: 实现简单；专注代码生成场景。

**劣势**: 无 Cron 定时任务；无 Gateway HTTP；无交叉编译；无双模式构建；无硬件支持；无评估框架；无 i18n；无 Tauri 桌面应用；无机器人控制；无 MCP 支持。基础设施是四者中最少的。

### ZeroClaw

**实现方式**: Gateway HTTP: axum 实现 50+ 路由，支持 REST/WS/SSE/Webhook/WebAuthn/A2A 协议。限流: 请求速率限制。幂等: 幂等存储支持。TLS 硬化: rustls 替代 OpenSSL。硬件抽象: Peripheral trait（STM32/RPi GPIO、I2C/SPI）。aardvark-sys: FFI 绑定硬件接口。robot-kit: 机器人控制工具包。评估框架: zeroclaw-eval 评估 Agent 性能。i18n: t() 宏多语言国际化。Tauri 桌面应用。双模式构建: --no-default-features。

**依赖库**: axum（HTTP 50+ 路由）、rustls（TLS 硬化）、tokio-tungstenite（WebSocket）、ring（WebAuthn/Ed25519）、linux-embedded-hal（GPIO/I2C/SPI）、aardvark-sys（FFI 硬件绑定）、tauri（桌面应用）、fluent（i18n）。

**原理**: Gateway HTTP 使用 axum 路由器: 每个协议（REST/WS/SSE/Webhook/WebAuthn/A2A）注册不同的路由组。限流中间件在 axum 层实现: 滑动窗口算法记录每个客户端的请求时间戳。幂等存储: 每个请求分配唯一 ID，在幂等表中记录已处理请求，重复请求返回缓存结果。TLS 硬化: rustls 提供 Rust 实现的 TLS 栈，避免 OpenSSL 的内存安全漏洞。Peripheral trait 抽象硬件接口: STM32 通过串口通信，RPi GPIO 通过 /sys/class/gpio 或 linux-embedded-hal。aardvark-sys 通过 FFI 调用 C 库控制 I2C/SPI 总线。robot-kit 封装机器人控制 API（运动控制、传感器读取）。zeroclaw-eval 评估框架: 定义评估任务集，运行 Agent 并收集指标（准确率、耗时、token 用量）。

**优势**: Gateway HTTP 50+ 路由覆盖最全面的 API 协议（四者中独有）；限流 + 幂等 + TLS 硬化企业级安全；硬件抽象 Peripheral trait 支持 IoT 场景（四者中独有）；aardvark-sys FFI 硬件控制（四者中独有）；robot-kit 机器人控制（四者中独有）；评估框架支持性能评估；i18n 多语言国际化；Tauri 桌面应用；双模式构建。

**劣势**: 50+ 路由维护成本高；硬件支持增加编译复杂度和二进制大小；aardvark-sys FFI 依赖特定硬件；robot-kit 仅支持特定机器人型号；评估框架需要维护评估任务集；系统过于庞大，不适合轻量级场景。

---

## 总结对比矩阵

| 维度 | Shadow | Hermes | Claude Code | ZeroClaw |
|------|--------|--------|-------------|----------|
| 语言 | Rust | TypeScript | TypeScript | Rust |
| 代码量 | 23887行 | ~100000行 | ~50000行 | 608951行 |
| 核心 Trait | 6个 | interface | 无抽象 | 7+5个 |
| Provider 数 | 2 (OpenAI/Anthropic) | 4+ (OpenAI/Anthropic/Gemini/Ollama) | 1 (Anthropic) | 50+ family |
| Memory 后端 | 3种 | 1种 (SQLite) | 0种 | 7种 |
| Tool 种类 | ~10种 | ~15种 | 6种固定 | ~15种 + WASM |
| Channel 数 | 1 (CLI) | 30+ | 1 (CLI) | 37 |
| 循环检测 | 有 | 无 | 无 | 有 |
| 参数校验 | jsonschema | 无 | 无 | 无 |
| 工具装饰器 | PathGuarded/RateLimited | 无 | 无 | 有 |
| 密钥加密 | ChaCha20 | 无 | 无 | ChaCha20 |
| WASM 插件 | 设计 | 无 | 无 | wasmtime 43组件 |
| Gateway HTTP | 设计 | 无 | 无 | axum 50+路由 |
| 硬件支持 | 无 | 无 | 无 | STM32/RPi/Aardvark |
| MCP 支持 | 设计 | 有 | 无 | 有 |
| 双模式构建 | 有 | 无 | 无 | 有 |
| 交叉编译 | ARM | 无 | 无 | 无 |
| Tauri 桌面 | 无 | 无 | 无 | 有 |
| i18n | 无 | 无 | 无 | 有 |
| TTS/语音 | 无 | TTS | 无 | TTS + 转写 |
| 生命周期 Hook | 无 | 无 | 无 | 14个 |
| SOP 引擎 | 无 | 无 | 无 | 7种SOP |
| 知识图谱 | 无 | 无 | 无 | 有 |

### 各系统定位

- **Shadow**: 高性能 Rust Agent 运行时。核心优势: 类型安全 + 性能 + 精简设计 + 参数校验（独有）+ 工具装饰器（独有）+ 交叉编译 ARM（独有）。适合嵌入式/边缘/本地场景。
- **Hermes**: 全功能个人 AI 助手。核心优势: 30+渠道 + MCP 原生 + Browser 工具 + 子代理委派 + 技能搜索分类。适合跨平台个人助手场景。
- **Claude Code**: 专注代码生成。核心优势: Anthropic 深度集成 + extended_thinking + prompt_caching + 精炼工具集。适合开发者代码辅助场景。
- **ZeroClaw**: 企业级 Agent 平台。核心优势: 全栈覆盖（trait 到硬件）+ WASM 插件 + 37渠道 + 50+provider + Gateway 50+路由 + 硬件支持 + 14 Hook + SOP 引擎。适合生产环境/物联网/企业级场景。

---

> 本文档基于源码分析和技术文档编写，旨在为 Shadow 的发展规划提供参考。各系统能力可能会随版本迭代而变化。

---

## 附录: Tool 系统逐项对比 (Shadow 15 vs ZeroClaw ~120)

> Shadow: 3611行 15工具 | ZeroClaw: 119736行 ~120工具

### 按类别对比

| 类别 | Shadow | ZeroClaw | 差距 |
|------|--------|----------|------|
| 文件操作 | 5 (read/write/edit/glob/content_search) | 8 (+download/upload/bundle/backup) | 缺3 |
| Shell执行 | 1 (shell) | 5 (+claude_code/codex/gemini/opencode CLI) | 缺4 |
| HTTP/网络 | 2 (http_request/web_fetch) | 4 (+web_search/search_routing) | 缺2 |
| Git | 1 (git_ops) | 1 (git_operations) | 对齐 |
| Memory | 2 (recall/store) | 5 (+export/forget/purge) | 缺3 |
| Skills | 2 (manage/http) | 4 (+read_skill/skill_tool) | 基本对齐 |
| 子代理 | 1 (spawn_subagent) | 2 (+delegate) | 基本对齐 |
| Cron Tool | 0 (有持久化无Tool) | 6 (add/list/remove/run/runs/update) | 缺6 |
| MCP | 0 | 9 (client/transport/protocol/tool/deferred/prompt/resource) | 缺9 |
| 浏览器 | 0 | 3 (browser/open/delegate) | 缺3 |
| SOP | 0 | 6 (list/execute/approve/advance/status/history) | 缺6 |
| 安全 | 0 (在agent层) | 3 (security_ops/verifiable_intent/safety_net) | 缺3 |
| 集成服务 | 0 | ~15 (email/google/jira/notion/linkedin/discord/cloud) | 缺15 |
| 硬件 | 0 | 3 (board_info/memory_map/memory_read) | 缺3 |
| 实用工具 | 0 | ~10 (calculator/weather/screenshot/pdf_read/canvas等) | 缺10 |
| 其他 | 0 | ~15 (pipeline/poll/escalate/text_browser等) | 缺15 |

### 优先补齐建议 (按价值排序)

P0 (核心能力):
1. memory_forget + memory_purge + memory_export -- 记忆管理补齐
2. cron tools (6个) -- Shadow 已有 cron 持久化, 缺 Tool 接口
3. web_search -- Web 搜索工具

P1 (扩展能力):
4. MCP client (核心) -- MCP 协议支持
5. ask_user -- 用户交互工具
6. calculator -- 计算器
7. text_browser -- 纯文本浏览器

P2 (集成能力):
8. email_read -- 邮件读取
9. pdf_read -- PDF 读取
10. screenshot -- 截图

P3 (长期目标):
11. CLI agent 集成 (claude_code/codex_cli)
12. 浏览器工具
13. SOP 引擎
14. 第三方集成 (Jira/Notion/Google)
15. 硬件工具
