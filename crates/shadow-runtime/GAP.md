# shadow-runtime 差距分析 -- 对照 ZeroClaw

## 当前状态 (Shadow)
- Agent: chat() + tool call loop (max_iterations 可配)
- 3 工具: Shell(黑名单+超时) / FileRead / FileWrite(原子写入)
- 工具超时 (tokio::time::timeout)
- Supervised 审批 (requires_approval)
- 上下文窗口管理 (max_history 截断)
- 自定义 system prompt
- ToolEventCallback 回调
- LogObserver
- 共 ~1100 行

## ZeroClaw 对应 (zeroclaw-runtime: 157684行, 246文件)
- Agent (30+字段) + Turn Pipeline (24步, 24文件)
- Security: Sandbox(5后端) + EstopManager + LeakDetector + PromptGuard + IAM + Audit + IngressPolicy + OTP + ExternalContent
- SOP: 5 执行模式 + 审批子系统 (resolve_gate) + SopRunStore (SQLite)
- Cron: 调度器 + SQLite 持久化 + 安全验证
- Skills: 四源合并 + 审计 + 缓存 + 自改进 + SkillForge
- Observability: 6 后端 + MultiObserver + TeeObserver + FlushGuard
- Tools: 15+ 运行时工具 (Delegate/SpawnSubagent/Cron/SOP/...)
- RPC: JSON-RPC + Unix socket + WSS
- daemon: 守护进程 + Reload + supervisor
- control_plane: TaskRegistry + reaper 回收
- hooks: HookRunner + 2 builtin
- heartbeat: 心跳引擎
- verifiable_intent: SD-JWT 分层凭证
- tunnel: 7 后端
- 循环检测器: 滑动窗口 + 3 模式
- 历史管理: 4 层 (预检/裁剪/截断/孤儿清理)
- 记忆策略: MemoryStrategy trait
- 提示词: PromptSection trait (9 section)
- dispatcher: ToolDispatcher (Native/XML)

## 缺失项
| 功能 | 严重度 | ZeroClaw 实现 | Shadow 状态 |
|------|--------|--------------|-------------|
| 历史持久化 | P0 | JSONL/SQLite | 仅内存 |
| 循环检测器 | P0 | 3 模式滑动窗口 | 缺失 |
| ToolDispatcher | P1 | Native/XML 双协议 | 缺失 |
| PromptSection | P1 | 9 section 可插拔 | 硬编码 |
| MemoryStrategy | P1 | load/consolidate/governance | 缺失 |
| Skills 系统 | P1 | 四源+审计+自改进 | 缺失 |
| Cron 调度 | P1 | SQLite+安全验证 | 缺失 |
| SOP 引擎 | P2 | 5模式+审批 | 缺失 |
| Sandbox | P1 | 5 后端 | 缺失 |
| EstopManager | P1 | 分级 kill | 缺失 |
| LeakDetector | P1 | 8类正则+熵 | 缺失 |
| PromptGuard | P1 | 6类检测 | 缺失 |
| RPC | P2 | JSON-RPC | 缺失 |
| daemon | P2 | 守护进程 | 缺失 |
| control_plane | P2 | TaskRegistry | 缺失 |
| hooks | P1 | HookRunner | 缺失 |
| 多 Observer 后端 | P2 | 6后端 | 1个 |
| DelegateTool | P2 | 多代理委托 | 缺失 |
| 流式响应 | P0 | stream_chat | 缺失 |

## 开发建议
1. P0: 历史持久化 (JSONL)
2. P0: 循环检测器 (滑动窗口)
3. P0: 流式响应
4. P1: Skills 系统 (SKILL.md 解析 + 目录加载)
5. P1: Cron 调度 (SQLite 持久化)
6. P1: ToolDispatcher (Native/XML)
7. P1: PromptSection trait
8. P1: MemoryStrategy
9. P1: LeakDetector + PromptGuard
10. P2: SOP / Sandbox / RPC / daemon
