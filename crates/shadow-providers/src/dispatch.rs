//! ProviderDispatch -- 归因 span 自动包裹层
//!
//! Router 通过此层调用 inner provider, 每个 chat/stream 调用都被自动包进
//! `shadow_log::attribution_span!` span, 让 telemetry/log 拿到 provider 归因.
//!
//! 关键: span 必须用 `tracing::Instrument::instrument()` 注入 future/stream,
//! 不能只 `let span = ...; async { }.await` —— 那样 span 创建即销毁, 归因失效.

use anyhow::Result;
use shadow_core::{ChatRequest, ChatResponse, ModelProvider};
use std::sync::Arc;
use tracing::Instrument;

/// Arc-backed dispatch -- 持有 provider 所有权
///
/// 当前主路径用 `ProviderDispatchRef` (零开销借用). 保留此 Arc-backed 版本
/// 供 Phase 2 Reliable 层 / 异步任务所有权场景使用.
pub struct ProviderDispatch {
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

    /// 借用为 ProviderDispatchRef -- 所有方法零开销委托
    #[must_use]
    pub fn as_ref(&self) -> ProviderDispatchRef<'_> {
        ProviderDispatchRef {
            inner: self.inner.as_ref(),
        }
    }

    /// 同步聊天 -- 返回完整 ChatResponse (含 tool_calls/usage)
    pub async fn chat(&self, request: ChatRequest<'_>) -> Result<ChatResponse> {
        self.as_ref().chat(request).await
    }
    /// 同步聊天 -- 返回完整 ChatResponse (含 tool_calls/usage)
    pub async fn simple_chat(&self, message: &str, model: &str, temperature: Option<f64>) -> Result<String> {
        self.as_ref().simple_chat(message, model, temperature).await
    }

    /// 列出可用模型
    pub async fn list_models(&self) -> Result<Vec<String>> {
        self.as_ref().list_models().await
    }
}

impl ProviderDispatchRef<'_> {
    /// 同步聊天 -- 自动包裹归因 span
    pub async fn chat(&self, request: ChatRequest<'_>) -> Result<ChatResponse> {
        let span = shadow_log::attribution_span!(&*self.inner);
        let model = request.model.clone();
        let temperature = request.temperature;
        self.inner.chat(request, &model, temperature).instrument(span).await
    }

    pub async fn simple_chat(&self, message: &str, model: &str, temperature: Option<f64>) -> Result<String> {
        self.inner.simple_chat(message, model, temperature).await
    }

    /// 列出可用模型 -- 自动包裹归因 span
    pub async fn list_models(&self) -> Result<Vec<String>> {
        let span = shadow_log::attribution_span!(&*self.inner);
        self.inner.list_models().instrument(span).await
    }
}
