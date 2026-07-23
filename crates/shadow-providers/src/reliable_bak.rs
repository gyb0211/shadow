// //! ReliableModelProvider -- 重试 / 退避 / key 轮换 / 限流 / fallback_models
// //!
// //! Decorator 模式包裹 inner provider (通常 OpenAiProvider). 实现 ModelProvider trait,
// //! Router 透明使用 -- 不感知 Reliable 存在.
// //!
// //! 错误分类由 [`crate::error::ChatError`] 提供:
// //! - Transient (5xx) / RateLimit (429) / Network -- 退避后重试
// //! - Auth (401/403) -- 触发 key 轮换 (如配置); 无 rotator 或 keys 用尽则直接返回
// //! - Permanent (4xx 其他) -- 立即返回不重试
// //!
// //! 流式重试: 只重试 pre-stream 错误 (建立连接失败). 一旦 Ok(BoxStream) 返回, mid-stream
// //! 错误视为 terminal, 不再重试 (避免重复 chunk).
// 
// use crate::error::ChatError;
// use crate::rate_limit::TokenBucket;
// use anyhow::Result;
// use async_trait::async_trait;
// use futures::stream::BoxStream;
// use shadow_core::kennel::provider::StreamChunk;
// use shadow_core::{Attributable, ChatRequest, ChatResponse, ModelProvider, Role};
// use std::sync::atomic::{AtomicUsize, Ordering};
// use std::sync::Arc;
// use std::time::Duration;
// use tracing::{debug, info, warn};
// 
// /// 可切换 key 的 provider trait -- OpenAiProvider 等具体 provider 实现
// ///
// /// Reliable 层通过此 trait 在调用前注入当前轮换的 key. 不强制所有 provider 实现,
// /// 只 OpenAiProvider (及未来其他带 key 的 provider) 实现.
// pub trait KeyRotator: Send + Sync {
//     fn set_key(&self, key: Option<&str>);
// }
// 
// /// 重试策略 -- 指数退避 + jitter
// #[derive(Debug, Clone, Copy)]
// pub struct RetryPolicy {
//     /// 最大重试次数 (0 = 不重试, 只调用一次)
//     pub max_retries: u32,
//     /// 初始退避毫秒
//     pub initial_backoff_ms: u64,
//     /// 退避上限毫秒
//     pub max_backoff_ms: u64,
//     /// jitter 百分比 (0-100, 加随机偏移防雪崩)
//     pub jitter_pct: u8,
// }
// 
// impl Default for RetryPolicy {
//     fn default() -> Self {
//         Self {
//             max_retries: 3,
//             initial_backoff_ms: 1000,
//             max_backoff_ms: 60_000,
//             jitter_pct: 25,
//         }
//     }
// }
// 
// impl RetryPolicy {
//     /// 计算第 `attempt` 次重试的退避时长 (含 jitter)
//     ///
//     /// attempt=0 是首次调用, attempt>=1 是重试.
//     /// 公式: backoff = min(initial * 2^(attempt-1), max) + jitter_pct% 偏移
//     #[must_use]
//     pub fn compute_backoff(&self, attempt: u32) -> Duration {
//         if attempt == 0 {
//             return Duration::ZERO;
//         }
//         let exp = (attempt - 1).min(20); // 防溢出, cap 2^20
//         let raw_ms = self
//             .initial_backoff_ms
//             .saturating_mul(2_u64.saturating_pow(exp));
//         let capped_ms = raw_ms.min(self.max_backoff_ms);
//         // jitter: 0..=jitter_pct 的随机比例
//         let jitter_range = (self.jitter_pct as u64).min(100);
//         let jitter_factor = if jitter_range == 0 {
//             0
//         } else {
//             // 简单的 LCG 伪随机 -- 不引入 rand crate
//             let seed = std::time::SystemTime::now()
//                 .duration_since(std::time::UNIX_EPOCH)
//                 .map(|d| d.subsec_nanos() as u64)
//                 .unwrap_or(0)
//                 .wrapping_add(capped_ms.wrapping_mul(0x9E3779B97F4A7C15));
//             seed % (jitter_range + 1)
//         };
//         let actual_ms = capped_ms + (capped_ms * jitter_factor / 100);
//         Duration::from_millis(actual_ms.min(self.max_backoff_ms))
//     }
// }
// 
// /// Reliable 包装层 -- 在 inner provider 之上加重试 / 退避 / key 轮换 / 限流 / fallback_models
// pub struct ReliableModelProvider {
//     alias: String,
//     inner: Arc<dyn ModelProvider>,
//     policy: RetryPolicy,
//     /// key 池 (用于轮换). 空 Vec 表示单 key 模式 (无轮换).
//     keys: Vec<String>,
//     /// round-robin 索引 (取模 keys.len())
//     key_idx: AtomicUsize,
//     /// 可选: key 注入器 (OpenAiProvider::set_api_key 等). None 表示不支持运行时切换.
//     rotator: Option<Arc<dyn KeyRotator>>,
//     /// 可选: 限流器. None 表示无限流.
//     rate_limiter: Option<Arc<TokenBucket>>,
//     /// 模型级 fallback (同 provider 失败时换模型重试). 优先级在 retry 之后, 跨 provider fallback 之前.
//     fallback_models: Vec<String>,
// }
// 
// impl ReliableModelProvider {
//     /// 构造 -- 简单形态 (仅重试, 无 key 轮换/限流/fallback)
//     #[must_use]
//     pub fn new(alias: impl Into<String>, inner: Arc<dyn ModelProvider>, policy: RetryPolicy) -> Self {
//         Self {
//             alias: alias.into(),
//             inner,
//             policy,
//             keys: Vec::new(),
//             key_idx: AtomicUsize::new(0),
//             rotator: None,
//             rate_limiter: None,
//             fallback_models: Vec::new(),
//         }
//     }
// 
//     /// Builder: 注入 key 轮换池 + rotator
//     #[must_use]
//     pub fn with_key_rotation(
//         mut self,
//         keys: Vec<String>,
//         rotator: Arc<dyn KeyRotator>,
//     ) -> Self {
//         self.keys = keys;
//         self.rotator = Some(rotator);
//         self
//     }
// 
//     /// Builder: 注入限流器
//     #[must_use]
//     pub fn with_rate_limiter(mut self, bucket: Arc<TokenBucket>) -> Self {
//         self.rate_limiter = Some(bucket);
//         self
//     }
// 
//     /// Builder: 注入模型级 fallback
//     #[must_use]
//     pub fn with_fallback_models(mut self, models: Vec<String>) -> Self {
//         self.fallback_models = models;
//         self
//     }
// 
//     /// 借用 inner -- 测试 / Router 内省用
//     #[must_use]
//     pub fn inner(&self) -> &dyn ModelProvider {
//         self.inner.as_ref()
//     }
// 
//     /// 提取 ChatError 分类; 非 ChatError 返回 None
//     fn classify_error(err: &anyhow::Error) -> Option<&ChatError> {
//         err.downcast_ref::<ChatError>()
//     }
// 
//     /// 在 inner.chat() 调用前应用 pre-call 副作用:
//     /// - 限流等待
//     /// - key 轮换注入
//     async fn pre_call(&self) {
//         if let Some(limiter) = &self.rate_limiter {
//             limiter.acquire().await;
//         }
//         if let (Some(rotator), Some(key)) = (
//             self.rotator.as_ref(),
//             self.pick_current_key(),
//         ) {
//             rotator.set_key(Some(key));
//         }
//     }
// 
//     /// 取当前轮换 key (round-robin). 无 keys 时返回 None.
//     fn pick_current_key(&self) -> Option<&str> {
//         if self.keys.is_empty() {
//             return None;
//         }
//         let idx = self.key_idx.load(Ordering::SeqCst) % self.keys.len();
//         Some(self.keys[idx].as_str())
//     }
// 
//     /// 推进 key 索引到下一个 -- Auth 错误时调用
//     fn advance_key(&self) {
//         if !self.keys.is_empty() {
//             self.key_idx.fetch_add(1, Ordering::SeqCst);
//         }
//     }
// 
//     /// 同步 chat 重试循环 -- 支持 key 轮换 + fallback_models
//     async fn chat_with_retry(&self, request: ChatRequest<'_>) -> Result<ChatResponse> {
//         // 模型列表: 原始 model + fallback_models
//         let mut models_to_try: Vec<String> = vec![request.model.clone()];
//         models_to_try.extend(self.fallback_models.iter().cloned());
// 
//         let mut last_err: Option<anyhow::Error> = None;
//         'outer: for (model_idx, model) in models_to_try.iter().enumerate() {
//             if model_idx > 0 {
//                 info!(new_model = %model, "切换到 fallback 模型");
//             }
//             let temperature = request.temperature;
// 
//             let mut keys_tried_this_model = 0usize;
//             for attempt in 0..=self.policy.max_retries {
//                 if attempt > 0 {
//                     let backoff = self.policy.compute_backoff(attempt);
//                     debug!(attempt, max_retries = self.policy.max_retries, ?backoff, "重试");
//                     tokio::time::sleep(backoff).await;
//                 }
//                 self.pre_call().await;
//                 match self.inner.chat(request.clone(), model, temperature).await {
//                     Ok(resp) => return Ok(resp),
//                     Err(err) => {
//                         let class_opt = Self::classify_error(&err);
//                         let retryable = class_opt
//                             .map(|e| e.is_retryable())
//                             .unwrap_or(false);
//                         let is_auth = class_opt
//                             .map(|e| e.is_auth_error())
//                             .unwrap_or(false);
// 
//                         // Auth 错误: 推进 key 立即重试 (无 backoff)
//                         if is_auth && !self.keys.is_empty() {
//                             keys_tried_this_model += 1;
//                             // 所有 key 都试过仍 auth → 放弃当前 model
//                             if keys_tried_this_model >= self.keys.len() {
//                                 warn!(attempt, error = %err, "所有 key 均 auth 失败");
//                                 last_err = Some(err);
//                                 break; // 跳到下一个 fallback model
//                             }
//                             self.advance_key();
//                             warn!(attempt, error = %err, "auth 错误, 切换 key 重试");
//                             last_err = Some(err);
//                             continue;
//                         }
//                         if !retryable {
//                             // 永久错误: 当前 model 无救, 尝试下一个 fallback model
//                             debug!(error = %err, "永久错误, 跳到下一个 fallback model");
//                             last_err = Some(err);
//                             continue 'outer;
//                         }
//                         warn!(attempt, error = %err, "可重试错误, 退避后重试");
//                         last_err = Some(err);
//                     }
//                 }
//             }
//             // 当前 model 重试耗尽, 尝试下一个 fallback model (外层循环)
//         }
//         Err(last_err.unwrap_or_else(|| anyhow::anyhow!("重试耗尽但无错误")))
//     }
// }
// 
// impl Attributable for ReliableModelProvider {
//     fn role(&self) -> Role {
//         Role::Provider
//     }
//     fn alias(&self) -> &str {
//         &self.alias
//     }
// }
// 
// #[async_trait]
// impl ModelProvider for ReliableModelProvider {
//     /// chat_with_system -- 带重试/key 轮换/fallback 的单轮调用
//     async fn chat_with_system(
//         &self,
//         system_prompt: Option<&str>,
//         message: &str,
//         model: &str,
//         temperature: Option<f64>,
//     ) -> Result<String> {
//         // 构建临时 ChatRequest 用于 chat_with_retry
//         let messages = vec![
//             shadow_core::ChatMessage {
//                 role: "system".to_string(),
//                 content: system_prompt.unwrap_or("").to_string(),
//             },
//             shadow_core::ChatMessage {
//                 role: "user".to_string(),
//                 content: message.to_string(),
//             },
//         ];
//         let request = ChatRequest {
//             messages: &messages,
//             model: model.to_string(),
//             temperature,
//             max_tokens: None,
//             tools: None,
//         };
//         let resp = self.chat_with_retry(request).await?;
//         Ok(resp.text.unwrap_or_default())
//     }
// 
//     async fn list_models(&self) -> Result<Vec<String>> {
//         self.inner.list_models().await
//     }
// 
//     fn supports_native_tools(&self) -> bool {
//         self.inner.supports_native_tools()
//     }
// 
//     fn default_temperature(&self) -> f64 {
//         self.inner.default_temperature()
//     }
// }
// 
