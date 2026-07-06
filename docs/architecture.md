# Shadow 系统架构图

> 生成时间: 2026-07-05
> 总代码: 30,806 行 | 测试: 653 | Crate: 17 | 提交: 100+

## 架构总览

```
                          ┌─────────────┐
                          │   CLI / TUI  │
                          │  src/main.rs  │
                          └──────┬───────┘
                                 │
                    ┌────────────┴────────────┐
                    │    shadow-runtime        │
                    │    (14254行 330测试)     │
                    │                          │
                    │  ┌─────────────────────┐ │
                    │  │   Agent Loop        │ │
                    │  │   循环检测+溢出恢复  │ │
                    │  │   工具并行+超时+审批  │ │
                    │  └─────────────────────┘ │
                    │  ┌─────────────────────┐ │
                    │  │   Prompt Engine     │ │
                    │  │   (2269行 10模块)   │ │
                    │  │                     │ │
                    │  │  PromptSection trait │ │
                    │  │  ├─ Identity (100)  │ │
                    │  │  ├─ Persona (99)    │ │
                    │  │  ├─ Bootstrap (95)  │ │
                    │  │  ├─ DateTime (90)   │ │
                    │  │  ├─ Workspace (80)  │ │
                    │  │  ├─ SafetyInj (75)  │ │
                    │  │  ├─ Safety (70)     │ │
                    │  │  └─ ToolHonesty(60) │ │
                    │  │                     │ │
                    │  │  注入防护(10模式)   │ │
                    │  │  缓存(system_and_3) │ │
                    │  │  截断(头尾+JSON)    │ │
                    │  │  压缩(预清理+估算)  │ │
                    │  │  ToolsPayload(3格式)│ │
                    │  │  PromptGuided(降级) │ │
                    │  │  Persona(配置驱动)  │ │
                    │  └─────────────────────┘ │
                    │  ┌─────────────────────┐ │
                    │  │   Tools (26个)      │ │
                    │  │                     │ │
                    │  │  文件: read/write/  │ │
                    │  │    edit/glob/       │ │
                    │  │    content_search/  │ │
                    │  │    download/upload/ │ │
                    │  │    upload_bundle/   │ │
                    │  │    backup           │ │
                    │  │                     │ │
                    │  │  网络: http_request/│ │
                    │  │    web_fetch/       │ │
                    │  │    web_search/      │ │
                    │  │    search_routing   │ │
                    │  │                     │ │
                    │  │  记忆: recall/store/│ │
                    │  │    forget/purge/    │ │
                    │  │    export           │ │
                    │  │                     │ │
                    │  │  系统: shell/git_ops│ │
                    │  │    cron/spawn_agent │ │
                    │  │    skill_manage     │ │
                    │  │                     │ │
                    │  │  装饰器: PathGuard/ │ │
                    │  │    RateLimited/     │ │
                    │  │    SSRF防护         │ │
                    │  └─────────────────────┘ │
                    │  ┌─────────────────────┐ │
                    │  │   Security          │ │
                    │  │   黑名单+参数校验    │ │
                    │  │   自治等级+路径守卫  │ │
                    │  └─────────────────────┘ │
                    │  ┌─────────────────────┐ │
                    │  │   Cron Scheduler    │ │
                    │  │   SQLite+6字段cron  │ │
                    │  └─────────────────────┘ │
                    └──┬──────┬──────┬───────┬──┘
                       │      │      │       │
          ┌────────────┴┐ ┌──┴──┐ ┌┴────┐ ┌┴──────────┐
          │shadow-core   │ │prov │ │mem  │ │shadow-proxy│
          │(2097行 39测) │ │3706 │ │1726 │ │(2452行 33测)│
          │              │ │68测 │ │53测 │ │             │
          │ 6大Trait:    │ │     │ │     │ │ AgentTrans  │
          │ Attributable │ │ 3层:│ │ 3后端│ │ LocalAgent  │
          │ Provider     │ │ Rtr │ │ None │ │ AcpClient   │
          │ Tool         │ │ Rlb │ │ MD   │ │ A2aClient   │
          │ Memory       │ │ Prv │ │ SQLite│ │ Registry    │
          │ Channel      │ │     │ │      │ │ TaskRouter  │
          │ Observer     │ │ 2家:│ │ 语义: │ │ HttpTrans   │
          │              │ │OpenAI│ │Embed │ │ StdioTrans  │
          │ Tool:        │ │Anthr│ │Vector│ │ Discovery   │
          │  validate    │ │     │ │Hybrid│ │ ProxyTool   │
          │  timeout     │ │SSRF │ │      │ │             │
          │  approval    │ │限流 │ │策略: │ │ 3模式:      │
          │              │ │轮换 │ │Memory│ │ HTTP/stdio/ │
          │ AutonomyLvl  │ │降级 │ │Strat │ │ Embedded    │
          └──────────────┘ └─────┘ └──────┘ └─────────────┘
                               │
                    ┌──────────┴──────────┐
                    │  shadow-config       │
                    │  (1039行 5测试)      │
                    │  TOML+密钥加密       │
                    │  Provider别名        │
                    │  dotted path set     │
                    │  [personas] 配置     │
                    └──────────┬──────────┘
                               │
                    ┌──────────┴──────────┐
                    │  shadow-log          │
                    │  (957行 8测试)       │
                    │  record!宏+JSONL     │
                    │  broadcast+Observer   │
                    └─────────────────────┘
```

## 模块完成度

| Crate | 代码行数 | 测试数 | 完成度 | 状态 |
|-------|---------|--------|--------|------|
| shadow-core | 2,097 | 39 | 100% | 6 Trait + Attributable + AutonomyLevel |
| shadow-config | 1,039 | 5 | 100% | TOML + 密钥加密 + 别名 + personas |
| shadow-log | 957 | 8 | 100% | record! + JSONL + broadcast + Observer |
| shadow-providers | 3,706 | 68 | 80% | OpenAI + Anthropic + 3层架构 (缺 Gemini) |
| shadow-memory | 1,726 | 53 | 100% | None/MD/SQLite + Embedding + Vector |
| shadow-runtime | 14,254 | 330 | 85% | Agent + Prompt(10) + Tools(26) + Security |
| shadow-proxy | 2,452 | 33 | 100% | A2A + ACP + Registry + 3传输模式 |
| shadow-tui | 2,720 | 65 | 80% | Chat + Config + Memory (feature分支) |
| shadow-channels | 6 | 0 | 0% | 仅骨架 |
| shadow-gateway | 6 | 0 | 0% | 仅骨架 |
| shadow-plugins | 6 | 0 | 0% | 仅骨架 |
| 其他 6个 | 36 | 0 | 0% | 仅骨架 |

## 功能维度

| 维度 | 完成度 | 详情 |
|------|--------|------|
| Agent 能力 | 100% | 循环检测+溢出恢复+并行+超时+审批+校验 |
| Prompt 工程 | 100% | 10项: Section/注入/缓存/截断/压缩/安全/Bootstrap/Payload/Guided/Persona |
| Tool 系统 | 85% | 26个工具 (vs ZeroClaw ~120) |
| Provider | 80% | 2家 + 3层架构 (缺 Gemini) |
| Memory | 100% | 3后端 + 语义搜索 + 混合检索 |
| Proxy | 100% | A2A + ACP + 注册发现 + 路由 |
| 安全 | 85% | 校验+黑名单+路径+SSRF+注入防护 |
| 日志 | 100% | record! + JSONL + broadcast + Observer |
| 配置 | 90% | TOML+加密+别名+personas (缺 Configurable宏) |

## vs ZeroClaw 对比 (30,806行 vs 608,951行)

### 已对齐
- 文件操作 9/9
- Memory 5/5
- Prompt 10/10
- Proxy 完整
- 安全 (校验+黑名单+路径+SSRF+注入)
- 日志 (record!+JSONL+broadcast+Observer)
- Provider 3层 (Router+Reliable+Provider)

### 主要差距
- Channel 集成 (0 vs 37种)
- MCP 支持 (0 vs 9工具)
- 浏览器工具 (0 vs 3)
- SOP 引擎 (0 vs 7)
- WASM 插件 (0 vs 完整)
- Gateway HTTP (0 vs 50+路由)
- 硬件支持 (0 vs STM32/RPi)
- Tauri 桌面 (0 vs 完整)
- i18n 多语言 (0 vs 完整)
- 生命周期 Hook (0 vs 14个)
- Configurable 宏 (0 vs 14属性)
- 50+ Provider family

## 下一步优先级

- **P0**: MCP 客户端 | Channel(Telegram) | TUI 合并 main
- **P1**: Gateway HTTP | 生命周期 Hook | Configurable 宏
- **P2**: 浏览器 | SOP | WASM 插件 | 硬件
