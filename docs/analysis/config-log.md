# Shadow 能力分析: Config/Log/Observability 配置与可观测性

> 对比 ZeroClaw 与 Shadow 的配置和日志系统

## 1. Config Schema 对比

| 项目 | ZeroClaw | Shadow | 差距 |
|------|----------|--------|------|
| 模块数 | 25 个子模块 | 8 个子模块 | 缺 17 |
| Configurable 宏 | zeroclaw-macros 派生宏 | 无 | 缺 |
| 配置迁移 | migration.rs (版本链) | migration.rs (有) | 基本对齐 |
| env_overrides | 环境变量覆盖 | 无 | 缺 |
| presets | 预设配置模板 | 无 | 缺 |
| validation_warnings | 配置校验警告 | 无 | 缺 |
| comment_writer | TOML 注释写入 | 无 | 缺 |
| field_visibility | 字段可见性控制 | 无 | 缺 |

## 2. TOML Schema 对比

| 配置块 | ZeroClaw | Shadow | 差距 |
|--------|----------|--------|------|
| [agent] | alias/model/temperature/autonomy/workspace | alias/model/temperature/autonomy | 基本对齐 |
| [providers.*] | models.<family>.<alias> 嵌套 | <family>.<alias> 扁平 | Shadow 更简洁 |
| [memory] | backend + embedding + routes | backend = "none" | Shadow 严重简化 |
| [security] | 25+ 字段 policy | 无独立配置块 | 缺 |
| [channels.*] | 每渠道独立配置 | 无 | 缺 |
| [cron] | 调度配置 | 无 | 缺 |
| [observability] | log_persistence + metrics | 无 | 缺 |
| [autonomy] | delegation/approval_route/on_no_approver | autonomy = "supervised" | 严重简化 |
| multi_agent | multi_agent.rs + alias_refs | multi/ (alias_agent/risk_profile/runtime_profile/skill_bundle) | Shadow 有多 agent 框架 |
| secrets | SecretStore 加密 | secrets.rs (有) | 基本对齐 |

## 3. Log 系统对比

| 项目 | ZeroClaw | Shadow | 差距 |
|------|----------|--------|------|
| record! 宏 | 有 (唯一日志发射点) | 有 | 对齐 |
| attribution_span! 宏 | 有 | 有 | 对齐 |
| LogEvent schema | OTel/ECS + zeroclaw.* 命名空间 | Severity x Action x Category x Outcome | 设计不同但功能等价 |
| JSONL 持久化 | runtime-trace.jsonl | 有 | 对齐 |
| 分页查询 | reader.rs | load_page() | 对齐 |
| broadcast hook | subscribe() | subscribe() + set_broadcast_hook() | 对齐 |
| LogCaptureLayer | tracing Layer 解析 span 归因 | 有 | 对齐 |
| observer_bridge | Observer 事件桥接 | 有 | 对齐 |
| tool_io | 工具 I/O 记录 | 无 | 缺 |
| chain | display_chain 日志链 | 无 | 缺 |
| migrate | 日志格式迁移 | 无 | 缺 |

## 4. Observer 对比

| 项目 | ZeroClaw | Shadow | 差距 |
|------|----------|--------|------|
| Observer trait | record_event + Send+Sync+'static | record_event + Send+Sync+'static | 对齐 |
| 事件数 | ~15 变体 | ~20 变体 | Shadow 更多 |
| AgentStart/End | 有 | 有 | 对齐 |
| LlmRequest/Response | 有 | 有 | 对齐 |
| ToolCallStart/ToolCall | 有 | 有 | 对齐 |
| MemoryRecall/Store | 有 | 有 | 对齐 |
| HistoryTrimmed | 有 | 有 | 对齐 |
| TurnComplete | 有 | 有 | 对齐 |
| CacheHit/Miss | 有 | 有 | 对齐 |
| RagRetrieve | 有 | 有 | 对齐 |
| ChannelMessage | 有 | 有 | 对齐 |
| Deployment* | 有 | 有 | 对齐 |
| ObserverMetric | RequestLatency/TokenUsed/ActiveSessions/QueueDepth | 有 | 对齐 |
| TurnGuard (RAII) | Drop 触发 AgentEnd | 无 | 缺 |
| Snapshot 类型 | LlmMessageSnapshot/MessageSnapshot/ToolCallSnapshot | 无 | 缺 |
| TurnTokenUsage | input/output tokens | 无 | 缺 |

## 5. Shadow Config 当前内容

Shadow config 已有:
- TOML schema (agent + providers + memory)
- 多 provider 支持 (openai/anthropic/custom)
- provider 解析 (resolve_provider)
- secrets (SecretStore)
- migration (版本链)
- multi agent 框架 (alias_agent/risk_profile/runtime_profile/skill_bundle)
- autonomy (AutonomyLevel)
- model_provider (custom provider 配置)

## 6. Shadow Log 当前内容

Shadow log 已有:
- record! 宏 (唯一日志发射点)
- attribution_span! 宏 (自动归因 span)
- LogEvent schema (Severity x Action x Category x Outcome)
- JSONL 持久化 + 分页查询
- broadcast hook (实时推送)
- LogCaptureLayer (tracing Layer)
- observer_bridge (日志 -> Observer 桥接)
- LogFilter + LogPage

## 7. Shadow 差距表

| # | 能力 | 需要做什么 | 优先级 | 为什么 |
|---|------|-----------|--------|--------|
| 1 | Config: security 块 | 添加 [security] 配置节 | P1 | 安全策略需要可配置 |
| 2 | Config: observability 块 | 添加 [observability] 配置节 | P2 | 日志级别/持久化开关 |
| 3 | Config: channels 块 | 添加 [channels.*] 配置节 | P2 | 渠道配置 |
| 4 | Config: env_overrides | 环境变量覆盖配置 | P2 | 12-factor app |
| 5 | Config: presets | 预设模板 | P3 | 用户体验 |
| 6 | Log: tool_io | 工具 I/O 记录 | P2 | 调试可观测性 |
| 7 | Log: chain | 日志链展示 | P3 | 日志关联 |
| 8 | Observer: TurnGuard | RAII 保证 AgentEnd 必发 | P1 | 事件可靠性 |
| 9 | Observer: Snapshot 类型 | LlmMessageSnapshot 等 | P2 | 事件内容丰富 |
| 10 | Observer: TurnTokenUsage | input/output tokens | P2 | 成本追踪 |

## Shadow 已有优势 (不需要改)
- record! 宏 (对齐)
- attribution_span! 宏 (对齐)
- JSONL 持久化 (对齐)
- broadcast hook (对齐)
- LogCaptureLayer (对齐)
- observer_bridge (对齐)
- ObserverEvent 比 ZeroClaw 更多变体
- 多 provider 配置 (比 ZeroClaw 更简洁)
- multi agent 框架 (alias_agent/risk_profile 等)
- SecretStore (已有)
- migration (已有)
