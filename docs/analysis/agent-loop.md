# Shadow 能力分析: Agent Loop

> 对比 ZeroClaw 与 Shadow 的 agent loop 设计

## 1. ZeroClaw Agent struct 字段清单

| 字段 | 类型 | 说明 |
|------|------|------|
| model_provider | Box<dyn ModelProvider> | LLM 推理后端 |
| tools | Vec<Box<dyn Tool>> | 可用工具列表 |
| memory | Arc<dyn Memory> | 持久化记忆后端 |
| observer | Arc<dyn Observer> | 可观测性 sink |
| prompt_builder | SystemPromptBuilder | 系统提示构建器 |
| tool_dispatcher | Box<dyn ToolDispatcher> | 工具调用协议分发器 |
| memory_strategy | Arc<dyn MemoryStrategy> | 记忆召回策略 |
| history | Vec<ConversationMessage> | 对话历史(内存中) |
| config | AliasedAgentConfig | agent 配置快照 |
| model_name | String | 当前模型名 |
| model_provider_name | String | 当前 provider 名 |
| workspace_dir | PathBuf | 沙箱根 |
| agent_workspace_dir | PathBuf | persona 工作区 |
| activated_tools | Option<Arc<Mutex<ActivatedToolSet>>> | MCP deferred loading |
| hook_runner | Option<Arc<HookRunner>> | 工具审计钩子 |
| approval_manager | Option<Arc<ApprovalManager>> | HITL 审批 |
| channel_handles | AgentChannelHandles | channel 后通道 |
| image_cache | LocalImageCache | per-session 图片缓存 |
| channel_name | String | 观测标签 |
| approval_route | Option<ApprovalRoute> | 跨渠道审批路由 |

## 2. ZeroClaw agent loop turn 循环步骤

```
for iteration in 0..max_iterations:
  1. Steering drain       -- 消费 mid-turn 消息注入
  2. 取消检查              -- cancellation_token.is_cancelled()
  3. 共享预算检查           -- shared_budget AtomicUsize == 0?
  4. preflight_history     -- 清理 orphaned tool messages + 归并 system
  5. Token 预算裁剪        -- iteration==0 时 trim_to_recent_turns
  6. Model switch 检查     -- 检测模型切换请求
  7. 构建 tool specs       -- native 或 prompt-guided
  8. Vision provider 检查  -- 多模态图片路由
  9. prepare_messages      -- inline base64 图片等
  10. announce_llm_request -- ObserverEvent::LlmRequest
  11. budget 检查          -- cost budget 超限 bail
  12. call_provider        -- LLM 调用 (流式/非流式)
  13. interpret_response   -- 解析文本 + 工具调用
  14. 无工具调用 -> 最终答案 -> return Ok(text)
  15. 有工具调用:
      a. prepare_tool_calls -- 去重 + approval gate
      b. should_parallel?  -- 并行还是串行
      c. execute_tools      -- 执行
      d. record_outcomes    -- 记录结果
      e. loop_detection     -- 连续相同输出检测
      f. append_to_history  -- 追加 assistant + tool 消息
  16. 继续循环
```

终止条件:
- LLM 返回无工具调用 -> 最终答案
- max_iterations 耗尽 -> 请求无工具摘要
- cancellation_token 触发 -> Err(ToolLoopCancelled)
- shared_budget == 0 -> break
- 连续相同输出 -> abort
- Model switch 请求 -> Err(ModelSwitchRequested)

## 3. ZeroClaw 工具执行管道 (execute_one_tool)

| 步骤 | 动作 | Observer 事件 |
|------|------|--------------|
| 1 | 序列化 args -> JSON | - |
| 2 | 通知观察者开始 | ToolCallStart |
| 3 | 排除列表检查 | - |
| 4 | find_tool (静态注册表) | - |
| 5 | activated_tools (MCP 动态) | - |
| 6 | 未找到 -> 错误结果 | ToolCall{success:false} |
| 7 | 创建 tracing span | - |
| 8 | record!(Invoke) 日志 | - |
| 9 | event_tx.send(ToolCall pending) | - |
| 10 | tool.execute(args) + cancel select | - |
| 11 | scrub_credentials 脱敏 | - |
| 12 | 成功 -> record!(Complete) | ToolCall{success:true} |
| 12 | 失败 -> record!(Fail) | ToolCall{success:false} |
| 13 | event_tx.send(ToolResult terminal) | - |

## 4. ZeroClaw 历史管理策略

| 策略 | 函数 | 时机 | 做法 |
|------|------|------|------|
| Orphan 清理 | remove_orphaned_tool_messages | 每轮 preflight | 删除无配对的 tool_result |
| System 归并 | normalize_system_messages | 每轮 preflight | 多条 system 合为一条 |
| Token 预算裁剪 | trim_to_recent_turns | iteration==0 | 按 turn 分组丢弃最旧, 不切割 turn |
| 溢出恢复 | try_recover_context_overflow | provider 错误时 | 压到 2/3 + breadcrumb |
| 工具结果截断 | truncate_tool_result | 工具输出过长 | head(2/3) + tail(1/3) |
| Token 估算 | estimate_history_tokens | 每轮 | content.len()/4 + 4/msg |

## 5. Observer 事件触发点清单

| 事件 | 触发位置 | 时机 |
|------|---------|------|
| AgentStart | run() 初始化后 | turn 开始 |
| AgentEnd | TurnGuard::Drop (RAII) | turn 结束 |
| LlmRequest | announce_llm_request() | 每次 provider 调用前 |
| LlmResponse | interpret_chat_response 后 | provider 返回后 |
| ToolCallStart | execute_one_tool step 2 | 工具执行前 |
| ToolCall | execute_one_tool step 12 | 工具执行后 |
| MemoryRecall | build_context() 中 | 记忆检索后 |
| MemoryStore | run() auto-save | 记忆存储后 |
| HistoryTrimmed | token budget 裁剪时 | 历史裁剪后 |
| TurnComplete | run() 最终输出后 | turn 完成 |

## 6. ZeroClaw Session 持久化时机

| 时机 | 说明 |
|------|------|
| Load | interactive REPL 启动时从 JSONL 加载 |
| Save (one-shot) | turn 完成后保存 |
| Save (interactive) | 每轮完成后保存 |
| Save (/clear /new) | 命令后清空+保存 |

## 7. ZeroClaw 错误处理和重试策略

| 错误类型 | 策略 |
|----------|------|
| Context overflow | 压到 2/3 + trim + breadcrumb + continue |
| Malformed tool protocol | 最多 2 次重试, 注入错误反馈 |
| Model switch | 重建 provider + continue + 新 AgentStart |
| Loop detection | Warning -> Block -> Break 三级 |
| Budget 超限 | bail |
| Stream 中断 (有输出) | 持久化 partial, 不重试 |
| Stream 取消 (有输出) | 链接 ToolLoopCancelled |
| Max iterations | 请求无工具摘要, 失败则 bail |

## 8. ZeroClaw 流式响应处理

```
should_stream = on_delta.is_some() || event_tx.is_some()
  && provider.supports_streaming()
  && (tools.is_none() || provider.supports_streaming_tool_events())

consume_provider_streaming_response:
  loop { stream.next() }:
    Final -> break
    Usage -> 记录 token
    ToolCall -> 累积
    TextDelta -> 区分 reasoning vs text, 经 guard + stripper 前向

  cancel: tokio::select! { token, stream }
  有输出 -> Err(StreamCancelledAfterOutput{partial})
  无输出 -> Err(ToolLoopCancelled)

  非流式 fallback: 流式失败且无输出 -> chat()
```

## 9. Shadow 注释代码中已有的设计

| 组件 | 位置 | 状态 |
|------|------|------|
| Agent struct | agent.rs:110-128 | 注释, 10 个字段 |
| AgentBuilder | agent.rs:788-919 | 注释, builder 模式 |
| chat_with_stream | agent.rs:178-621 | 注释, 完整 turn 循环 |
| execute_tool_call | agent.rs:650-731 | 注释, 含审批/超时/回调 |
| LoopDetector | loop_detector.rs:1-259 | 完整实现 (独立文件) |
| estimate_tokens | agent.rs:925-930 | 注释 |
| trim_history | agent.rs:935-945 | 注释 |
| try_recover_context_overflow | agent.rs:952-979 | 注释 |
| scrub_credentials | agent.rs:998-1016 | 注释, 3 个正则 |
| chars_preview | agent.rs:982-988 | 注释 |
| clear_history / load_history | agent.rs:734-775 | 注释 |
| AgentConfig | agent.rs:68-107 | 注释 |
| StreamDelta enum | agent.rs:52-57 | 注释 |
| ToolEventCallback trait | agent.rs:45-48 | 注释 |
| session 持久化 | agent.rs:554-580 | 注释 |
| memory_strategy 集成 | agent.rs:214-231, 585-593 | 注释 |
| 并行/串行执行 | agent.rs:350-530 | 注释, 含 join_all |
| ToolDispatcher | dispatcher.rs | 已实现, NativeToolDispatcher + XmlToolDispatcher |
| ObserverEvent | observer.rs | 已实现, 完整事件枚举 |

## 10. Shadow 差距表 + 需要做什么 + 为什么

| # | 能力 | ZeroClaw | Shadow | 需要做什么 | 为什么 |
|---|------|----------|--------|-----------|--------|
| 1 | Agent 主体 | 30+ 字段, 活跃 | 全注释, 10 字段 | 取消注释 agent.rs, 适配当前 trait 签名 | agent loop 是核心功能, 无 agent = 无法对话 |
| 2 | Turn 循环 | 22 个 step module | 注释中有完整循环 | 取消注释 chat_with_stream, 适配 ChatRequest/ChatResponse | 已有设计框架, 不需要从头设计 |
| 3 | 工具执行 | execute_one_tool 910 行 | execute_tool_call 注释中 | 取消注释, 移除 requires_approval/timeout (trait 中没有) | 需要适配当前 Tool trait |
| 4 | 历史管理 | trim_to_recent_turns + orphan + normalize | 注释中 trim_history 简单版 | 取消注释 + 补 orphan 清理 | 当前 trim 只按条数删, 不考虑 tool 消息配对 |
| 5 | Observer 触发 | 全链路 | ObserverEvent 已定义但无触发点 | 在 agent loop 中加 record_event 调用 | 事件定义了但不触发 = observer 空转 |
| 6 | Session | JSONL, 多时机保存 | 注释中 append_message | 取消注释, 适配 SessionStore trait | 会话持久化是基础能力 |
| 7 | 流式 | consume_provider_streaming_response | 注释中 StreamDelta 但无 SSE 消费 | P1: 先支持非流式, P2: 加流式 | 流式非 P0, 先跑通非流式 |
| 8 | 错误恢复 | context overflow + malformed + model switch | 注释中 try_recover_context_overflow | 取消注释 + 补 malformed retry | context overflow 恢复是 P0 |
| 9 | 循环检测 | LoopDetector + 相同输出 abort | LoopDetector 已实现! | 在 agent loop 中调用 loop_detector.record() | 已有实现, 只需接入 |
| 10 | 工具注册 | default_tools 注册 6 核心 | default_tools() 返回空 | 在 default_tools 中注册工具 | agent 无工具 = 无法完成任务 |
| 11 | ToolDispatcher | NativeToolDispatcher | NativeToolDispatcher + XmlToolDispatcher | 已对齐, 无需修改 | Shadow 更完善 |
| 12 | ToolEventCallback | TurnEvent + event_tx | ToolEventCallback trait (注释) | 取消注释, 或改为 channel 方式 | CLI 需要工具执行进度反馈 |
