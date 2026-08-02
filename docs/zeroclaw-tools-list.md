# ZeroClaw 工具清单与 Shadow 实现计划

## 总览

ZeroClaw 共约 95+ 个工具，分为 24 个类别。Shadow 按优先级逐步实现。

标记说明：
- ✅ 已实现
- ❌ 未实现

---

## 1. 文件系统工具

| 工具 | 作用 | Shadow 状态 |
|------|------|-------------|
| file_read | 读取文本文件（行号+分页+二进制检测） | ✅ |
| file_write | 创建或覆盖文件（自动创建父目录） | ✅ |
| file_edit | 精确字符串查找替换 | ✅ |
| glob_search | 文件名 glob 模式搜索 | ✅ |
| content_search | 文件内容正则搜索 | ✅ |
| file_download | 从 URL 下载文件到本地 | ✅ |
| file_upload | 上传文件到外部服务 | ❌ |
| file_upload_bundle | 批量上传多个文件 | ❌ |
| pdf_read | 读取 PDF 文件内容 | ✅ |

## 2. 命令执行工具

| 工具 | 作用 | Shadow 状态 |
|------|------|-------------|
| shell | 执行 shell 命令（超时+环境过滤） | ✅ |
| git_operations | Git 操作（clone/commit/push/pull 等） | ✅ |
| backup_tool | 备份文件/目录 | ❌ |

## 3. 网络与 Web 工具

| 工具 | 作用 | Shadow 状态 |
|------|------|-------------|
| http_request | 发送自定义 HTTP 请求 | ❌ |
| web_search | 网络搜索 | ❌ |
| web_fetch | 抓取网页内容 | ❌ |
| text_browser | 文本浏览器（解析网页） | ❌ |
| screenshot | 网页截图 | ❌ |

## 4. 浏览器自动化工具

| 工具 | 作用 | Shadow 状态 |
|------|------|-------------|
| browser | 完整浏览器控制（Playwright） | ❌ |
| browser_delegate | 委托浏览器任务给子进程 | ❌ |
| browser_open | 打开 URL | ❌ |

## 5. 记忆系统工具

| 工具 | 作用 | Shadow 状态 |
|------|------|-------------|
| memory_store | 存储记忆 | ❌ |
| memory_recall | 检索记忆 | ❌ |
| memory_forget | 删除单条记忆 | ❌ |
| memory_purge | 清空记忆 | ❌ |
| memory_export | 导出记忆 | ❌ |

## 6. 定时任务工具

| 工具 | 作用 | Shadow 状态 |
|------|------|-------------|
| cron_add | 添加定时任务 | ❌ |
| cron_list | 列出定时任务 | ❌ |
| cron_remove | 删除定时任务 | ❌ |
| cron_run | 手动执行定时任务 | ❌ |
| cron_runs | 查看执行历史 | ❌ |
| cron_update | 更新定时任务 | ❌ |

## 7. 子代理与委派工具

| 工具 | 作用 | Shadow 状态 |
|------|------|-------------|
| spawn_subagent | 启动子代理执行独立任务 | ❌ |
| delegate | 委派任务给其他 agent | ❌ |
| llm_task | 调用 LLM 执行独立任务 | ✅ |
| claude_code_runner | 运行 Claude Code 子进程 | ❌ |

## 8. 外部 AI 工具集成

| 工具 | 作用 | Shadow 状态 |
|------|------|-------------|
| claude_code | 调用 Claude Code CLI | ❌ |
| codex_cli | 调用 OpenAI Codex CLI | ❌ |
| gemini_cli | 调用 Gemini CLI | ❌ |
| opencode_cli | 调用 OpenCode CLI | ❌ |
| cli_discovery | 自动发现系统中的 AI CLI 工具 | ❌ |

## 9. 项目管理工具

| 工具 | 作用 | Shadow 状态 |
|------|------|-------------|
| jira_tool | Jira 交互（查/搜/评论/创建工单） | ✅ |
| project_intel | 项目智能分析（状态报告/风险检测） | ❌ |
| report_template | 报告模板引擎 | ❌ |

## 10. 邮件工具

| 工具 | 作用 | Shadow 状态 |
|------|------|-------------|
| email_read | 读取邮件 | ❌ |
| email_search | 搜索邮件 | ❌ |

## 11. 第三方平台集成

| 工具 | 作用 | Shadow 状态 |
|------|------|-------------|
| google_workspace | Google 日历/文档/表格/邮件 | ❌ |
| microsoft365 | Microsoft 365 集成 | ❌ |
| notion | Notion 笔记操作 | ❌ |
| linkedin | LinkedIn 操作 | ❌ |
| discord_search | 搜索 Discord 消息历史 | ❌ |
| composio | Composio 平台集成 | ❌ |
| pushover | Pushover 推送通知 | ❌ |
| weather | 天气查询 | ❌ |

## 12. 模型与路由工具

| 工具 | 作用 | Shadow 状态 |
|------|------|-------------|
| model_switch | 运行时切换模型 | ✅ |
| model_routing_config | 配置模型路由规则 | ❌ |
| tool_search | 搜索可用工具 | ❌ |

## 13. MCP 协议工具

| 工具 | 作用 | Shadow 状态 |
|------|------|-------------|
| mcp_tool | 调用 MCP 服务器工具 | ❌ |
| mcp_prompts_tool | 获取 MCP prompts | ❌ |
| mcp_resources_tool | 获取 MCP resources | ❌ |
| mcp_client | MCP 客户端管理 | ❌ |

## 14. 技能系统工具

| 工具 | 作用 | Shadow 状态 |
|------|------|-------------|
| read_skill | 读取技能文档 | ❌ |
| skill_manage | 管理技能（增删改查） | ❌ |
| skill_http | HTTP 技能调用 | ❌ |
| skill_tool | 执行技能中的工具 | ❌ |

## 15. SOP 标准操作流程工具

| 工具 | 作用 | Shadow 状态 |
|------|------|-------------|
| sop_list | 列出可用 SOP | ❌ |
| sop_execute | 执行 SOP | ❌ |
| sop_advance | 推进 SOP 到下一步 | ❌ |
| sop_approve | 审批 SOP 步骤 | ❌ |
| sop_status | 查看 SOP 执行状态 | ❌ |

## 16. 安全与运维工具

| 工具 | 作用 | Shadow 状态 |
|------|------|-------------|
| security_ops | 安全运营操作 | ❌ |
| cloud_ops | 云平台运维操作 | ❌ |
| cloud_patterns | 云架构模式检查 | ❌ |
| data_management | 数据管理操作 | ❌ |
| proxy_config | 代理配置管理 | ❌ |

## 17. 渠道与通知工具

| 工具 | 作用 | Shadow 状态 |
|------|------|-------------|
| ask_user | 向用户提问等待回复 | ❌ |
| send_message_to_peer | 向 peer 发送消息 | ❌ |
| channel_room | 渠道房间管理 | ❌ |
| reaction | 添加表情反应 | ❌ |

## 18. 图像与媒体工具

| 工具 | 作用 | Shadow 状态 |
|------|------|-------------|
| image_gen | AI 图像生成 | ❌ |
| image_info | 图像信息分析 | ❌ |
| canvas | 画布操作（存储/检索内容） | ❌ |

## 19. 会话管理工具

| 工具 | 作用 | Shadow 状态 |
|------|------|-------------|
| sessions | 会话管理（切换/列出/清除） | ❌ |

## 20. 用户交互工具

| 工具 | 作用 | Shadow 状态 |
|------|------|-------------|
| escalate_to_human | 升级到人工处理 | ❌ |
| poll | 发起投票 | ❌ |

## 21. 机器人硬件工具 (robot-kit)

| 工具 | 作用 | Shadow 状态 |
|------|------|-------------|
| drive | 机器人移动控制 | ❌ |
| look | 摄像头视觉识别 | ❌ |
| listen | 麦克风语音识别 | ❌ |
| speak | 扬声器语音播报 | ❌ |
| sense | 传感器数据读取 | ❌ |
| emote | 表情/动作输出 | ❌ |

## 22. 工具包装器

| 工具 | 作用 | Shadow 状态 |
|------|------|-------------|
| path_guarded_tool | 路径安全包装器 | ❌ |
| rate_limited_tool | 限流包装器 | ❌ |

## 23. 其他工具

| 工具 | 作用 | Shadow 状态 |
|------|------|-------------|
| calculator | 数学计算器 | ❌ |
| knowledge_tool | 知识库查询 | ❌ |
| verifiable_intent | 可验证意图（安全审计） | ❌ |

## 24. 硬件工具

| 工具 | 作用 | Shadow 状态 |
|------|------|-------------|
| hardware_board_info | 硬件板信息 | ❌ |
| hardware_memory_map | 硬件内存映射 | ❌ |
| hardware_memory_read | 硬件内存读取 | ❌ |

---

## Shadow 实现进度

已实现：10/95+
- ✅ shell
- ✅ file_read
- ✅ file_write
- ✅ file_edit
- ✅ glob_search
- ✅ content_search
- ✅ file_download
- ✅ pdf_read
- ✅ jira_tool
- ✅ llm_task
- ✅ model_switch