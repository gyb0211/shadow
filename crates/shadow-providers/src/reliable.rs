//! ReliableModelProvider -- 重试 / 退避 / (Phase 2: key 轮换 / 限流 / fallback_models)
//!
//! Decorator 模式包裹 inner provider (通常 OpenAiProvider). 实现 Provider trait,
//! Router 透明使用 -- 不感知 Reliable 存在.
//!
//! 错误分类由 [`crate::error::ChatError`] 提供:
//! - Transient (5xx) / RateLimit (429) / Network -- 退避后重试
//! - Auth (401/403) -- 当前阶段直接返回 (Phase 2 切 key 重试)
//! - Permanent (4xx 其他) -- 立即返回不重试
//!
//! 流式重试: 只重试 pre-stream 错误 (建立连接失败). 一旦 Ok(BoxStream) 返回, mid-stream
//! 错误视为 terminal, 不再重试 (避免重复 chunk).

use crate::error::ChatError;
use anyhow::Result;
use async_trait::async_trait;
use futures::stream::BoxStream;
use shadow_core::provider::ChatChunk;
use shadow_core::{Attributable, ChatRequest, ChatResponse, Provider, Role};
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, warn};

/// 重试策略 -- 指数退避 + jitter
#[derive(Debug, Clone, Copy)]
pub struct RetryPolicy {
    /// 最大重试次数 (0 = 不重试, 只调用一次)
    pub max_retries: u32,
    /// 初始退避毫秒
    pub initial_backoff_ms: u64,
    /// 退避上限毫秒
    pub max_backoff_ms: u64,
    /// jitter 百分比 (0-100, 加随机偏移防雪崩)
    pub jitter_pct: u8,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 3,
            initial_backoff_ms: 1000,
            max_backoff_ms: 60_000,
            jitter_pct: 25,
        }
    }
}

impl RetryPolicy {
    /// 计算第 `attempt` 次重试的退避时长 (含 jitter)
    ///
    /// attempt=0 是首次调用, attempt>=1 是重试.
    /// 公式: backoff = min(initial * 2^(attempt-1), max) * (1 + jitter_pct/100 * random[-1,1])
    #[must_use]
    pub fn compute_backoff(&self, attempt: u32) -> Duration {
        if attempt == 0 {
            return Duration::ZERO;
        }
        let exp = (attempt - 1).min(20); // 防溢出, cap 2^20
        let raw_ms = self
            .initial_backoff_ms
            .saturating_mul(2_u64.saturating_pow(exp));
        let capped_ms = raw_ms.min(self.max_backoff_ms);
        // jitter: 0..=jitter_pct 的随机比例
        let jitter_range = (self.jitter_pct as u64).min(100);
        let jitter_factor = if jitter_range == 0 {
            0
        } else {
            // 简单的 LCG 伪随机 -- 不引入 rand crate
            let seed = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos() as u64)
                .unwrap_or(0)
                .wrapping_add(capped_ms.wrapping_mul(0x9E3779B97F4A7C15));
            (seed % (jitter_range + 1)) as u64
        };
        let actual_ms = capped_ms + (capped_ms * jitter_factor / 100);
        Duration::from_millis(actual_ms.min(self.max_backoff_ms))
    }
}

/// Reliable 包装层 -- 在 inner provider 之上加重试 / 退避
///
/// Phase 1: 单 key, 仅重试.
/// Phase 2 将扩展: 多 key 轮换 / 限流 / fallback_models.
pub struct ReliableModelProvider {
    alias: String,
    inner: Arc<dyn Provider>,
    policy: RetryPolicy,
}

impl ReliableModelProvider {
    /// 构造 -- 包装 inner provider, 应用重试策略
    #[must_use]
    pub fn new(alias: impl Into<String>, inner: Arc<dyn Provider>, policy: RetryPolicy) -> Self {
        Self {
            alias: alias.into(),
            inner,
            policy,
        }
    }

    /// 借用 inner -- 测试 / Router 内省用
    #[must_use]
    pub fn inner(&self) -> &dyn Provider {
        self.inner.as_ref()
    }

    /// 提取 ChatError 分类; 非 ChatError 返回 None
    fn classify_error(err: &anyhow::Error) -> Option<&ChatError> {
        err.downcast_ref::<ChatError>()
    }

    /// chat / chat_stream 共用的重试循环 (同步语义, 不含 stream)
    async fn run_with_retry<F, Fut, T>(&self, op: F) -> Result<T>
    where
        F: Fn() -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
    {
        let mut last_err: Option<anyhow::Error> = None;
        for attempt in 0..=self.policy.max_retries {
            if attempt > 0 {
                let backoff = self.policy.compute_backoff(attempt);
                debug!(
                    attempt,
                    max_retries = self.policy.max_retries,
                    ?backoff,
                    "重试 chat 调用"
                );
                tokio::time::sleep(backoff).await;
            }
            match op().await {
                Ok(value) => return Ok(value),
                Err(err) => {
                    let retryable = Self::classify_error(&err)
                        .map(|e| e.is_retryable())
                        .unwrap_or(false);
                    if !retryable {
                        return Err(err);
                    }
                    warn!(
                        attempt,
                        max_retries = self.policy.max_retries,
                        error = %err,
                        "可重试错误, 将重试"
                    );
                    last_err = Some(err);
                }
            }
        }
        // 重试耗尽
        Err(last_err.unwrap_or_else(|| anyhow::anyhow!("重试耗尽但无错误")))
    }
}

impl Attributable for ReliableModelProvider {
    fn role(&self) -> Role {
        Role::Provider
    }
    fn alias(&self) -> &str {
        &self.alias
    }
}

#[async_trait]
impl Provider for ReliableModelProvider {
    fn provider_type(&self) -> &str {
        self.inner.provider_type()
    }

    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse> {
        let inner = Arc::clone(&self.inner);
        self.run_with_retry(|| {
            let req = request.clone();
            let inner = Arc::clone(&inner);
            async move { inner.chat(req).await }
        })
        .await
    }

    /// 流式 chat -- 只重试 pre-stream 错误 (建立连接失败).
    /// Ok(BoxStream) 返回后, mid-stream 错误直接透传, 不再重试.
    async fn chat_stream(
        &self,
        request: ChatRequest,
    ) -> Result<BoxStream<'static, Result<ChatChunk>>> {
        let mut last_err: Option<anyhow::Error> = None;
        for attempt in 0..=self.policy.max_retries {
            if attempt > 0 {
                let backoff = self.policy.compute_backoff(attempt);
                debug!(
                    attempt,
                    max_retries = self.policy.max_retries,
                    ?backoff,
                    "重试 chat_stream 连接"
                );
                tokio::time::sleep(backoff).await;
            }
            match self.inner.chat_stream(request.clone()).await {
                Ok(stream) => return Ok(stream),
                Err(err) => {
                    let retryable = Self::classify_error(&err)
                        .map(|e| e.is_retryable())
                        .unwrap_or(false);
                    if !retryable {
                        return Err(err);
                    }
                    warn!(
                        attempt,
                        max_retries = self.policy.max_retries,
                        error = %err,
                        "流连接可重试错误, 将重试"
                    );
                    last_err = Some(err);
                }
            }
        }
        Err(last_err.unwrap_or_else(|| anyhow::anyhow!("重试耗尽但无错误")))
    }

    async fn list_models(&self) -> Result<Vec<String>> {
        self.inner.list_models().await
    }

    fn supports_native_tools(&self) -> bool {
        self.inner.supports_native_tools()
    }

    fn default_temperature(&self) -> f64 {
        self.inner.default_temperature()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use futures::StreamExt;
    use shadow_core::ChatMessage;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Mutex;

    /// 测试用 mock provider -- 可注入错误序列
    struct MockProvider {
        name: String,
        /// 调用次数 → 返回值 (Ok=响应, Err=错误)
        /// 每次调用消费序列下一个元素
        responses: Mutex<Vec<Result<ChatResponse>>>,
        call_count: Arc<AtomicU32>,
    }

    impl MockProvider {
        fn new(name: &str, responses: Vec<Result<ChatResponse>>) -> Arc<Self> {
            Arc::new(Self {
                name: name.to_string(),
                responses: Mutex::new(responses),
                call_count: Arc::new(AtomicU32::new(0)),
            })
        }
    }

    impl Attributable for MockProvider {
        fn role(&self) -> Role {
            Role::Provider
        }
        fn alias(&self) -> &str {
            &self.name
        }
    }

    #[async_trait]
    impl Provider for MockProvider {
        fn provider_type(&self) -> &str {
            "mock"
        }

        async fn chat(&self, _request: ChatRequest) -> Result<ChatResponse> {
            self.call_count.fetch_add(1, Ordering::SeqCst);
            let mut guard = self.responses.lock().unwrap();
            if guard.is_empty() {
                return Err(anyhow::anyhow!("mock 序列耗尽"));
            }
            let result = guard.remove(0);
            drop(guard);
            result
        }

        async fn list_models(&self) -> Result<Vec<String>> {
            Ok(vec!["mock-model".to_string()])
        }
    }

    fn ok_response(content: &str) -> Result<ChatResponse> {
        Ok(ChatResponse {
            content: content.to_string(),
            tool_calls: vec![],
            usage: shadow_core::TokenUsage::default(),
            reasoning_content: None,
        })
    }

    fn transient_err() -> Result<ChatResponse> {
        Err(anyhow::Error::new(ChatError::from_status(
            reqwest::StatusCode::INTERNAL_SERVER_ERROR,
            "5xx".to_string(),
        )))
    }

    fn permanent_err() -> Result<ChatResponse> {
        Err(anyhow::Error::new(ChatError::from_status(
            reqwest::StatusCode::BAD_REQUEST,
            "400".to_string(),
        )))
    }

    fn make_request() -> ChatRequest {
        ChatRequest {
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: "hi".to_string(),
                ..Default::default()
            }],
            model: "test-model".to_string(),
            temperature: None,
            max_tokens: None,
            tools: vec![],
        }
    }

    #[tokio::test]
    async fn retry_on_transient_then_succeeds() {
        let mock = MockProvider::new("test", vec![transient_err(), ok_response("done")]);
        let calls = mock.call_count.clone();
        let reliable = ReliableModelProvider::new(
            "test",
            mock,
            RetryPolicy {
                max_retries: 3,
                initial_backoff_ms: 1,
                max_backoff_ms: 10,
                jitter_pct: 0,
            },
        );

        let resp = reliable.chat(make_request()).await.unwrap();
        assert_eq!(resp.content, "done");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn no_retry_on_permanent() {
        let mock = MockProvider::new(
            "test",
            vec![permanent_err(), ok_response("should-not-reach")],
        );
        let calls = mock.call_count.clone();
        let reliable = ReliableModelProvider::new("test", mock, RetryPolicy::default());

        let err = reliable.chat(make_request()).await.unwrap_err();
        let chat_err = err.downcast_ref::<ChatError>().unwrap();
        assert!(!chat_err.is_retryable());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn retries_exhausted_returns_last_error() {
        let mock = MockProvider::new(
            "test",
            vec![
                transient_err(),
                transient_err(),
                transient_err(),
                transient_err(), // max_retries=3 → 4 次尝试
            ],
        );
        let calls = mock.call_count.clone();
        let reliable = ReliableModelProvider::new(
            "test",
            mock,
            RetryPolicy {
                max_retries: 3,
                initial_backoff_ms: 1,
                max_backoff_ms: 10,
                jitter_pct: 0,
            },
        );

        let err = reliable.chat(make_request()).await.unwrap_err();
        let chat_err = err.downcast_ref::<ChatError>().unwrap();
        assert!(chat_err.is_retryable());
        assert_eq!(calls.load(Ordering::SeqCst), 4);
    }

    #[tokio::test]
    async fn zero_retries_means_single_attempt() {
        let mock = MockProvider::new("test", vec![transient_err()]);
        let calls = mock.call_count.clone();
        let reliable = ReliableModelProvider::new(
            "test",
            mock,
            RetryPolicy {
                max_retries: 0,
                initial_backoff_ms: 1,
                max_backoff_ms: 10,
                jitter_pct: 0,
            },
        );

        let _err = reliable.chat(make_request()).await.unwrap_err();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn backoff_is_increasing() {
        // jitter_pct=0 时退避是确定性的
        let policy = RetryPolicy {
            max_retries: 3,
            initial_backoff_ms: 50,
            max_backoff_ms: 1000,
            jitter_pct: 0,
        };
        // attempt=1: 50ms, attempt=2: 100ms, attempt=3: 200ms
        assert_eq!(policy.compute_backoff(1).as_millis(), 50);
        assert_eq!(policy.compute_backoff(2).as_millis(), 100);
        assert_eq!(policy.compute_backoff(3).as_millis(), 200);
    }

    #[test]
    fn backoff_respects_max_cap() {
        let policy = RetryPolicy {
            max_retries: 10,
            initial_backoff_ms: 1000,
            max_backoff_ms: 5000,
            jitter_pct: 0,
        };
        // attempt=10 应被 cap 到 5000ms
        let backoff = policy.compute_backoff(10);
        assert!(backoff.as_millis() <= 5000);
    }

    #[test]
    fn backoff_attempt_zero_is_zero() {
        let policy = RetryPolicy::default();
        assert_eq!(policy.compute_backoff(0), Duration::ZERO);
    }

    #[tokio::test]
    async fn stream_retries_only_pre_stream() {
        // MockProvider 没实现 chat_stream, 但通过 Provider trait 默认实现会调 chat()
        // 第一次 chat() 返回 transient_err → 重试
        // 第二次 chat() 返回 ok_response → 默认实现包成单个 chunk
        let mock = MockProvider::new("test", vec![transient_err(), ok_response("done")]);
        let calls = mock.call_count.clone();
        let reliable = ReliableModelProvider::new(
            "test",
            mock,
            RetryPolicy {
                max_retries: 3,
                initial_backoff_ms: 1,
                max_backoff_ms: 10,
                jitter_pct: 0,
            },
        );

        let stream = reliable.chat_stream(make_request()).await.unwrap();
        let chunks: Vec<_> = stream.collect().await;
        assert_eq!(chunks.len(), 1);
        let chunk = chunks[0].as_ref().unwrap();
        match chunk {
            ChatChunk::Done { content, .. } => assert_eq!(content, "done"),
            _ => panic!("应该是 Done chunk"),
        }
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn retry_policy_default_values() {
        let p = RetryPolicy::default();
        assert_eq!(p.max_retries, 3);
        assert_eq!(p.initial_backoff_ms, 1000);
        assert_eq!(p.max_backoff_ms, 60_000);
        assert_eq!(p.jitter_pct, 25);
    }

    #[test]
    fn attributable_implementation() {
        let mock = MockProvider::new("m", vec![]);
        let reliable = ReliableModelProvider::new("reliable-test", mock, RetryPolicy::default());
        assert_eq!(reliable.role(), Role::Provider);
        assert_eq!(reliable.alias(), "reliable-test");
        assert_eq!(reliable.provider_type(), "mock");
    }

    #[tokio::test]
    async fn non_chat_error_returned_immediately() {
        // 非 ChatError 的 anyhow::Error 也直接返回 (不重试)
        let plain_err = anyhow::anyhow!("some weird error");
        let mock = MockProvider::new("test", vec![Err(plain_err)]);
        let calls = mock.call_count.clone();
        let reliable = ReliableModelProvider::new("test", mock, RetryPolicy::default());

        let err = reliable.chat(make_request()).await.unwrap_err();
        assert!(err.downcast_ref::<ChatError>().is_none());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
