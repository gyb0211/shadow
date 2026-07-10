# Shadow 能力分析: Tools 工具系统

> 对比 ZeroClaw 与 Shadow 的工具系统

## 1. Tool Trait 对比

| 项目 | ZeroClaw | Shadow | 差距 |
|------|----------|--------|------|
| trait bound | Send + Sync + Attributable | Attributable (隐式含 Send+Sync) | 无 |
| name() | &str | &str | 无 |
| description() | &str | &str | 无 |
| parameters_schema() | Value | Value | 无 |
| execute() | Result<ToolResult> | Result<ToolResult> | 无 |
| spec() | 默认实现 | 默认实现 | 无 |
| timeout() | 无 (靠装饰器) | 无 | 无 |
| requires_approval() | 无 (在 ApprovalManager) | 无 | 无 |
| ToolResult::ok/err | 无 | 有 | Shadow 更好 |
| EPHEMERAL_WORKSPACE_WARNING | 有 | 无 | 缺失 |

结论: Tool trait 已对齐, 不需要修改。

## 2. ToolRegistry 对比

| 特性 | ZeroClaw (无Registry) | Shadow (有Registry) |
|------|---------------------|---------------------|
| 存储 | Vec<Box<dyn Tool>> | Vec<Box<dyn Tool>> (封装) |
| 查找 | find_tool() 模块函数 | registry.find() 方法 |
| 规格导出 | 手动遍历 | registry.specs() |
| 动态注册 | register_skill_tools() | registry.register()/extend() |
| excluded_tools | ToolDispatchContext 传递 | 无 |
| ActivatedToolSet | 独立 Mutex<HashMap> | 无 |

结论: Shadow Registry 更整洁, 缺 excluded_tools 和 MCP 激活集。

## 3. ZeroClaw 工具执行管道 vs Shadow

| 步骤 | ZeroClaw | Shadow (注释代码) | 差距 |
|------|----------|-----------------|------|
| 排除检查 | is_excluded_tool | 无 | 缺 |
| 工具查找 | find_tool + activated_tools | tools.find() | 简化版 |
| Observer 通知 | ToolCallStart + ToolCall | record_tool_event | 注释中有 |
| 凭证脱敏 | scrub_credentials (8类正则) | scrub_credentials (3正则, 注释) | 缺 |
| 超时控制 | 外部 tokio::time::timeout | t.timeout() (注释, trait无此方法) | 需适配 |
| 审批检查 | ApprovalManager | requires_approval() (注释, trait无此方法) | 需适配 |
| 并行执行 | join_all + cancel select | futures::join_all (注释) | 有框架 |
| HMAC 回执 | ReceiptGenerator | 无 | 缺 |
| I/O 日志 | 统一管道记录 | 无统一管道 | 缺 |

## 4. ZeroClaw 装饰器模式

| 装饰器 | 结构 | 作用 | Shadow 状态 |
|--------|------|------|------------|
| RateLimitedTool<T> | 泛型 wrapper | 速率限制, 仅 success 消耗预算 | 有源码但未注册 |
| PathGuardedTool<T> | 泛型 wrapper | 路径安全, 检查 args 中路径字段 | 有源码但未注册 |
| 组合顺序 | RateLimited(PathGuarded(Tool)) | 外到内 | default_tools 未使用 |

## 5. ZeroClaw 工具清单 (60+)

| 类别 | 工具 |
|------|------|
| 文件 | shell, file_read, file_write, file_edit, file_download, file_upload, file_upload_bundle, pdf_read |
| 搜索 | glob_search, content_search, discord_search, tool_search, web_search, web_fetch |
| 记忆 | memory_store, memory_recall, memory_forget, memory_export, memory_purge |
| 调度 | cron_add, cron_list, cron_remove, cron_run, cron_runs, cron_update, schedule |
| 浏览器 | browser, browser_delegate, browser_open, text_browser, screenshot |
| 网络 | http_request, web_fetch, web_search |
| 子代理 | spawn_subagent, delegate, send_message_to_peer, llm_task |
| CLI 集成 | claude_code, claude_code_runner, codex_cli, gemini_cli, opencode_cli |
| 云/集成 | cloud_ops, cloud_patterns, google_workspace, jira_tool, notion_tool, microsoft365, linkedin, composio |
| 技能 | skill_tool, skill_http, skill_manage, read_skill |
| SOP | sop_list, sop_execute, sop_approve, sop_advance, sop_status |
| 配置 | model_routing_config, model_switch, proxy_config |
| 会话 | sessions_list, sessions_history, sessions_current, sessions_send, session_delete, session_reset |
| 其他 | calculator, weather, canvas, poll, pushover, reaction, backup, escalate, ask_user, image_gen, image_info, knowledge, project_intel, pipeline, report_template, data_management |

## 6. Shadow 已有工具清单

| 文件 | 工具 | ToolKind | attribution.rs | 状态 |
|------|------|----------|----------------|------|
| shell.rs | ShellTool | Shell | 已注册 | 源码存在 |
| file_read.rs | FileReadTool | Plugin | 已注册 | 源码存在 |
| file_write.rs | FileWriteTool | Plugin | 已注册 | 源码存在 |
| file_edit.rs | FileEditTool | Plugin | 已注册 | 源码存在 |
| file_download.rs | FileDownloadTool | Plugin | 已注册 | 源码存在 |
| file_upload.rs | FileUploadTool | Plugin | 已注册 | 源码存在 |
| file_upload_bundle.rs | FileUploadBundleTool | Plugin | 已注册 | 源码存在 |
| backup_tool.rs | BackupTool | Plugin | 已注册 | 源码存在 |
| glob_search.rs | GlobSearchTool | Search | 已注册 | 源码存在 |
| content_search.rs | ContentSearchTool | Search | 已注册 | 源码存在 |
| git_ops.rs | GitOpsTool | Shell | 已注册 | 源码存在 |
| http_request.rs | HttpRequestTool | HttpRequest | 已注册 | 源码存在 |
| web_fetch.rs | WebFetchTool | FetchUrl | 已注册 | 源码存在 |
| web_search.rs | WebSearchTool | Search | 已注册 | 源码存在 |
| spawn_subagent.rs | SpawnSubagentTool | SpawnSubAgent | 已注册 | 源码存在 |
| cron_tool.rs | CronTool | Plugin | 已注册 | 源码存在 |
| memory_recall.rs | MemoryRecallTool | Memory | 已注册 | 空文件 |
| memory_store.rs | MemoryStoreTool | Memory | 已注册 | 空文件 |
| memory_forget.rs | MemoryForgetTool | Memory | 已注册 | 源码存在 |
| memory_purge.rs | MemoryPurgeTool | Memory | 已注册 | 源码存在 |
| memory_export.rs | MemoryExportTool | Memory | 已注册 | 源码存在 |
| skill_manage.rs | SkillManageTool/SkillListTool/SkillViewTool | - | 未注册 | 源码存在 |
| wrapper.rs | RateLimitedTool | Plugin | 已注册 | 源码存在 |
| wrapper.rs | PathGuardedTool | Plugin | 已注册 | 源码存在 |

注意: mod.rs 的 default_tools() 返回空 ToolRegistry, 以上工具均未装配。

## 7. Shadow 差距表

| # | 能力 | 需要做什么 | 优先级 | 为什么 |
|---|------|-----------|--------|--------|
| 1 | default_tools 装配 | 在 default_tools 中注册工具 | P0 | agent 无工具 = 无法工作 |
| 2 | 装饰器接入 | default_tools 中用 RateLimited(PathGuarded(tool)) 包装 | P0 | 安全防护 |
| 3 | memory_recall/store | 填充空文件 | P0 | LLM 需要记忆工具 |
| 4 | skill_manage 注册 | attribution.rs 追加 | P1 | 技能系统已实现但未注册 |
| 5 | scrub_credentials | 激活并扩充正则 | P1 | 凭证泄露风险 |
| 6 | excluded_tools | ToolRegistry 加字段 | P2 | 按会话禁用工具 |
| 7 | MCP 动态激活 | DeferredMcpToolSet + ActivatedToolSet | P3 | 延迟加载 |
| 8 | HMAC 回执 | ReceiptGenerator | P3 | 审计需求 |
| 9 | TurnEvent 推送 | event_tx | P3 | 前端进度 |
| 10 | 统一 I/O 日志 | 管道层 record! | P2 | 可观测性 |

## 不需要改的 (Shadow 已有优势)
- Tool trait 签名 (对齐)
- ToolRegistry 封装 (比 ZeroClaw 更整洁)
- ToolDispatcher (NativeToolDispatcher + XmlToolDispatcher, 比 ZeroClaw 更完善)
- tool_attribution! 宏 (已修复, 20 个工具集中注册)
- 工具源码 (大部分已存在, 只需装配)
