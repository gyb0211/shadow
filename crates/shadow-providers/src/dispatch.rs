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
use shadow_core::provider::ChatChunk;
use shadow_core::{ChatRequest, ChatResponse, Provider};
use std::sync::Arc;
use tracing::Instrument;

/// Arc-backed dispatch -- 持有 provider 所有权
///
/// 当前主路径用 `ProviderDispatchRef` (零开销借用). 保留此 Arc-backed 版本
/// 供 Phase 2 Reliable 层 / 异步任务所有权场景使用.
pub struct ProviderDispatch {
    inner: Arc<dyn Provider>,
}

/// 引用 dispatch -- 短期借用, 零开销
pub struct ProviderDispatchRef<'a> {
    inner: &'a dyn Provider,
}

impl ProviderDispatch {
    #[must_use]
    pub fn new(inner: Arc<dyn Provider>) -> Self {
        Self { inner }
    }

    /// 从引用构造 dispatch (Router 用: model_providers[i].1 是 Box<dyn Provider>)
    #[must_use]
    pub fn from_ref(inner: &dyn Provider) -> ProviderDispatchRef<'_> {
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
    pub async fn chat(&self, request: ChatRequest) -> Result<ChatResponse> {
        self.as_ref().chat(request).await
    }

    /// 流式聊天 -- 返回 BoxStream, 逐块推送 ChatChunk
    pub async fn chat_stream(
        &self,
        request: ChatRequest,
    ) -> Result<BoxStream<'static, Result<ChatChunk>>> {
        self.as_ref().chat_stream(request).await
    }

    /// 列出可用模型
    pub async fn list_models(&self) -> Result<Vec<String>> {
        self.as_ref().list_models().await
    }
}

impl ProviderDispatchRef<'_> {
    /// 同步聊天 -- 自动包裹归因 span
    pub async fn chat(&self, request: ChatRequest) -> Result<ChatResponse> {
        let span = shadow_log::attribution_span!(&self.inner);
        self.inner.chat(request).instrument(span).await
    }

    /// 流式聊天 -- span 通过 unfold 在每次 poll 时 enter, 保证流处理过程在归因 span 内
    pub async fn chat_stream(
        &self,
        request: ChatRequest,
    ) -> Result<BoxStream<'static, Result<ChatChunk>>> {
        let span = shadow_log::attribution_span!(&self.inner);
        let inner = self.inner.chat_stream(request).instrument(span.clone()).await?;
        Ok(instrument_stream(inner, span))
    }

    /// 列出可用模型 -- 自动包裹归因 span
    pub async fn list_models(&self) -> Result<Vec<String>> {
        let span = shadow_log::attribution_span!(&self.inner);
        self.inner.list_models().instrument(span).await
    }
}

/// 用 unfold 包裹流, 在每次 poll 时 enter span -- 保证 inner provider 的
/// tracing 事件都归因到此 provider 的 span.
fn instrument_stream(
    inner: BoxStream<'static, Result<ChatChunk>>,
    span: tracing::Span,
) -> BoxStream<'static, Result<ChatChunk>> {
    unfold(
        inner,
        move |mut s: BoxStream<'static, Result<ChatChunk>>| {
            let span = span.clone();
            async move {
                let _guard = span.enter();
                s.next().await.map(|item| (item, s))
            }
        },
    )
    .boxed()
}
