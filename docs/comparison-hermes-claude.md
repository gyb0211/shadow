# Shadow vs Hermes vs Claude Code 全面对比

> 生成时间: 2026-07-04
> Shadow 版本: 0.1.0 (23,887行, 486测试)
> Hermes 版本: 基于实际使用观察
> Claude Code: 基于 Anthropic 公开信息

## 一、总览

| 维度 | Shadow | Hermes Agent | Claude Code |
|------|--------|-------------|------------|
| 语言 | Rust | TypeScript (Node.js) | TypeScript |
| 代码量 | 23,887 行 | ~100,000+ 行 (估) | ~50,000+ 行 (估) |
| 架构 | Trait 驱动微内核 | 插件式 Agent | CLI Agent |
| 运行时 | tokio (Rust async) | Node.js | Node.js |
| 配置 | TOML (~/.shadow/) | YAML (~/.hermes/) | JSON (~/.claude/) |
| 二进制 | 单文件 (shadow) | 单文件 (hermes) | 单文件 (claude) |

## 二、核心 Agent 能力

| 能力 | Shadow | Hermes | Claude Code |
|------|--------|--------|-------------|
| 工具调用循环 | [完整] 多轮循环, max_iterations 可配 | [完整] 多轮循环 | [完整] 多轮循环 |
| 流式响应 | [完整] SSE 流式, BoxStream | [完整] 流式 + TUI 逐字 | [完整] 流式 |
| 多轮对话 | [完整] 内存历史 | [完整] 持久化 + 跨会话 | [完整] 上下文管理 |
| 上下文窗口管理 | [完整] token 预算 + 截断 | [完整] 历史截断 | [完整] 自动压缩 |
| 循环检测 | [完整] loop_detector | [无] | [无] |
| 上下文溢出恢复 | [完整] 自动恢复 | [无] | [无] |
| 工具并行执行 | [完整] futures::join_all | [完整] 并行 | [完整] 并行 |
| 工具超时 | [完整] tokio::time::timeout | [完整] 可配 | [完整] |
| 工具审批 | [完整] Supervised 模式 | [完整] 交互审批 | [完整] 交互审批 |
| 参数校验 | [完整] jsonschema 运行时校验 | [无] | [无] |
| 工具可视化 | [完整] CLI 实时输出 | [完整] TUI 可视化 | [完整] 终端可视化 |

## 三、Provider 能力

| 能力 | Shadow | Hermes | Claude Code |
|------|--------|--------|-------------|
| OpenAI 兼容 | [完整] function calling | [完整] 多 provider | [无] 仅 Claude |
| Anthropic 原生 | [完整] tool_use 格式 | [完整] 原生支持 | [完整] 自家 |
| 多 key 轮换 | [完整] KeyRotator trait | [完整] | [无] |
| 重试 + 退避 | [完整] ReliableModelProvider | [完整] | [无] |
| 速率限制 | [完整] 令牌桶限流 | [无] | [无] |
| Provider 降级 | [完整] fallback chain | [完整] fallback | [无] |
| 多模型路由 | [完整] hint 路由 + 默认 | [完整] 模型路由 | [无] |
| 模型列表 | [完整] list_models() | [完整] | [无] |
| MiniMax 支持 | [完整] OpenAI 兼容 | [完整] | [无] |
| Ollama 本地 | [完整] OpenAI 兼容 | [完整] | [无] |
| Gemini | [设计] 未实现 | [完整] | [无] |

## 四、Tool 能力

| 工具 | Shadow | Hermes | Claude Code |
|------|--------|--------|-------------|
| Shell 执行 | [完整] 黑名单 + 超时 + 工作目录 | [完整] 终端工具 | [完整] Bash |
| 文件读取 | [完整] 截断 + 行范围 | [完整] read_file | [完整] |
| 文件写入 | [完整] 原子写入 + append | [完整] write_file | [完整] |
| 文件搜索 | [完整] content_search + glob | [完整] search_files | [完整] |
| 记忆存储 | [完整] memory_store 工具 | [完整] memory 工具 | [无] |
| 记忆召回 | [完整] memory_recall 工具 | [完整] session_search | [无] |
| 技能管理 | [完整] skill_manage 工具 | [完整] skill_manage | [无] |
| HTTP 请求 | [完整] skill_http 工具 | [完整] web 工具 | [无] |
| 工具注册表 | [完整] 动态注册/注销 | [完整] 动态注册 | [固定] |
| 工具装饰器 | [完整] PathGuarded + RateLimited | [无] | [无] |
| MCP 支持 | [设计] 未实现 | [完整] native MCP | [无] |
| 浏览器工具 | [无] | [完整] browser 工具集 | [无] |
| 邮件工具 | [无] | [完整] himalaya | [无] |
| GitHub 工具 | [无] | [完整] gh CLI | [无] |

## 五、Memory 能力

| 能力 | Shadow | Hermes | Claude Code |
|------|--------|--------|-------------|
| None 后端 | [完整] | [无] | [无] |
| Markdown 后端 | [完整] | [无] | [无] |
| SQLite 后端 | [完整] FTS5 trigram | [完整] SQLite FTS5 | [无] |
| 语义搜索 | [完整] embedding + 混合 merge | [无] | [无] |
| Embedding Provider | [完整] OpenAI 兼容 | [无] | [无] |
| 向量检索 | [完整] cosine + hybrid_merge | [无] | [无] |
| 记忆策略 | [完整] MemoryStrategy trait | [无] | [无] |
| 对话持久化 | [完整] JSONL + SQLite | [完整] SQLite | [完整] |
| 跨会话搜索 | [完整] session_search | [完整] session_search | [无] |
| 三层记忆 | [设计] 设计文档 | [完整] user + memory + skills | [无] |
| 记忆巩固 | [设计] 设计文档 | [完整] 自动巩固 | [无] |
| 时间衰减 | [设计] 设计文档 | [无] | [无] |
| 重要性评分 | [设计] 设计文档 | [无] | [无] |

## 六、Proxy / 多 Agent 能力

| 能力 | Shadow | Hermes | Claude Code |
|------|--------|--------|-------------|
| 进程内 delegate | [完整] LocalAgent | [完整] delegate_task | [无] |
| ACP 子进程 | [完整] AcpClient (claude/codex) | [完整] ACP 协议 | [完整] ACP |
| A2A 远程 | [完整] A2aClient HTTP JSON-RPC | [无] | [无] |
| Agent 注册/发现 | [完整] AgentRegistry | [无] | [无] |
| 任务路由 | [完整] 按名/能力路由 | [无] | [无] |
| HTTP Proxy Server | [完整] axum RESTful | [无] | [无] |
| stdio JSON-RPC | [完整] StdioTransport | [无] | [无] |
| 自动发现 | [完整] PATH + 端口 + 配置 | [无] | [无] |
| ProxyTool | [完整] impl Tool | [无] | [无] |
| 多 Agent 配置 | [完整] config.toml | [无] | [无] |
| 子代理并行 | [无] | [完整] delegate_task 批量 | [无] |
| 子代理隔离 | [无] | [完整] 独立会话 + 终端 | [无] |

## 七、TUI 能力

| 能力 | Shadow | Hermes | Claude Code |
|------|--------|--------|-------------|
| TUI 框架 | [完整] ratatui | [完整] 自定义 TUI | [无] CLI |
| 聊天界面 | [完整] 消息列表 + 输入框 | [完整] | [无] |
| 配置界面 | [完整] 配置编辑视图 | [完整] | [无] |
| 记忆界面 | [完整] 记忆浏览视图 | [无] | [无] |
| 主题 | [完整] 暗色/亮色自动检测 | [完整] | [无] |
| 命令面板 | [完整] 命令面板 | [无] | [无] |
| 状态栏 | [完整] 底部固定 | [完整] | [无] |
| 滚动消息区 | [完整] 可滚动 + 固定输入 | [完整] | [无] |

## 八、Skills 能力

| 能力 | Shadow | Hermes | Claude Code |
|------|--------|--------|-------------|
| SKILL.md 解析 | [完整] 前置元数据 | [完整] | [完整] |
| 技能加载 | [完整] skill_tool | [完整] skill_view | [完整] |
| 技能管理 | [完整] skill_manage 工具 | [完整] skill_manage | [无] |
| 技能审查 | [完整] review.rs | [无] | [无] |
| 技能改进 | [完整] improver.rs | [无] | [无] |
| HTTP 技能工具 | [完整] skill_http.rs | [无] | [无] |
| 自进化反馈 | [设计] 设计文档 | [完整] 自动保存技能 | [无] |
| 技能搜索 | [无] | [完整] skills_list | [完整] |
| 技能分类 | [无] | [完整] category | [无] |
| 技能脚本 | [无] | [完整] scripts/ | [完整] |
| 技能模板 | [无] | [完整] templates/ | [无] |

## 九、日志与可观测性

| 能力 | Shadow | Hermes | Claude Code |
|------|--------|--------|-------------|
| record! 宏 | [完整] 结构化日志 | [无] | [无] |
| JSONL 持久化 | [完整] 滚动轮转 | [完整] | [无] |
| 广播 channel | [完整] broadcast | [无] | [无] |
| Observer trait | [完整] 事件 + 指标 | [无] | [无] |
| LogObserver 桥接 | [完整] 事件转发 | [无] | [无] |
| 日志读取 | [完整] reader + 分页 | [无] | [无] |
| Schema 迁移 | [完整] v1 -> v2 | [无] | [无] |

## 十、配置能力

| 能力 | Shadow | Hermes | Claude Code |
|------|--------|--------|-------------|
| TOML 配置 | [完整] 多层嵌套 | [完整] YAML | [完整] JSON |
| 环境变量覆盖 | [完整] SHADOW_* | [完整] | [完整] |
| 密钥加密 | [完整] ChaCha20-Poly1305 | [无] | [无] |
| config set | [完整] dotted path 解析 | [完整] | [无] |
| Provider 别名 | [完整] family.alias | [无] | [无] |
| 配置迁移 | [完整] 版本检测 | [无] | [无] |

## 十一、安全能力

| 能力 | Shadow | Hermes | Claude Code |
|------|--------|--------|-------------|
| 自治等级 | [完整] ReadOnly/Supervised/Auto | [完整] | [完整] |
| Shell 黑名单 | [完整] 危险命令拦截 | [无] | [无] |
| 工具审批 | [完整] requires_approval() | [完整] | [完整] |
| 参数校验 | [完整] jsonschema | [无] | [无] |
| 路径守卫 | [完整] PathGuardedTool | [无] | [无] |
| 限流装饰器 | [完整] RateLimitedTool | [无] | [无] |

## 十二、Channel / 外部集成

| 能力 | Shadow | Hermes | Claude Code |
|------|--------|--------|-------------|
| CLI 通道 | [完整] stdin/stdout | [完整] | [完整] |
| Telegram | [无] | [完整] | [无] |
| Discord | [无] | [完整] | [无] |
| Slack | [无] | [完整] | [无] |
| 飞书 | [无] | [完整] | [无] |
| Email | [无] | [完整] | [无] |
| Webhook | [无] | [完整] | [无] |
| Matrix | [无] | [完整] | [无] |

## 十三、基础设施

| 能力 | Shadow | Hermes | Claude Code |
|------|--------|--------|-------------|
| Cron 定时任务 | [完整] SQLite 持久化 | [完整] cronjob 工具 | [无] |
| Session 管理 | [完整] SessionStore | [完整] session 持久化 | [完整] |
| Workspace 抽象 | [完整] 路径布局 | [完整] | [完整] |
| 插件系统 | [设计] 未实现 | [无] | [无] |
| WASM 插件 | [设计] 未实现 | [无] | [无] |
| Gateway HTTP | [设计] 未实现 | [无] | [无] |
| 交叉编译 | [完整] ARM 支持 | [无] | [无] |
| 双模式构建 | [完整] kernel-only | [无] | [无] |

## 十四、总结

### Shadow 的独特优势
1. Rust 性能 + 类型安全
2. 语义搜索记忆 (embedding + 混合检索) -- 独有
3. 参数校验框架 (jsonschema) -- 独有
4. 完整 Proxy 层 (A2A + ACP + 注册发现) -- 独有
5. 循环检测 + 上下文溢出恢复 -- 独有
6. 工具装饰器模式 (PathGuarded + RateLimited) -- 独有
7. 密钥加密存储 (ChaCha20-Poly1305) -- 独有
8. 双模式构建 (kernel-only) -- 独有

### Shadow 的主要差距
1. 无 Channel 集成 (Telegram/Discord/Slack 等) -- Hermes 有 30+
2. 无 MCP 支持 -- Hermes 有原生 MCP
3. 无浏览器工具 -- Hermes 有完整 browser 工具集
4. 无子代理隔离 -- Hermes 有 delegate_task 独立会话
5. 无 Web Gateway -- ZeroClaw 有完整 HTTP API
6. 无 WASM 插件 -- ZeroClaw 有 wasmtime 插件系统
7. 无三层记忆 -- Hermes 有 user + memory + skills
8. 无技能搜索/分类 -- Hermes 有 skills_list + category
9. 无流式工具输出 -- Hermes 工具可流式返回中间进度
10. TUI 未集成到主分支 -- feature/shadow-tui 分支

### 定位差异
- Shadow: 高性能 Rust Agent 运行时, 适合嵌入式/边缘场景
- Hermes: 全功能 Agent, 30+ 渠道集成, 适合个人助手
- Claude Code: 专注代码生成, 深度集成 Anthropic 生态
