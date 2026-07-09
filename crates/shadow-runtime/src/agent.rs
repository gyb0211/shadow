// //! Agent -- 核心代理, 持有 provider/tool/memory/observer
// //!
// //! 借鉴 ZeroClaw 的 Agent 设计, 但大幅精简:
// //! - ZeroClaw: 30+ 字段, 7874 行
// //! - Shadow: ~10 字段, 目标 ~300 行
// //!
// //! P0 增强: 循环检测 + Token 预算 + 上下文溢出恢复
// 
// mod loop_detector;
// 
// pub use loop_detector::{LoopDetectionResult, LoopDetector};
// 
// use anyhow::Result;
// use futures::StreamExt;
// use futures::stream::BoxStream;
// use parking_lot::Mutex;
// use shadow_core::{
//     Attributable, AutonomyLevel, StreamChunk, ChatMessage, ChatRequest, ChatResponse, Memory,
//     MemoryStrategy, ModelProvider, Observer, ObserverEvent, Role, SessionStore, TokenUsage, ToolCall,
//     ToolResult,
// };
// use shadow_log::Action;
// use std::sync::Arc;
// 
// use crate::tools::ToolRegistry;
// use shadow_memory::format_entries;
// 
// /// 工具调用最大循环次数 -- 默认值, 可被 AgentConfig.max_iterations 覆盖
// const DEFAULT_MAX_ITERATIONS: usize = 10;
// 
// /// 对话历史最大条数 -- 默认值, 可被 AgentConfig.max_history 覆盖
// const DEFAULT_MAX_HISTORY: usize = 50;
// 
// /// 默认 system prompt
// const DEFAULT_SYSTEM_PROMPT: &str = "你是一个有用的 AI 助手. 你可以使用工具来完成任务.";
// 
// /// 默认上下文 token 预算 (0 = 不限制)
// pub const DEFAULT_CONTEXT_TOKEN_BUDGET: usize = 100_000;
// 
// /// 工具事件回调 trait -- CLI 通过此回调接收工具执行事件通知
// ///
// /// 两个 &str 参数: (事件类型, 详情)
// /// 事件类型: "tool_start" / "tool_success" / "tool_error" / "tool_timeout" / "tool_approval_skipped"
// /// 详情: 工具名称或错误信息
// pub trait ToolEventCallback: Fn(&str, &str) + Send + Sync {}
// 
// // 为所有满足 Fn(&str, &str) + Send + Sync 的类型自动实现 ToolEventCallback
// impl<T> ToolEventCallback for T where T: Fn(&str, &str) + Send + Sync {}
// 
// /// 流式增量类型 -- 区分回答和思考
// #[derive(Debug, Clone)]
// pub enum StreamDelta {
//     /// 回答增量
//     Content(String),
//     /// 思考增量
//     Reasoning(String),
// }
// 
// /// 流式增量回调 trait -- TUI/CLI 通过此回调接收逐字增量
// ///
// /// 参数: StreamDelta (回答或思考的增量片段)
// pub trait StreamDeltaCallback: Fn(StreamDelta) + Send + Sync {}
// 
// // 为所有满足 Fn(StreamDelta) + Send + Sync 的类型自动实现 StreamDeltaCallback
// impl<T> StreamDeltaCallback for T where T: Fn(StreamDelta) + Send + Sync {}
// 
// /// Agent 配置
// #[derive(Debug, Clone)]
// pub struct AgentConfig {
//     pub alias: String,
//     pub model_provider_type: String,
//     pub model: String,
//     pub temperature: Option<f64>,
//     pub autonomy: AutonomyLevel,
//     pub workspace_dir: std::path::PathBuf,
//     /// 工具调用最大循环次数 (默认 10)
//     pub max_iterations: usize,
//     /// 对话历史最大条数 (超过时自动截断旧消息, 默认 50)
//     pub max_history: usize,
//     /// 自定义 system prompt (None 则使用默认)
//     pub system_prompt: Option<String>,
//     /// 上下文 token 预算 (0 = 不限制, 默认 100000)
//     pub context_token_budget: usize,
//     /// 是否启用对话后技能审查 (默认 false)
//     pub skill_review_enabled: bool,
//     /// 技能审查触发阈值 -- 工具调用次数达到此值才触发 (默认 5)
//     pub skill_review_nudge_threshold: usize,
// }
// 
// impl Default for AgentConfig {
//     fn default() -> Self {
//         Self {
//             alias: "default".to_string(),
//             model_provider_type: "openai".to_string(),
//             model: "gpt-4o-mini".to_string(),
//             temperature: Some(0.7),
//             autonomy: AutonomyLevel::default(),
//             workspace_dir: std::path::PathBuf::from("."),
//             max_iterations: DEFAULT_MAX_ITERATIONS,
//             max_history: DEFAULT_MAX_HISTORY,
//             system_prompt: None,
//             context_token_budget: DEFAULT_CONTEXT_TOKEN_BUDGET,
//             skill_review_enabled: false,
//             skill_review_nudge_threshold: 5,
//         }
//     }
// }
// 
// /// Agent -- 核心代理
// pub struct Agent {
//     pub alias: String,
//     pub provider: Arc<dyn ModelProvider>,
//     pub tools: ToolRegistry,
//     pub memory: Arc<dyn Memory>,
//     pub observer: Arc<dyn Observer>,
//     pub config: AgentConfig,
//     pub history: Mutex<Vec<ChatMessage>>,
//     /// 工具事件回调 (可选) -- CLI 通过此回调接收工具执行通知
//     pub tool_event_callback: Option<Arc<dyn ToolEventCallback>>,
//     /// 会话存储 (可选) -- 用于持久化对话历史
//     pub session_store: Option<Arc<dyn SessionStore>>,
//     /// 记忆策略 (可选) -- 对话前 recall + 对话后 store
//     pub memory_strategy: Option<Arc<dyn MemoryStrategy>>,
//     /// 技能改进器 (可选) -- 对话后异步触发技能审查
//     // pub skill_improver: Option<Arc<tokio::sync::Mutex<crate::skills::SkillImprover>>>,
//     /// 当前会话 ID (内存中维护, 通过 list() 修改时间动态确定)
//     current_session_id: Mutex<Option<String>>,
// }
// 
// impl Attributable for Agent {
//     fn role(&self) -> Role {
//         Role::Agent
//     }
//     fn alias(&self) -> &str {
//         &self.alias
//     }
// }
// 
// impl Agent {
//     /// 构建器
//     pub fn builder() -> AgentBuilder {
//         AgentBuilder::default()
//     }
// 
//     /// 获取当前生效的 system prompt
//     /// 优先使用 config.system_prompt, 未设则用默认
//     fn system_prompt(&self) -> &str {
//         self.config
//             .system_prompt
//             .as_deref()
//             .unwrap_or(DEFAULT_SYSTEM_PROMPT)
//     }
// 
//     /// 通知工具事件回调 (若已设置)
//     fn notify_tool_event(&self, event: &str, detail: &str) {
//         if let Some(cb) = &self.tool_event_callback {
//             cb(event, detail);
//         }
//     }
// 
//     /// 单轮对话 (含工具调用循环) -- 不带流式回调
//     ///
//     /// 等价于 `chat_with_stream(user_message, None)`
//     pub async fn chat(&self, user_message: &str) -> Result<String> {
//         self.chat_with_stream(user_message, None).await
//     }
// 
//     /// 单轮对话 (含工具调用循环) -- 带流式增量回调
//     ///
//     /// 流程:
//     /// 1. 截断过长的历史 (上下文窗口管理)
//     /// 2. 记忆策略 before_chat: recall 相关记忆, 注入 system prompt
//     /// 3. 构建消息 (system + history + user)
//     /// 4. 调用 LLM (流式 -- 每个 ContentDelta/ReasoningDelta 调用 on_delta 回调)
//     /// 5. 若响应包含 tool_calls, 执行工具, 将结果追加到消息, 回到步骤 4
//     /// 6. 若无 tool_calls, 保存历史并返回最终内容
//     /// 7. 记忆策略 after_chat: 存储本轮重要事实
//     pub async fn chat_with_stream(
//         &self,
//         user_message: &str,
//         on_delta: Option<Arc<dyn StreamDeltaCallback>>,
//     ) -> Result<String> {
//         // 记录会话开始
//         shadow_log::record!(INFO, Action::Start, "agent chat 开始");
// 
//         // 提前获取 session_id (供 memory_strategy 与 session_store 共用)
//         // 注: 首轮对话时为 None, after_chat 后由 session_store 触发生成 (见后文)
//         let session_id = self.current_session_id.lock().clone();
// 
//         // 上下文窗口管理: 截断过长的历史, 保留最近 max_history 条
//         // 注: 锁必须在 block 内释放, 避免进入 async 状态机导致 future !Send
//         let truncated = {
//             let mut history = self.history.lock();
//             if history.len() > self.config.max_history {
//                 let drain_count = history.len() - self.config.max_history;
//                 history.drain(0..drain_count);
//                 Some(drain_count)
//             } else {
//                 None
//             }
//         };
//         if let Some(drain_count) = truncated {
//             shadow_log::record!(
//                 INFO,
//                 Action::Note,
//                 format!(
//                     "历史超过 max_history({}), 截断 {} 条旧消息",
//                     self.config.max_history, drain_count
//                 )
//             );
//         }
// 
//         // 记忆策略 before_chat: recall 相关记忆, 拼接到 system prompt
//         let memory_context = if let Some(strategy) = &self.memory_strategy {
//             let entries = strategy
//                 .before_chat(user_message, session_id.as_deref())
//                 .await;
//             if entries.is_empty() {
//                 String::new()
//             } else {
//                 let ctx = format_entries(&entries);
//                 shadow_log::record!(
//                     INFO,
//                     Action::Note,
//                     format!("注入 {} 条记忆到 system prompt", entries.len())
//                 );
//                 ctx
//             }
//         } else {
//             String::new()
//         };
// 
//         // 构建初始消息 -- system prompt (基础 prompt + 可选记忆上下文)
//         let system_content = if memory_context.is_empty() {
//             self.system_prompt().to_string()
//         } else {
//             format!("{}\n\n{memory_context}", self.system_prompt())
//         };
//         let mut messages = vec![ChatMessage {
//             role: "system".to_string(),
//             content: system_content,
//         }];
// 
//         // 加载历史 (锁在 block 内释放, 避免 future !Send)
//         {
//             let history = self.history.lock();
//             messages.extend(history.iter().cloned());
//         }
// 
//         // 添加用户消息
//         messages.push(ChatMessage {
//             role: "user".to_string(),
//             content: user_message.to_string(),
//         });
// 
//         // 工具调用循环
//         #[allow(unused_assignments)]
//         let mut final_content = String::new();
//         let mut final_reasoning: Option<String> = None;
//         let mut iteration = 0;
// 
//         // P0: 循环检测器
//         let mut loop_detector = LoopDetector::new();
// 
//         loop {
//             iteration += 1;
//             if iteration > self.config.max_iterations {
//                 shadow_log::record!(WARN, Action::Fail, "工具调用超过最大循环次数, 终止");
//                 final_content = "工具调用次数超过上限, 终止对话.".to_string();
//                 break;
//             }
// 
//             // P0: 第一轮预裁剪历史 (若超过 token 预算)
//             if iteration == 1 && self.config.context_token_budget > 0 {
//                 let tokens = estimate_tokens(&messages);
//                 if tokens > self.config.context_token_budget {
//                     trim_history(&mut messages, self.config.context_token_budget);
//                     shadow_log::record!(INFO, Action::Note, "历史超过 token 预算, 已预裁剪");
//                 }
//             }
// 
//             // 记录 LLM 请求
//             self.observer.record_event(&ObserverEvent::LlmRequest {
//                 model: self.config.model.clone(),
//                 message_count: messages.len(),
//             });
// 
//             // 构建请求
//             let request = ChatRequest {
//                 messages: &messages,
//                 model: self.config.model.clone(),
//                 temperature: self.config.temperature,
//                 max_tokens: None,
//                 tools: None,
//             };
// 
//             // 调用 provider (流式) -- P0: 上下文溢出恢复
//             let start = std::time::Instant::now();
//             let response = match self.provider.chat(request, &self.config.model.clone(), self.config.temperature).await {
//                 Ok(s) => s,
//                 Err(e) => {
//                     // 尝试从上下文溢出中恢复
//                     if try_recover_context_overflow(&mut messages, &e) {
//                         shadow_log::record!(WARN, Action::Note, "上下文溢出, 已裁剪历史, 重试");
//                         continue;
//                     }
//                     return Err(e);
//                 }
//             };
// 
//             let duration_ms = start.elapsed().as_millis() as u64;
// 
//             // 记录 LLM 响应
//             self.observer.record_event(&ObserverEvent::LlmResponse {
//                 model: self.config.model.clone(),
//                 duration_ms,
//                 tokens: response.usage.as_ref().map(|u| u.total_tokens).unwrap_or(0),
//             });
// 
//             // 判断是否有工具调用
//             if response.tool_calls.is_empty() {
//                 // 无工具调用, 对话结束
//                 final_content = response.text.clone().unwrap_or_default();
//                 final_reasoning = response.reasoning_content.clone();
// 
//                 // 添加 assistant 消息到历史
//                 messages.push(ChatMessage {
//                     role: "assistant".to_string(),
//                     content: response.text.clone().unwrap_or_default(),
//                 });
//                 break;
//             }
// 
//             // 有工具调用: 先添加 assistant 消息 (含 tool_calls + reasoning_content)
//             // 注: reasoning_content 必须回传, 部分 provider 拒绝缺少此字段的 tool-call 历史
//             messages.push(ChatMessage {
//                 role: "assistant".to_string(),
//                 content: response.text.clone().unwrap_or_default(),
//             });
// 
//             // 执行工具调用 -- 检查是否有工具需要审批
//             // 如果任何工具 requires_approval, 全部串行执行 (安全第一)
//             let any_needs_approval = response.tool_calls.iter().any(|tc| {
//                 self.tools
//                     .find(&tc.name)
//                     .map(|t| t.requires_approval())
//                     .unwrap_or(false)
//             });
// 
//             if any_needs_approval {
//                 // 串行执行 (有工具需要审批, 逐个执行)
//                 let mut should_break = false;
//                 for tool_call in &response.tool_calls {
//                     shadow_log::record!(
//                         INFO,
//                         Action::Invoke,
//                         format!("调用工具: {} (id: {})", tool_call.name, tool_call.id)
//                     );
// 
//                     let tool_start = std::time::Instant::now();
//                     let result = self.execute_tool_call(tool_call).await;
//                     let tool_duration_ms = tool_start.elapsed().as_millis() as u64;
// 
//                     // 记录工具调用事件 (脱敏后)
//                     self.record_tool_event(&tool_call.name, &result, tool_duration_ms);
// 
//                     // 将工具结果添加到消息 (脱敏后)
//                     let tool_content = if result.success {
//                         scrub_credentials(&result.output)
//                     } else {
//                         format!(
//                             "[工具执行失败] {}",
//                             scrub_credentials(&result.error.unwrap_or_default())
//                         )
//                     };
// 
//                     // P0: 循环检测 -- 记录本次调用并处理结果
//                     let det_result =
//                         loop_detector.record(&tool_call.name, &tool_call.arguments, &tool_content);
//                     // (action_tag, message) -- action_tag 用于后续分支判断
//                     let (det_tag, det_msg): (u8, Option<String>) = match det_result {
//                         LoopDetectionResult::Ok => (0, None),
//                         LoopDetectionResult::Warning(msg) => {
//                             shadow_log::record!(WARN, Action::Note, &msg);
//                             (1, Some(msg))
//                         }
//                         LoopDetectionResult::Block(msg) => {
//                             shadow_log::record!(WARN, Action::Note, &msg);
//                             (2, Some(msg))
//                         }
//                         LoopDetectionResult::Break(msg) => {
//                             shadow_log::record!(WARN, Action::Fail, &msg);
//                             final_content = format!("工具循环被终止: {msg}");
//                             should_break = true;
//                             (3, None)
//                         }
//                     };
// 
//                     if should_break {
//                         messages.push(ChatMessage {
//                             role: "tool".to_string(),
//                             content: tool_content,
//                         });
//                         break;
//                     }
// 
//                     // Block(2) 时替换工具结果内容
//                     let actual_content = if det_tag == 2 {
//                         if let Some(ref m) = det_msg {
//                             format!("调用被循环检测阻止: {m}")
//                         } else {
//                             tool_content
//                         }
//                     } else {
//                         tool_content
//                     };
// 
//                     messages.push(ChatMessage {
//                         role: "tool".to_string(),
//                         content: actual_content,
//                     });
// 
//                     // Warning(1) 时注入提示消息
//                     if det_tag == 1
//                         && let Some(msg) = det_msg
//                     {
//                         messages.push(ChatMessage {
//                             role: "system".to_string(),
//                             content: format!("你似乎在重复调用工具, 请尝试不同方法. ({msg})"),
//                         });
//                     }
//                 }
//                 if should_break {
//                     break;
//                 }
//             } else {
//                 // 并行执行 (无审批需求, 使用 join_all 并发执行)
//                 shadow_log::record!(
//                     INFO,
//                     Action::Invoke,
//                     format!("并行执行 {} 个工具", response.tool_calls.len())
//                 );
// 
//                 let tool_calls = &response.tool_calls;
//                 let futures: Vec<_> = tool_calls
//                     .iter()
//                     .map(|tc| async move {
//                         let tool_start = std::time::Instant::now();
//                         let result = self.execute_tool_call(tc).await;
//                         let tool_duration_ms = tool_start.elapsed().as_millis() as u64;
//                         (tc, result, tool_duration_ms)
//                     })
//                     .collect();
//                 let results = futures::future::join_all(futures).await;
// 
//                 // 按顺序处理结果
//                 let mut should_break = false;
//                 for (tool_call, result, tool_duration_ms) in results {
//                     // 记录工具调用事件 (脱敏后)
//                     self.record_tool_event(&tool_call.name, &result, tool_duration_ms);
// 
//                     // 将工具结果添加到消息 (脱敏后)
//                     let tool_content = if result.success {
//                         scrub_credentials(&result.output)
//                     } else {
//                         format!(
//                             "[工具执行失败] {}",
//                             scrub_credentials(&result.error.unwrap_or_default())
//                         )
//                     };
// 
//                     // P0: 循环检测 -- 记录本次调用并处理结果
//                     let det_result =
//                         loop_detector.record(&tool_call.name, &tool_call.arguments, &tool_content);
//                     let (det_tag, det_msg): (u8, Option<String>) = match det_result {
//                         LoopDetectionResult::Ok => (0, None),
//                         LoopDetectionResult::Warning(msg) => {
//                             shadow_log::record!(WARN, Action::Note, &msg);
//                             (1, Some(msg))
//                         }
//                         LoopDetectionResult::Block(msg) => {
//                             shadow_log::record!(WARN, Action::Note, &msg);
//                             (2, Some(msg))
//                         }
//                         LoopDetectionResult::Break(msg) => {
//                             shadow_log::record!(WARN, Action::Fail, &msg);
//                             final_content = format!("工具循环被终止: {msg}");
//                             should_break = true;
//                             (3, None)
//                         }
//                     };
// 
//                     if should_break {
//                         messages.push(ChatMessage {
//                             role: "tool".to_string(),
//                             content: tool_content,
//                         });
//                         break;
//                     }
// 
//                     // Block(2) 时替换工具结果内容
//                     let actual_content = if det_tag == 2 {
//                         if let Some(ref m) = det_msg {
//                             format!("调用被循环检测阻止: {m}")
//                         } else {
//                             tool_content
//                         }
//                     } else {
//                         tool_content
//                     };
// 
//                     messages.push(ChatMessage {
//                         role: "tool".to_string(),
//                         content: actual_content,
//                     });
// 
//                     // Warning(1) 时注入提示消息
//                     if det_tag == 1
//                         && let Some(msg) = det_msg
//                     {
//                         messages.push(ChatMessage {
//                             role: "system".to_string(),
//                             content: format!("你似乎在重复调用工具, 请尝试不同方法. ({msg})"),
//                         });
//                     }
//                 }
//                 if should_break {
//                     break;
//                 }
//             }
// 
//             // 继续循环, 将工具结果发给 LLM
//         }
// 
//         // 保留原始内容 (含 <think> 标签), 由显示层决定是否过滤
//         // reasoning_content (DeepSeek-R1 等 API 独立字段) 也保留
//         let final_reasoning = final_reasoning.take();
// 
//         // 保存到历史 (只保存 user + 最终 assistant, 不保存中间 tool 消息)
//         // 注: 锁必须在 block 内释放, 避免进入 async 状态机导致 future !Send
//         {
//             let mut history = self.history.lock();
//             history.push(ChatMessage {
//                 role: "user".to_string(),
//                 content: user_message.to_string(),
//             });
//             history.push(ChatMessage {
//                 role: "assistant".to_string(),
//                 content: final_content.clone(),
//             });
//         }
// 
//         // 保存到 session store (追加新的 user + assistant 消息)
//         if let Some(store) = &self.session_store {
//             // 获取或生成会话 ID
//             let session_id = {
//                 let mut sid = self.current_session_id.lock();
//                 if sid.is_none() {
//                     *sid = Some(uuid::Uuid::new_v4().to_string());
//                 }
//                 sid.clone()
//             };
//             if let Some(id) = session_id {
//                 // 使用 append_message API -- 语义清晰: 追加两条新消息到现有会话
//                 let user_msg = ChatMessage {
//                     role: "user".to_string(),
//                     content: user_message.to_string(),
//                 };
//                 let assistant_msg = ChatMessage {
//                     role: "assistant".to_string(),
//                     content: final_content.clone(),
//                     };
//                 if let Err(e) = store.append_message(&id, &user_msg).await {
//                     shadow_log::record!(WARN, Action::Fail, format!("追加用户消息失败: {e}"));
//                 }
//                 if let Err(e) = store.append_message(&id, &assistant_msg).await {
//                     shadow_log::record!(WARN, Action::Fail, format!("追加助手消息失败: {e}"));
//                 }
//             }
//         }
// 
//         // 记忆策略 after_chat: 提取并存储本轮重要事实
//         // 重新读取 session_id (首轮对话 session_store 会刚生成新 id,
//         // 让 after_chat 用与下一轮 before_chat 一致的 session 作用域)
//         if let Some(strategy) = &self.memory_strategy {
//             let sid = self.current_session_id.lock().clone();
//             if let Err(e) = strategy
//                 .after_chat(user_message, &final_content, sid.as_deref())
//                 .await
//             {
//                 shadow_log::record!(WARN, Action::Fail, format!("after_chat 记忆存储失败: {e}"));
//             }
//         }
// 
//         shadow_log::record!(INFO, Action::Complete, "agent chat 完成");
// 
//         // 技能审查: 对话后异步触发, 不阻塞用户
//         if self.config.skill_review_enabled {
//             let history_snapshot: Vec<ChatMessage> = self.history.lock().clone();
//             let workspace = self.config.workspace_dir.clone();
//             let threshold = self.config.skill_review_nudge_threshold;
//             let model = self.config.model.clone();
//             let provider = Arc::clone(&self.provider);
//             // 异步触发, 不阻塞用户返回
//             tokio::spawn(async move {
//                 if let Err(e) = crate::skills::maybe_run_skill_review(
//                     workspace,
//                     &history_snapshot,
//                     threshold,
//                     provider.as_ref(),
//                     &model,
//                 )
//                 .await
//                 {
//                     shadow_log::record!(WARN, Action::Fail, format!("技能审查异步任务失败: {e}"));
//                 }
//             });
//         }
// 
//         Ok(final_content)
//     }
// 
//     /// 记录工具调用事件到 observer (脱敏后)
//     fn record_tool_event(&self, tool_name: &str, result: &ToolResult, duration_ms: u64) {
//         let full = if result.success {
//             result.output.clone()
//         } else {
//             result.error.clone().unwrap_or_default()
//         };
//         // 脱敏后截断预览
//         let scrubbed = scrub_credentials(&full);
//         let preview = chars_preview(&scrubbed, 200);
// 
//         self.observer.record_event(&ObserverEvent::ToolCall {
//             tool: tool_name.to_string(),
//             success: result.success,
//             duration_ms,
//             output_preview: preview,
//         });
//     }
// 
//     /// 执行单个工具调用
//     ///
//     /// 包含以下检查:
//     /// 1. 只读模式拒绝
//     /// 2. Supervised 模式审批检查 (requires_approval 的工具跳过执行)
//     /// 3. 参数校验 (P1.6: validate_args 校验 args 符合 parameters_schema)
//     /// 4. 工具超时控制 (tool.timeout() 返回的时长)
//     /// 5. 工具事件回调通知
//     async fn execute_tool_call(&self, tool_call: &ToolCall) -> ToolResult {
//         // 查找匹配的工具 (通过 ToolRegistry)
//         let tool = self.tools.find(&tool_call.name);
// 
//         match tool {
//             Some(t) => {
//                 // 检查自主级别 -- 只读模式拒绝所有工具
//                 if self.config.autonomy == AutonomyLevel::ReadOnly {
//                     return ToolResult::err("只读模式: 工具执行被拒绝");
//                 }
// 
//                 // 审批检查: Supervised 模式下, 需要审批的工具跳过执行
//                 if self.config.autonomy == AutonomyLevel::Supervised && t.requires_approval() {
//                     let msg = format!(
//                         "工具 [{}] 需要用户审批 (supervised 模式), 已跳过执行",
//                         tool_call.name
//                     );
//                     println!("{msg}");
//                     self.notify_tool_event("tool_approval_skipped", &tool_call.name);
//                     return ToolResult::err("需要用户审批 (supervised 模式)");
//                 }
// 
//                 // 通知工具开始执行
//                 self.notify_tool_event("tool_start", &tool_call.name);
// 
//                 // P1.6: 参数校验 -- 在 execute 前校验 args 符合 parameters_schema
//                 if let Err(msg) = t.validate_args(&tool_call.arguments) {
//                     let detail = format!("{}: {msg}", tool_call.name);
//                     self.notify_tool_event("tool_error", &detail);
//                     return ToolResult::err(format!("参数校验失败: {msg}"));
//                 }
// 
//                 // 获取工具超时配置 -- None 表示不限制
//                 let timeout = t.timeout();
// 
//                 // 执行工具 (带超时控制)
//                 let exec_future = t.execute(tool_call.arguments.clone());
// 
//                 let result = match timeout {
//                     Some(d) => {
//                         // 有超时: 用 tokio::time::timeout 包装
//                         match tokio::time::timeout(d, exec_future).await {
//                             Ok(Ok(result)) => result,
//                             Ok(Err(e)) => ToolResult::err(format!("工具执行异常: {e}")),
//                             Err(_) => {
//                                 let msg = format!(
//                                     "工具 [{}] 执行超时 ({}ms)",
//                                     tool_call.name,
//                                     d.as_millis()
//                                 );
//                                 println!("{msg}");
//                                 self.notify_tool_event("tool_timeout", &tool_call.name);
//                                 ToolResult::err("工具执行超时")
//                             }
//                         }
//                     }
//                     None => {
//                         // 无超时: 直接执行
//                         match exec_future.await {
//                             Ok(result) => result,
//                             Err(e) => ToolResult::err(format!("工具执行异常: {e}")),
//                         }
//                     }
//                 };
// 
//                 // 通知工具执行结果
//                 if result.success {
//                     self.notify_tool_event("tool_success", &tool_call.name);
//                 } else {
//                     let detail = format!(
//                         "{}: {}",
//                         tool_call.name,
//                         result.error.as_deref().unwrap_or("未知错误")
//                     );
//                     self.notify_tool_event("tool_error", &detail);
//                 }
// 
//                 result
//             }
//             None => ToolResult::err(format!("未找到工具: {}", tool_call.name)),
//         }
//     }
// 
//     /// 清空历史 (同时删除 session store 中的当前会话)
//     pub async fn clear_history(&self) {
//         self.history.lock().clear();
// 
//         // 同时清除 session store 中的当前会话
//         if let Some(store) = &self.session_store {
//             let sid = self.current_session_id.lock().take();
//             if let Some(id) = sid
//                 && let Err(e) = store.delete(&id).await
//             {
//                 shadow_log::record!(WARN, Action::Fail, format!("删除会话失败: {e}"));
//             }
//         }
//     }
// 
//     /// 从 session store 加载最近的会话历史到 self.history
//     ///
//     /// 通过 list() 按修改时间降序排序, 取第一个 (最近修改的) 会话.
//     /// 如果没有已保存的会话, 不做任何操作.
//     pub async fn load_history(&self) -> Result<()> {
//         let Some(store) = &self.session_store else {
//             return Ok(());
//         };
// 
//         let sessions = store.list().await?;
//         if sessions.is_empty() {
//             return Ok(());
//         }
// 
//         // list() 按修改时间降序排序, 第一个是最新的
//         let latest_id = &sessions[0];
//         if let Some(session) = store.load(latest_id).await? {
//             {
//                 let mut history = self.history.lock();
//                 history.clear();
//                 history.extend(session.messages);
//             }
//             let mut sid = self.current_session_id.lock();
//             *sid = Some(session.id);
//         }
// 
//         Ok(())
//     }
// 
//     /// 返回当前会话 ID
//     ///
//     /// 会话 ID 在 load_history() 时从最新修改的会话确定,
//     /// 或在首次 chat() 时生成新的 UUID.
//     #[must_use]
//     pub fn current_session_id(&self) -> Option<String> {
//         self.current_session_id.lock().clone()
//     }
// }
// 
// /// Agent 构建器
// #[derive(Default)]
// pub struct AgentBuilder {
//     alias: Option<String>,
//     provider: Option<Arc<dyn ModelProvider>>,
//     tools: Option<ToolRegistry>,
//     memory: Option<Arc<dyn Memory>>,
//     observer: Option<Arc<dyn Observer>>,
//     config: Option<AgentConfig>,
//     tool_event_callback: Option<Arc<dyn ToolEventCallback>>,
//     session_store: Option<Arc<dyn SessionStore>>,
//     memory_strategy: Option<Arc<dyn MemoryStrategy>>,
//     skill_improver: Option<Arc<tokio::sync::Mutex<crate::skills::SkillImprover>>>,
// }
// 
// impl AgentBuilder {
//     pub fn alias(mut self, alias: impl Into<String>) -> Self {
//         self.alias = Some(alias.into());
//         self
//     }
//     pub fn provider(mut self, provider: Arc<dyn ModelProvider>) -> Self {
//         self.provider = Some(provider);
//         self
//     }
//     pub fn tools(mut self, tools: ToolRegistry) -> Self {
//         self.tools = Some(tools);
//         self
//     }
//     pub fn memory(mut self, memory: Arc<dyn Memory>) -> Self {
//         self.memory = Some(memory);
//         self
//     }
//     pub fn observer(mut self, observer: Arc<dyn Observer>) -> Self {
//         self.observer = Some(observer);
//         self
//     }
//     pub fn config(mut self, config: AgentConfig) -> Self {
//         self.config = Some(config);
//         self
//     }
// 
//     /// 设置自定义 system prompt
//     pub fn system_prompt(mut self, prompt: impl Into<String>) -> Self {
//         let config = self.config.get_or_insert_with(AgentConfig::default);
//         config.system_prompt = Some(prompt.into());
//         self
//     }
// 
//     /// 设置对话历史最大条数
//     pub fn max_history(mut self, max: usize) -> Self {
//         let config = self.config.get_or_insert_with(AgentConfig::default);
//         config.max_history = max;
//         self
//     }
// 
//     /// 设置上下文 token 预算 (0 = 不限制)
//     pub fn context_token_budget(mut self, budget: usize) -> Self {
//         let config = self.config.get_or_insert_with(AgentConfig::default);
//         config.context_token_budget = budget;
//         self
//     }
// 
//     /// 设置工具事件回调
//     pub fn tool_event_callback(mut self, callback: Arc<dyn ToolEventCallback>) -> Self {
//         self.tool_event_callback = Some(callback);
//         self
//     }
// 
//     /// 设置会话存储 (用于持久化对话历史)
//     pub fn session_store(mut self, store: Arc<dyn SessionStore>) -> Self {
//         self.session_store = Some(store);
//         self
//     }
// 
//     /// 设置记忆策略 (用于对话前 recall + 对话后 store)
//     ///
//     /// 不设置则不启用记忆上下文注入和自动存储.
//     pub fn memory_strategy(mut self, strategy: Arc<dyn MemoryStrategy>) -> Self {
//         self.memory_strategy = Some(strategy);
//         self
//     }
// 
//     /// 设置技能改进器 (用于对话后异步技能审查)
//     pub fn skill_improver(
//         mut self,
//         improver: Arc<tokio::sync::Mutex<crate::skills::SkillImprover>>,
//     ) -> Self {
//         self.skill_improver = Some(improver);
//         self
//     }
// 
//     /// 启用对话后技能审查
//     pub fn enable_skill_review(mut self, nudge_threshold: usize) -> Self {
//         let config = self.config.get_or_insert_with(AgentConfig::default);
//         config.skill_review_enabled = true;
//         config.skill_review_nudge_threshold = nudge_threshold;
//         self
//     }
// 
//     /// 构建 Agent
//     pub fn build(self) -> Result<Agent> {
//         let config = self.config.unwrap_or_default();
//         let alias = self.alias.unwrap_or_else(|| config.alias.clone());
//         let provider = self
//             .provider
//             .ok_or_else(|| anyhow::anyhow!("缺少 provider, 请通过 .provider() 设置"))?;
//         let memory = self
//             .memory
//             .unwrap_or_else(|| Arc::new(shadow_core::NoneMemory));
//         let observer = self
//             .observer
//             .unwrap_or_else(|| Arc::new(shadow_core::NoopObserver));
//         let tools = self.tools.unwrap_or_default();
//         let tool_event_callback = self.tool_event_callback;
//         let session_store = self.session_store;
//         let memory_strategy = self.memory_strategy;
//         let skill_improver = self.skill_improver;
// 
//         Ok(Agent {
//             alias,
//             provider,
//             tools,
//             memory,
//             observer,
//             config,
//             history: Mutex::new(Vec::new()),
//             tool_event_callback,
//             session_store,
//             memory_strategy,
//             skill_improver,
//             current_session_id: Mutex::new(None),
//         })
//     }
// }
// 
// /// 估算消息列表的 token 数 (~4 chars/token + 每条消息 10 token 开销)
// ///
// /// 粗略估算, 不依赖 tokenizer; 用于上下文预算检查.
// fn estimate_tokens(messages: &[ChatMessage]) -> usize {
//     messages
//         .iter()
//         .map(|m| m.content.chars().count() / 4 + 10)
//         .sum()
// }
// 
// /// 按整轮裁剪历史 (保留 system 消息, 删最旧的非 system 消息)
// ///
// /// 反复删除第一个非 system 消息, 直到 token 估算 <= budget 或消息数 <= 3.
// fn trim_history(messages: &mut Vec<ChatMessage>, budget: usize) {
//     while estimate_tokens(messages) > budget && messages.len() > 3 {
//         // 找到第一个非 system 消息删除
//         let pos = messages.iter().position(|m| m.role != "system");
//         if let Some(pos) = pos {
//             messages.remove(pos);
//         } else {
//             break;
//         }
//     }
// }
// 
// /// 尝试从上下文溢出中恢复
// ///
// /// 检查错误信息是否包含上下文/token 溢出关键词.
// /// 若是, 裁剪历史到当前的 2/3 并注入裁剪提示, 返回 true.
// /// 否则返回 false (调用方应原样返回错误).
// fn try_recover_context_overflow(messages: &mut Vec<ChatMessage>, error: &anyhow::Error) -> bool {
//     let err_str = error.to_string().to_lowercase();
//     let is_overflow = err_str.contains("context")
//         || err_str.contains("token")
//         || err_str.contains("too long")
//         || err_str.contains("maximum")
//         || err_str.contains("overflow");
// 
//     if !is_overflow {
//         return false;
//     }
// 
//     let current = estimate_tokens(messages);
//     let target = current * 2 / 3;
//     trim_history(messages, target);
// 
//     // 注入裁剪提示 (在所有 system 消息后)
//     let system_count = messages.iter().take_while(|m| m.role == "system").count();
//     messages.insert(
//         system_count,
//         ChatMessage {
//             role: "system".to_string(),
//             content: "[部分早期对话历史已裁剪以适应上下文窗口]".to_string(),
//         },
//     );
// 
//     true
// }
// 
// /// 截断字符串到最多 n 个字符 (按 char, 非 byte), 超出加 "..."
// fn chars_preview(s: &str, n: usize) -> String {
//     let mut out: String = s.chars().take(n).collect();
//     if s.chars().count() > n {
//         out.push_str("...");
//     }
//     out
// }
// 
// /// 凭证脱敏 -- 替换文本中的 API key / Bearer token / token= 等敏感信息
// ///
// /// 匹配模式:
// /// - `sk-xxx` (20+ 字符): OpenAI 风格 API key
// /// - `Bearer xxx` (20+ 字符): HTTP Bearer token
// /// - `token=xxx` / `token:xxx` (20+ 字符): query/header token
// ///
// /// 替换为对应的 `***` 占位符, 防止敏感信息泄露到日志和 observer 事件中.
// fn scrub_credentials(text: &str) -> String {
//     use regex::Regex;
//     use std::sync::OnceLock;
// 
//     // 正则编译较慢, 使用 OnceLock 缓存
//     static RE_SK: OnceLock<Regex> = OnceLock::new();
//     static RE_BEARER: OnceLock<Regex> = OnceLock::new();
//     static RE_TOKEN: OnceLock<Regex> = OnceLock::new();
// 
//     let re_sk = RE_SK.get_or_init(|| Regex::new(r"sk-[a-zA-Z0-9]{20,}").unwrap());
//     let re_bearer = RE_BEARER.get_or_init(|| Regex::new(r"Bearer\s+[a-zA-Z0-9._-]{20,}").unwrap());
//     let re_token = RE_TOKEN.get_or_init(|| Regex::new(r"token[=:]\s*[a-zA-Z0-9]{20,}").unwrap());
// 
//     let result = re_sk.replace_all(text, "sk-***");
//     let result = re_bearer.replace_all(&result, "Bearer ***");
//     let result = re_token.replace_all(&result, "token=***");
// 
//     result.into_owned()
// }
// 
