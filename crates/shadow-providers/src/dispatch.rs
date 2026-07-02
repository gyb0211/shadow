//! ProviderDispatch -- 归因 span 自动包裹层
//!
//! Router 通过此层调用 inner provider, 每个 chat/stream 调用都被自动包进
//! `shadow_log::attribution_span!` span, 让 telemetry/log 拿到 provider 归因.
//!
//! 关键: span 必须用 `tracing::Instrument::instrument()` 注入 future/stream,
//! 不能只 `let span = ...; async { }.await` —— 那样 span 创建即销毁, 归因失效.

use anyhow::Result;
use futures::stream::{unfold, BoxStream};
use futures::StreamExt;
use shadow_core::{
    ChatMessage, ChatRequest, ChatResponse, ModelProvider,
};
use shadow_core::provider::{StreamEvent, StreamOptions, StreamResult};
use std::sync::Arc;
use tracing::Instrument;

/// Arc-backed dispatch -- 持有 provider 所有权
///
/// 当前主路径用 `ProviderDispatchRef` (零开销借用). 保留此 Arc-backed 版本
/// 供 Phase 2 Reliable 层 / 异步任务所有权场景使用.
pub struct ProviderDispatch {
    #[allow(dead_code)]
    inner: Arc<dyn ModelProvider>,
}

/// 引用 dispatch -- 短期借用, 零开销
pub struct ProviderDispatchRef<'a> {
    inner: &'a dyn ModelProvider,
}

impl ProviderDispatch {
    #[must_use]
    pub fn new(inner: Arc<dyn ModelProvider>) -> Self {
        Self { inner }
    }

    /// 从引用构造 dispatch (Router 用: model_providers[i].1 是 Box<dyn ModelProvider>)
    #[must_use]
    pub fn from_ref(inner: &dyn ModelProvider) -> ProviderDispatchRef<'_> {
        ProviderDispatchRef { inner }
    }
}

impl ProviderDispatchRef<'_> {
    /// 单轮对话 (无历史, 可带 system prompt)
    pub async fn chat_with_system(
        &self,
        system_prompt: Option<&str>,
        message: &str,
        model: &str,
        temperature: Option<f64>,
    ) -> Result<String> {
        let span = shadow_log::attribution_span!(&self.inner);
        self.inner
            .chat_with_system(system_prompt, message, model, temperature)
            .instrument(span)
            .await
    }

    /// 带历史的单轮对话 (返回纯文本)
    pub async fn chat_with_history(
        &self,
        messages: &[ChatMessage],
        model: &str,
        temperature: Option<f64>,
    ) -> Result<String> {
        let span = shadow_log::attribution_span!(&self.inner);
        self.inner
            .chat_with_history(messages, model, temperature)
            .instrument(span)
            .await
    }

    /// 完整 ChatRequest (含 tools) -- 返回 ChatResponse (含 tool_calls/usage)
    pub async fn chat(
        &self,
        request: ChatRequest<'_>,
        model: &str,
        temperature: Option<f64>,
    ) -> Result<ChatResponse> {
        let span = shadow_log::attribution_span!(&self.inner);
        self.inner
            .chat(request, model, temperature)
            .instrument(span)
            .await
    }

    /// 带工具的对话
    pub async fn chat_with_tools(
        &self,
        messages: &[ChatMessage],
        tools: &[serde_json::Value],
        model: &str,
        temperature: Option<f64>,
    ) -> Result<ChatResponse> {
        let span = shadow_log::attribution_span!(&self.inner);
        self.inner
            .chat_with_tools(messages, tools, model, temperature)
            .instrument(span)
            .await
    }

    /// 流式对话 -- span 通过 unfold 在每次 poll 时 enter, 保证流处理过程在归因 span 内
    pub fn stream_chat(
        &self,
        request: ChatRequest<'_>,
        model: &str,
        temperature: Option<f64>,
        options: StreamOptions,
    ) -> BoxStream<'static, StreamResult<StreamEvent>> {
        let span = shadow_log::attribution_span!(&self.inner);
        let inner = self
            .inner
            .stream_chat(request, model, temperature, options);
        instrument_stream(inner, span)
    }

    pub fn stream_chat_with_system(
        &self,
        system_prompt: Option<&str>,
        message: &str,
        model: &str,
        temperature: Option<f64>,
        options: StreamOptions,
    ) -> BoxStream<'static, StreamResult<StreamEvent>> {
        let span = shadow_log::attribution_span!(&self.inner);
        let inner = self.inner.stream_chat_with_system(
            system_prompt,
            message,
            model,
            temperature,
            options,
        );
        instrument_stream(inner, span)
    }

    pub fn stream_chat_with_history(
        &self,
        messages: &[ChatMessage],
        model: &str,
        temperature: Option<f64>,
        options: StreamOptions,
    ) -> BoxStream<'static, StreamResult<StreamEvent>> {
        let span = shadow_log::attribution_span!(&self.inner);
        let inner = self
            .inner
            .stream_chat_with_history(messages, model, temperature, options);
        instrument_stream(inner, span)
    }
}

/// 用 unfold 包裹流, 在每次 poll 时 enter span -- 保证 inner provider 的
/// tracing 事件都归因到此 provider 的 span.
fn instrument_stream(
    inner: BoxStream<'static, StreamResult<StreamEvent>>,
    span: tracing::Span,
) -> BoxStream<'static, StreamResult<StreamEvent>> {
    unfold(
        inner,
        move |mut s: BoxStream<'static, StreamResult<StreamEvent>>| {
            let span = span.clone();
            async move {
                let _guard = span.enter();
                s.next().await.map(|item| (item, s))
            }
        },
    )
    .boxed()
}
