# Shadow 能力分析: 缺失组件全景

> Shadow Role 枚举有 13 种角色, 但只有 6 种有对应实现。
> 本文档分析每个缺失组件的作用、归属路径、设计形态。

## 总览: Role 枚举 vs 实际实现

| Role | 有无 trait | 有无实现 | 归属 crate | 作用 |
|------|-----------|---------|-----------|------|
| Agent | 无 (AgentBuilder 注释) | 注释 | shadow-runtime | agent loop 主体 |
| Channel | Channel trait (3方法) | 空壳 | shadow-channels | 消息平台接入 |
| Tool | Tool trait | 20个工具源码 | shadow-runtime/tools | agent 可调用能力 |
| Provider | ModelProvider trait | OpenAI 兼容 | shadow-providers | LLM 后端 |
| Memory | Memory trait (30方法) | NoneMemory | shadow-memory | 长期记忆 |
| Session | SessionStore trait | JsonlSessionStore | shadow-core | 会话持久化 |
| System | 无 | 无 | shadow-runtime | 系统级操作 |
| Swarm | 无 | 无 | shadow-runtime | 多 agent 协作 |
| Cron | 无 trait, 有 CronScheduler | 有 (SQLite) | shadow-runtime | 定时任务 |
| PeerGroup | 无 | 无 | shadow-runtime | agent 间通信 |
| Skill | 无 trait, 有 SkillsService | 有 (~350行) | shadow-runtime/skills | 技能加载 |
| Mcp | 无 | 无 | shadow-runtime | MCP 工具协议 |
| Sop | 无 | 无 | shadow-runtime | 标准操作流程 |

## 1. Agent -- 已有注释代码, 需激活

| 项目 | 说明 |
|------|------|
| 是什么 | agent loop 主体: 接收用户消息 -> LLM 推理 -> 工具调用循环 -> 返回结果 |
| 归属路径 | shadow-runtime/src/agent.rs (已有注释代码) |
| 应该是 trait 还是 struct | struct + AgentBuilder, 不需要 trait (ZeroClaw 也是 struct) |
| 发挥作用的位置 | CLI 入口 / Channel orchestrator / Cron job 执行 |
| ZeroClaw 做法 | Agent struct (30字段) + AgentBuilder + run_tool_call_loop(ToolLoop) |
| Shadow 需要做什么 | 取消注释, 适配当前 trait 签名 (ChatRequest/ChatResponse/ObserverEvent), 注册工具 |

## 2. Channel -- trait 严重不足

| 项目 | 说明 |
|------|------|
| 是什么 | 消息平台抽象: Telegram/Discord/Slack/CLI 等渠道的入站监听 + 出站发送 |
| 归属路径 | shadow-core/src/channel.rs (trait) + shadow-channels/ (实现, 当前空壳) |
| 应该是 trait 还是 struct | Channel trait (需补 listen/typing/draft 等方法) |
| 发挥作用的位置 | orchestrator 调用 channel.listen() 接收消息, channel.send() 发送结果 |
| ZeroClaw 做法 | Channel trait (30方法) + 42个渠道实现 + orchestrator (23846行) |
| Shadow 需要做什么 | P0: 补 Send+Sync bound + listen() + 至少实现 CliChannel |

## 3. Skill -- 不应该是 trait, 是 Tool 的动态来源

| 项目 | 说明 |
|------|------|
| 是什么 | SKILL.md 文件解析为 Skill 结构, Skill 中的 SkillTool 转为 Box<dyn Tool> 注册到 ToolRegistry |
| 归属路径 | shadow-runtime/src/skills/mod.rs (已有 ~350行) |
| 应该是 trait 还是 Tool 变种 | **Tool 的动态来源, 不是 trait**。Skill 解析后生成 SkillTool (实现 Tool trait), 注入 ToolRegistry |
| 发挥作用的位置 | agent 启动时: SkillsService::load() -> 解析 SKILL.md -> all_tools() -> ToolRegistry::extend() |
| ZeroClaw 做法 | SkillsService (四源合并: Workspace/OpenSkills/Plugin/Bundle) + SkillDocument(frontmatter解析) + SkillForge(自改进) + skill_tool (Tool impl) + skill_http (HTTP 工具) |
| Shadow 当前 | SkillsService + Skill + SkillTool(数据结构) + parse_skill_md, 但 SkillTool 未实现 Tool trait |
| Shadow 需要做什么 | 为 SkillTool 实现 Tool trait (execute 执行 shell 命令模板), 注册到 attribution.rs, 在 default_tools 中加载 |

## 4. Cron -- 已有实现, 缺 trait 抽象

| 项目 | 说明 |
|------|------|
| 是什么 | 定时任务调度: cron 表达式解析 + SQLite 持久化 + 按时触发 agent turn |
| 归属路径 | shadow-runtime/src/cron/ (已有 schedule/types + CronScheduler) |
| 应该是 trait 还是 struct | CronScheduler struct (不需要 trait, 只有一个实现) |
| 发挥作用的位置 | agent 启动时启动调度器, 到时间时调用 agent.chat() |
| ZeroClaw 做法 | CronScheduler (7306行, SQLite) + 安全验证 + announce 投递 + 工具接口 (cron_add/list/remove/run/runs) |
| Shadow 当前 | CronScheduler + schedule 验证 + types, 但 cron 工具只有 add 实现了 |
| Shadow 需要做什么 | 补齐 cron_list/remove/run/runs 工具, 在 agent 启动时启动调度器 |

## 5. MCP -- 需要 trait + 动态工具集

| 项目 | 说明 |
|------|------|
| 是什么 | Model Context Protocol: 外部工具服务器, 动态发现 + 延迟加载 + 激活后转为 Tool |
| 归属路径 | shadow-runtime/src/mcp/ (未创建) |
| 应该是 trait 还是 Tool 变种 | **两者都是**: McpRegistry (trait/struct, 管理连接) + ActivatedToolSet (动态工具集) + McpToolWrapper (实现 Tool trait) |
| 发挥作用的位置 | agent 启动时: McpRegistry::connect_all() -> 构建 DeferredMcpToolSet -> 系统提示词列出可用工具 -> LLM 调用 tool_search -> 激活 -> 转为 Tool |
| ZeroClaw 做法 | DeferredMcpToolSet (轻量桩) + ActivatedToolSet (HashMap<String, Arc<dyn Tool>>) + tool_search 工具 (关键词搜索激活) + get_resolved (后缀匹配) |
| Shadow 当前 | Role::Mcp 枚举有, 但无任何实现 |
| Shadow 需要做什么 | P3: McpRegistry + DeferredMcpToolSet + ActivatedToolSet + ToolSearchTool |

## 6. SOP -- 需要 trait + 引擎

| 项目 | 说明 |
|------|------|
| 是什么 | Standard Operating Procedure: 多步骤工作流引擎, 支持 trigger 触发 / 步骤路由 / 审批 / 条件分支 |
| 归属路径 | shadow-runtime/src/sop/ (未创建) |
| 应该是 trait 还是 struct | SopEngine struct (不需要 trait, 引擎只有一个实现) + Sop/SopStep/SopRun 数据结构 |
| 发挥作用的位置 | 独立运行, 可被 cron/channel/手动触发; 执行时可能调用 agent turn + 工具 |
| ZeroClaw 做法 | SopEngine (engine.rs) + SopRunStore (持久化) + SopMetricsCollector + 6个工具 (sop_list/execute/approve/advance/status/history) + SopExecutionMode (Auto/Supervised/StepByStep/PriorityBased/Deterministic) + condition 路由 |
| Shadow 当前 | Role::Sop 枚举有 + ToolKind 有 Sop* 6个变体, 但无实现 |
| Shadow 需要做什么 | P3: SopEngine + SOP schema (YAML/TOML 定义流程) + 持久化 + 工具 |

## 7. Swarm -- 多 agent 协作

| 项目 | 说明 |
|------|------|
| 是什么 | 多个 agent 实例协作完成任务: 主 agent 可 spawn 子 agent, 子 agent 继承父权限但受限 |
| 归属路径 | shadow-runtime/src/swarm/ (未创建) |
| 应该是 trait 还是 struct | 不需要 trait。是 Agent + SpawnSubagentTool 的组合, ZeroClaw 也没有独立的 Swarm trait |
| 发挥作用的位置 | agent loop 中 LLM 调用 spawn_subagent 工具 -> 创建子 Agent (降级权限) -> 子 agent 独立跑 turn -> 结果返回父 agent |
| ZeroClaw 做法 | SpawnSubagentTool + delegate + send_message_to_peer + EscalationViolation (11种权限升级检测) + PerSenderTracker (预算继承) |
| Shadow 当前 | Role::Swarm 枚举有 + SpawnSubagentTool 源码存在, 但无权限升级检测 |
| Shadow 需要做什么 | P3: EscalationViolation 检测 + 预算继承 (有 SubAgent 时需要) |

## 8. PeerGroup -- agent 间消息路由

| 项目 | 说明 |
|------|------|
| 是什么 | 配置驱动的 agent 间通信: 定义哪些 agent 是对等关系, 允许跨 agent 发消息 |
| 归属路径 | shadow-runtime/src/peers.rs (未创建) |
| 应该是 trait 还是 struct | 不需要 trait。ResolvedPeers struct (从 Config 解析) |
| 发挥作用的位置 | orchestrator 收到消息后, 检查 sender 是否是已知 peer, 决定路由 |
| ZeroClaw 做法 | resolve_peer_set() (配置解析) + ResolvedPeers (agent_peers + external_peers, 按 channel 分组) + send_message_to_peer 工具 |
| Shadow 当前 | Role::PeerGroup 枚举有, 但无实现 |
| Shadow 需要做什么 | P3: ResolvedPeers + send_message_to_peer 工具 (有多 agent 时需要) |

## 9. Plugin -- WASM 插件系统

| 项目 | 说明 |
|------|------|
| 是什么 | WASM 模块插件: 第三方可以写 .wasm 插件, 提供自定义 Tool / Channel / Skill |
| 归属路径 | shadow-plugins/ (空壳 crate) |
| 应该是 trait 还是 struct | PluginHost struct (管理加载/签名验证) + PluginManifest (元数据) + PluginCapability (能力声明) |
| 发挥作用的位置 | 启动时 PluginHost 扫描 plugins/ 目录, 加载 .wasm, 注册为 Tool/Skill |
| ZeroClaw 做法 | PluginHost (WASM 加载) + 签名验证 (SignatureMode) + 技能子目录 (插件可携带 skills/) + PluginManifest |
| Shadow 当前 | shadow-plugins 空壳 (6行), Role 中 ChannelKind::Plugin 有 |
| Shadow 需要做什么 | P3/P4: 需要先确定 WASM runtime (wasmtime/wasmer), 工作量大 |

## 10. Gateway -- HTTP API 服务

| 项目 | 说明 |
|------|------|
| 是什么 | HTTP REST API + WebSocket + SSE, 让 Web 前端 / 外部系统控制 agent |
| 归属路径 | shadow-gateway/ (空壳 crate) |
| 应该是 trait 还是 struct | GatewayServer struct (axum/hyper, 不需要 trait) |
| 发挥作用的位置 | daemon 模式启动时, Gateway 监听 HTTP 端口, 提供配置/会话/日志/工具等 API |
| ZeroClaw 做法 | axum HTTP gateway + ACP (Agent Communication Protocol) + SSE 流式 + WS 双向 + WebAuthn 认证 + 静态文件服务 + TLS |
| Shadow 当前 | shadow-gateway 空壳 (6行) |
| Shadow 需要做什么 | P3: axum 基础 API (chat/sessions/config/logs) |

## 11. Daemon -- 长驻进程

| 项目 | 说明 |
|------|------|
| 是什么 | agent 作为后台服务运行: 信号处理 + 配置热更新 + 多渠道并行 + 生命周期管理 |
| 归属路径 | shadow-runtime/src/daemon/ (未创建) |
| 应该是 trait 还是 struct | daemon 函数 (不需要 trait) |
| 发挥作用的位置 | `shadow daemon` 命令启动, 替代 `shadow chat` 的一次性模式 |
| ZeroClaw 做法 | daemon::run() + DaemonExit(Shutdown/Reload) + wait_for_exit_signal (SIGINT/SIGTERM) + 配置 watch channel 热更新 + GatewayReloadControls |
| Shadow 当前 | 无, 只有一次性 CLI chat |
| Shadow 需要做什么 | P2: 信号处理 + 配置 reload + 多渠道启动 |

## 12. Spawn -- 归因传播的 tokio::spawn

| 项目 | 说明 |
|------|------|
| 是什么 | tokio::spawn 的包装: 自动传播 attribution span + 生命周期遥测日志 |
| 归属路径 | shadow-spawn/ (空壳 crate) |
| 应该是 trait 还是 struct | spawn! 宏 (不需要 trait) |
| 发挥作用的位置 | 所有需要 spawn 后台 task 的地方 (memory consolidation / cron 执行 / skill review 等) |
| ZeroClaw 做法 | spawn! 宏 -> tokio::spawn + Instrument::in_current_span() + record!(Spawn) + record!(Complete) |
| Shadow 当前 | shadow-spawn 空壳 (6行) |
| Shadow 需要做什么 | P2: spawn! 宏 (简单, 依赖 shadow-log) |

## 13. TTS / Transcription -- 语音能力

| 项目 | 说明 |
|------|------|
| 是什么 | TTS: 文本转语音; Transcription: 语音转文本 |
| 归属路径 | shadow-providers/ (ProviderKind 已有 Tts/Transcription 枚举) |
| 应该是 trait 还是 struct | TtsProvider trait + TranscriptionProvider trait (类似 ModelProvider) |
| 发挥作用的位置 | Channel 收到语音消息 -> Transcription 转文本 -> agent 处理 -> TTS 转语音回复 |
| ZeroClaw 做法 | ProviderKind 含 Tts/Transcription + channel 集成 (start_typing -> TTS) + SendMessage.suppress_voice |
| Shadow 当前 | ProviderKind::Tts(TtsProviderKind::Plugin) + Transcription(TranscriptionProviderKind::Google/Plugin) 枚举有, 但无 trait 无实现 |
| Shadow 需要做什么 | P3: TtsProvider trait + TranscriptionProvider trait (有语音需求时) |

## 14. Hardware -- 硬件外设

| 项目 | 说明 |
|------|------|
| 是什么 | USB 设备发现 / GPIO / I2C / SPI / 串口通信, 控制 STM32/树莓派等硬件 |
| 归属路径 | shadow-hardware/ (空壳 crate) |
| 应该是 trait 还是 struct | Peripheral trait (硬件抽象) + 各设备实现 |
| 发挥作用的位置 | agent 通过 hardware_* 工具控制硬件 |
| ZeroClaw 做法 | Peripheral trait + Aardvark FFI + RPi GPIO + STM32 flash + Arduino upload + datasheet + serial |
| Shadow 当前 | shadow-hardware 空壳 (6行) |
| Shadow 需要做什么 | P4: 有硬件需求时再考虑 |

## 优先级路线图

```
P0 (阻塞 agent loop):
  1. Agent -- 取消注释, 适配 trait 签名
  2. Tool 装配 -- default_tools 注册工具
  3. Memory 后端 -- 取消注释 sqlite/markdown

P1 (核心可用):
  4. Cron 工具补齐
  5. Skill 实现 Tool trait
  6. Channel trait 补全 + CliChannel
  7. Spawn 宏

P2 (生产级):
  8. Daemon 模式
  9. Gateway HTTP API
  10. MCP 动态工具
  11. TTS/Transcription

P3 (扩展):
  12. SOP 引擎
  13. Swarm (SubAgent 权限检测)
  14. PeerGroup (多 agent 路由)
  15. Plugin (WASM)

P4 (远期):
  16. Hardware
```

## 设计原则

1. **能不引入 trait 就不引入**: Agent/Cron/Sop/Daemon/Gateway 都是 struct, 只有 Provider/Memory/Tool/Channel/Observer 需要 trait (因为有多实现)

2. **Tool 是万能扩展点**: SkillTool / McpToolWrapper / SopTool 都是 Tool trait 的实现, 不是独立 trait。新能力优先考虑"能不能做成一个 Tool"

3. **Role 枚举是归因标签, 不是架构**: Role::Sop 有值不代表必须有 SopTrait。Role 只用于日志归因 (record! / attribution_span!), 实现可以是 struct / 函数 / 宏

4. **空壳 crate 按需激活**: shadow-channels / shadow-gateway / shadow-plugins 等空壳 crate 不需要提前填充, 等对应功能进入开发时再激活
