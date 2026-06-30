# Shadow vs ZeroClaw 差距分析

> 生成时间: 2026-07-01
> Shadow 版本: 0.1.0 | ZeroClaw 版本: 0.8.2

## 一、规模对比

| 维度 | Shadow | ZeroClaw | 比值 |
|------|--------|----------|------|
| 总代码行 | 3,945 | 630,103 | 0.6% |
| 源文件数 | 28 | 975 | 2.9% |
| Crate 数 | 7 | 23 | 30% |
| Trait 数 | 6 | 18 | 33% |
| Trait 实现数 | ~15 | ~60 | 25% |
| 测试数 | 34 | ~500+ | ~7% |
| 二进制大小 (release) | ~2.5 MB | ~26 MB | 10% |

### 各 Crate 对比

| Shadow Crate | 行数 | ZeroClaw 对应 | 行数 | 比值 |
|--------------|------|---------------|------|------|
| agent-core | 486 | zeroclaw-api | 6,293 | 7.7% |
| shadow-config | 689 | zeroclaw-config | 65,171 | 1.1% |
| shadow-log | 494 | zeroclaw-log | 5,079 | 9.7% |
| shadow-runtime | 843 | zeroclaw-runtime | 157,684 | 0.5% |
| shadow-providers | 320 | zeroclaw-providers | 49,664 | 0.6% |
| shadow-memory | 178 | zeroclaw-memory | 17,442 | 1.0% |
| src/main.rs | 597 | src/main.rs | 8,672 | 6.9% |
| — (无) | 0 | zeroclaw-channels | 122,677 | — |
| — (无) | 0 | zeroclaw-tools | 62,235 | — |
| — (无) | 0 | zeroclaw-gateway | 29,861 | — |
| — (无) | 0 | zeroclaw-hardware | 11,163 | — |
| — (无) | 0 | zeroclaw-infra | 4,888 | — |
| — (无) | 0 | zeroclaw-plugins | 4,243 | — |
| — (无) | 0 | zeroclaw-tool-call-parser | 3,869 | — |
| — (无) | 0 | zeroclaw-macros | 2,917 | — |
| — (无) | 0 | zeroclaw-eval | 1,430 | — |
| — (无) | 0 | zeroclaw-spawn | 231 | — |
| — (无) | 0 | robot-kit | 3,514 | — |
| — (无) | 0 | aardvark-sys | 483 | — |

---

## 二、Trait 对比

### Shadow 已有的 Trait (6 个)

| Shadow Trait | 方法数 | ZeroClaw 对应 | 方法数 | 差距 |
|--------------|--------|---------------|--------|------|
| Attributable | 2 (role/alias) | Attributable | 2 | 一致 |
| ModelProvider | 5 | ModelProvider | 8+ | 缺 pricing/capabilities/default_base_url |
| Tool | 7 (含 timeout/approval) | Tool | 5 | Shadow 多了 timeout/approval |
| Memory | 5 | Memory | 25+ | 严重不足 |
| Observer | 3 | Observer | 5 | 缺 record_metric |
| Channel | 3 | Channel | 30+ | 严重不足 |

### ZeroClaw 有但 Shadow 没有的 Trait (12 个)

| ZeroClaw Trait | 用途 | Shadow 是否需要 |
|----------------|------|-----------------|
| RuntimeAdapter | 执行环境抽象 (Native/Docker/Wasm) | P2 |
| Peripheral | 硬件外设 (GPIO/I2C/SPI) | P3 |
| Sandbox | OS 级进程隔离 (Docker/Firejail/Landlock) | P1 |
| SopRunStore | SOP 运行状态持久化 | P3 |
| RpcTransport | RPC 传输层 (Unix socket/WSS) | P2 |
| TaskRegistry | 任务注册/调度/回收 | P2 |
| HookHandler | 生命周期钩子 (Void/Modifying) | P2 |
| Tunnel | 隧道抽象 (Cloudflare/Ngrok/Tailscale) | P3 |
| Scout | 技能发现 | P3 |
| ToolDispatcher | 工具协议分发 (Native/XML) | P1 |
| MemoryStrategy | 记忆加载/合并/治理 | P1 |
| PromptSection | 系统提示可插拔段 | P1 |

---

## 三、功能差距矩阵

### 3.1 Agent 核心引擎

| 功能 | Shadow | ZeroClaw | 差距等级 |
|------|--------|----------|----------|
| Agent loop (tool call 循环) | ✅ 基础 | ✅ 22 步流水线 | 中 |
| Turn pipeline 分层 | ❌ 单函数 | ✅ 24 文件 | 高 |
| 历史管理 | ✅ max_history 截断 | ✅ 四层 (预检/整轮裁剪/结果截断/孤儿清理) | 高 |
| 循环检测 | ❌ | ✅ 三种模式 (exact/ping-pong/no-progress) | 高 |
| 工具超时 | ✅ timeout() trait 方法 | ✅ tokio::time::timeout | 一致 |
| 工具审批 | ✅ Supervised 模式 | ✅ 三模式 (CLI/channel/ACP) + resolve_gate | 中 |
| 上下文溢出恢复 | ❌ | ✅ 整轮裁剪 + 重试 | 高 |
| 流式响应 | ❌ | ❌ (ZeroClaw 也未实现) | 一致 |
| AgentBuilder | ✅ 基础 | ✅ 30+ 字段 | 中 |
| Token 预算追踪 | ❌ | ✅ shared_budget | 高 |

### 3.2 Provider 层

| 功能 | Shadow | ZeroClaw | 差距等级 |
|------|--------|----------|----------|
| OpenAI 兼容 | ✅ 1 个实现 | ✅ CompatFamilySpec blanket impl | 中 |
| Anthropic 原生 | ❌ | ✅ 原生 tool_use 格式 | 高 |
| Provider 数量 | 1 (OpenAI 兼容) | 72+ (含中国厂商) | 高 |
| 可靠重试 | ❌ | ✅ ReliableModelProvider (重试+退避+key轮换) | 高 |
| Fallback 链 | ❌ | ✅ fallback + fallback_models | 中 |
| 流式 (SSE) | ❌ | ❌ | 一致 |
| OAuth 认证 | ❌ | ✅ Qwen/MiniMax OAuth | P2 |
| Provider 归因 span | ❌ | ✅ ProviderDispatch | 中 |

### 3.3 Memory 层

| 功能 | Shadow | ZeroClaw | 差距等级 |
|------|--------|----------|----------|
| None 后端 | ✅ | ✅ | 一致 |
| Markdown 后端 | ✅ 关键词匹配 | ✅ | 一致 |
| SQLite 后端 | ❌ | ✅ 4,015 行 + FTS5 | 高 |
| PostgreSQL 后端 | ❌ | ✅ pgvector | P3 |
| Qdrant 向量 | ❌ | ✅ 1,161 行 | P3 |
| Lucid 混合 | ❌ | ✅ SQLite + 本地嵌入 | P2 |
| Agent 隔离 | ❌ | ✅ AgentScopedMemory | 中 |
| 审计包装器 | ❌ | ✅ AuditedMemory | P2 |
| 记忆卫生 | ❌ | ✅ hygiene (定期清理/归档) | 中 |
| 记忆合并 | ❌ | ✅ consolidation (LLM 提取) | P2 |
| WASM 插件 | ❌ | ✅ WasmMemory | P3 |

### 3.4 Config 层

| 功能 | Shadow | ZeroClaw | 差距等级 |
|------|--------|----------|----------|
| TOML 加载/保存 | ✅ | ✅ | 一致 |
| 多 Provider | ✅ flatten HashMap | ✅ 60+ typed slot | 中 (设计不同) |
| 密钥加密 | ❌ 明文 | ✅ ChaCha20-Poly1305 AEAD | 高 |
| 版本迁移 | ❌ | ✅ V1->V2->V3 链 | 中 |
| 环境变量覆盖 | ❌ | ✅ ZEROCLAW_* 系列 | 中 |
| Configurable derive | ❌ | ✅ 自动生成 secret/prop 管理 | P2 |
| config set | ✅ dotted path | ✅ | 一致 |
| Schema 导出 | ❌ | ✅ JSON Schema | P2 |

### 3.5 Log 层

| 功能 | Shadow | ZeroClaw | 差距等级 |
|------|--------|----------|----------|
| record! 宏 | ✅ | ✅ | 一致 |
| LogCaptureLayer | ✅ 基础 | ✅ 762 行 (span 遍历+归因合并) | 高 |
| JSONL 持久化 | ✅ 滚动裁剪 | ✅ 滚动+轮转+日期+大小 | 中 |
| 广播 (SSE) | ✅ broadcast channel | ✅ | 一致 |
| Observer 桥接 | ✅ LogObserver | ✅ observer_bridge.rs | 一致 |
| 归因 span 自动填充 | ❌ | ✅ attribution_span! + span scope 遍历 | 高 |
| Action 封闭枚举 | ✅ 12 种 | ✅ 37 种 | 中 |
| OTel/ECS schema | ❌ 简化 | ✅ 完整 | 中 |
| 旧版迁移 | ❌ | ✅ migrate_legacy_jsonl | P2 |

### 3.6 Security

| 功能 | Shadow | ZeroClaw | 差距等级 |
|------|--------|----------|----------|
| AutonomyLevel | ✅ Full/Supervised/ReadOnly | ✅ | 一致 |
| Sandbox | ❌ | ✅ 6 后端 (Docker/Firejail/Bubblewrap/Landlock/Seatbelt/Noop) | 高 |
| EstopManager | ❌ | ✅ KillAll/NetworkKill/DomainBlock/ToolFreeze | 高 |
| LeakDetector | ❌ | ✅ 8 类正则 + Shannon 熵 | 高 |
| PromptGuard | ❌ | ✅ 6 类检测 | 高 |
| OTP | ❌ | ✅ TOTP | P2 |
| IAM | ❌ | ✅ deny-by-default 角色映射 | P3 |
| Audit (Merkle) | ❌ | ✅ SHA-256 哈希链 | P3 |
| 危险命令黑名单 | ✅ 4 个模式 | ❌ (ZeroClaw 无) | Shadow 优 |

### 3.7 工具层

| 功能 | Shadow | ZeroClaw | 差距等级 |
|------|--------|----------|----------|
| Shell | ✅ 黑名单+超时+截断 | ✅ 沙箱化+ChildGroupGuard | 中 |
| FileRead | ✅ 100KB 截断 | ✅ | 一致 |
| FileWrite | ✅ 原子写入+append | ✅ | 一致 |
| 工具数量 | 3 | 80+ | 高 |
| RateLimitedTool | ❌ | ✅ 装饰器模式 | P1 |
| PathGuardedTool | ❌ | ✅ 路径安全守卫 | P1 |
| MCP 协议 | ❌ | ✅ 完整 MCP 客户端 | P2 |
| 第三方集成 | ❌ | ✅ Jira/Notion/LinkedIn/Google/Microsoft | P3 |
| 工具注册表 | ❌ | ✅ 工厂+注册 | P1 |

### 3.8 通信层 (Shadow 完全缺失)

| 功能 | Shadow | ZeroClaw | 差距等级 |
|------|--------|----------|----------|
| Channel (消息平台) | ❌ (仅 CliChannel) | ✅ 30+ 平台 | 高 |
| Gateway (HTTP/WS) | ❌ | ✅ axum + REST + WS + SSE | 高 |
| RPC (JSON-RPC) | ❌ | ✅ 60+ method + exhaustive match | 高 |
| Daemon | ❌ | ✅ 生命周期+supervisor+ephemeral | 高 |
| Orchestrator | ❌ | ✅ 23,846 行 | 高 |

### 3.9 高级功能 (Shadow 完全缺失)

| 功能 | Shadow | ZeroClaw | 差距等级 |
|------|--------|----------|----------|
| SOP 引擎 | ❌ | ✅ 5 种模式 + 审批子系统 | P3 |
| Cron 调度 | ❌ | ✅ SQLite 持久化 | P2 |
| Skills 系统 | ❌ | ✅ 四源合并+审计+缓存+自改进 | P2 |
| Heartbeat | ❌ | ✅ 两阶段 LLM 决策 | P3 |
| SubAgent | ❌ | ✅ 权限收窄+预算继承 | P2 |
| Verifiable Intent | ❌ | ✅ SD-JWT 分层凭证 | P3 |
| Control Plane | ❌ | ✅ 任务注册+reaper 回收 | P2 |
| Hooks | ❌ | ✅ Void/Modifying 双模式 | P2 |
| Trust 系统 | ❌ | ✅ 信任分数+衰减+回归 | P3 |
| WASM 插件 | ❌ | ✅ WIT Component Model + Ed25519 | P3 |
| Tunnel | ❌ | ✅ 7 后端 | P3 |
| TUI (zerocode) | ❌ | ✅ 37,223 行 ratatui | P3 |
| Desktop (Tauri) | ❌ | ✅ 1,805 行 | P3 |
| Kernel-only 构建 | ✅ | ✅ | 一致 |

---

## 四、架构设计差距

### 4.1 Attributable 归因系统

| 维度 | Shadow | ZeroClaw |
|------|--------|----------|
| Role 枚举 | 6 种 (无子枚举) | 14 种 + 6 个子枚举 (72 provider kind + 37 channel kind) |
| composite 字段 | ❌ | ✅ channel.type / model_provider.type |
| attribution_span! 宏 | ✅ 有宏但 Layer 不解析 | ✅ 宏 + LogCaptureLayer 自动 span 遍历 |
| 常量表驱动 | ❌ | ✅ ATTRIBUTION_FIELDS + COMPOSITE_PREFIXES |
| blanket impl | ✅ Arc/Box/& | ✅ |

### 4.2 单一入口禁止门

| 禁止门 | Shadow | ZeroClaw |
|--------|--------|----------|
| record! 宏 | ✅ 有宏但无 clippy 强制 | ✅ workspace 级 clippy disallowed_macros |
| spawn! 宏 | ❌ | ✅ zeroclaw-spawn crate |
| 单一真相源 | ❌ 无强制 | ✅ AGENTS.md 强制 + architecture test |

### 4.3 依赖方向

| 维度 | Shadow | ZeroClaw |
|------|--------|----------|
| 依赖向内流动 | ✅ agent-core 零依赖 | ✅ |
| runtime 不知 channel | ✅ | ✅ |
| 编译器强制分层 | ❌ 无 | ✅ crate 边界 |
| Feature gate 矩阵 | ✅ runtime/kernel | ✅ 40+ feature |

---

## 五、Shadow 的优势 (ZeroClaw 没有的)

| 功能 | 说明 |
|------|------|
| 危险命令黑名单 | ShellTool 自带 rm -rf / 等检测, ZeroClaw 靠 Sandbox |
| ToolEventCallback | 工具执行可视化回调, ZeroClaw 用 Observer 但无 CLI 专用回调 |
| config set 命令行 | dotted path 直接写入, ZeroClaw 需手动编辑 |
| 简化的 Provider 配置 | flatten HashMap 任意 family/alias, ZeroClaw 60+ typed slot |
| 代码可读性 | 3,945 行 vs 630,103 行, 新人可快速理解 |

---

## 六、优先级排序 (基于差距分析)

### P0 — 核心差距 (使 Shadow 达到 "可用")

1. **ToolDispatcher trait** — Native/XML 双协议, 支持 non-native-tool 模型
2. **循环检测器** — 防止 agent 陷入无限循环
3. **上下文溢出恢复** — 400 错误后整轮裁剪 + 重试
4. **SQLite Memory 后端** — 对话历史持久化的基础
5. **历史持久化** — Agent history 写入 JSONL/SQLite, 启动恢复

### P1 — 安全与健壮 (使 Shadow 达到 "生产可用")

6. **Sandbox trait** — 至少 NoopSandbox + DockerSandbox
7. **LeakDetector** — API key/密钥泄漏检测 + scrub()
8. **RateLimitedTool / PathGuardedTool** — 工具安全包装器
9. **PromptSection trait** — 可插拔系统提示段
10. **MemoryStrategy trait** — 记忆加载/合并/治理
11. **Anthropic Provider** — 原生 tool_use 格式
12. **ReliableModelProvider** — 重试+退避+key 轮换

### P2 — 通信与扩展

13. **Gateway** — HTTP/WS/SSE 服务
14. **RPC** — JSON-RPC + Unix socket
15. **Daemon** — 长运行进程
16. **Channel trait 完整实现** — 至少 Telegram/Discord
17. **Cron 调度** — 定时任务
18. **SubAgent** — 子代理权限收窄
19. **Control Plane** — 任务注册/回收
20. **Hooks** — 生命周期钩子

### P3 — 远期目标

21. SOP 引擎 / Skills 系统 / WASM 插件 / Tunnel / TUI / Desktop / 硬件 / 评估

---

## 七、总结

Shadow 当前是 ZeroClaw 的 **0.6% 代码量**, 实现了核心 trait 架构和基本工具调用, 但在以下维度有显著差距:

- **安全**: 无 Sandbox/LeakDetector/PromptGuard/Estop (ZeroClaw 10,149 行)
- **通信**: 无 Channel/Gateway/RPC/Daemon (ZeroClaw 298,000+ 行)
- **持久化**: 无 SQLite Memory/历史持久化 (ZeroClaw 17,442 行)
- **高级**: 无 SOP/Cron/Skills/SubAgent/Plugins (ZeroClaw 50,000+ 行)
- **归因**: LogCaptureLayer 不做 span 遍历, 归因不自动填充

Shadow 的设计方向正确 (trait 驱动 + 微内核 + 归因), 核心骨架可扩展。下一步应聚焦 P0 项 (ToolDispatcher + 循环检测 + SQLite Memory + 历史持久化), 使 agent 达到真正可用。
