//! Router -- 按 alias 路由到具体 family provider
//!
//! 3 层架构的顶层。Agent 只面对 `dyn ModelProvider`,不感知后端是 OpenAiCompat
//! 还是未来的 AnthropicNative。Router 接收请求,按 default_alias (MVP 阶段) 转发到
//! 注册的 inner provider.
//!
//! MVP 行为:
//! - 只服务 `default_alias` 对应的 provider
//! - 未注册 alias 调用返回 Err
//!
//! 未来扩展 (Phase 2+):
//! - 按 ChatRequest 中的 alias 字段动态路由
//! - ReliableModelProvider 包裹 (重试/退避/key 轮换)

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use futures::stream::BoxStream;
use shadow_core::{
    Attributable, ChatChunk, ChatRequest, ChatResponse, ModelProvider, Role,
};
use std::collections::HashMap;
use std::sync::Arc;

/// 路由器 -- 按 alias 路由 ModelProvider 调用
pub struct Router {
    default_alias: String,
    routes: HashMap<String, Arc<dyn ModelProvider>>,
}

impl Router {
    /// 构造器 -- 指定默认 alias (后续 register 必须包含此 alias)
    #[must_use]
    pub fn new(default_alias: impl Into<String>) -> Self {
        Self {
            default_alias: default_alias.into(),
            routes: HashMap::new(),
        }
    }

    /// 注册一个 provider 到指定 alias
    pub fn register(&mut self, alias: &str, provider: Arc<dyn ModelProvider>) {
        self.routes.insert(alias.to_string(), provider);
    }

    /// 按 alias 取出 provider; 不存在返回 Err
    pub fn route(&self, alias: &str) -> Result<&Arc<dyn ModelProvider>> {
        self.routes
            .get(alias)
            .ok_or_else(|| anyhow!("未注册的 provider alias: {alias}"))
    }

    /// 取默认 alias 的 provider; 未注册返回 Err
    fn default_provider(&self) -> Result<&Arc<dyn ModelProvider>> {
        self.route(&self.default_alias)
    }
}

impl Attributable for Router {
    fn role(&self) -> Role {
        Role::Provider
    }
    fn alias(&self) -> &str {
        &self.default_alias
    }
}

#[async_trait]
impl ModelProvider for Router {
    fn provider_type(&self) -> &str {
        "router"
    }

    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse> {
        let provider = self.default_provider()?;
        provider.chat(request).await
    }

    async fn chat_stream(
        &self,
        request: ChatRequest,
    ) -> Result<BoxStream<'static, Result<ChatChunk>>> {
        let provider = self.default_provider()?;
        provider.chat_stream(request).await
    }

    async fn list_models(&self) -> Result<Vec<String>> {
        let provider = self.default_provider()?;
        provider.list_models().await
    }

    fn supports_native_tools(&self) -> bool {
        self.default_provider()
            .map(|p| p.supports_native_tools())
            .unwrap_or(false)
    }

    fn supports_vision(&self) -> bool {
        self.default_provider()
            .map(|p| p.supports_vision())
            .unwrap_or(false)
    }

    fn default_temperature(&self) -> f64 {
        self.default_provider()
            .map(|p| p.default_temperature())
            .unwrap_or(0.7)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shadow_core::TokenUsage;

    /// 测试用 Mock Provider -- 记录被调用情况
    struct MockProvider {
        alias: String,
        family: String,
        supports_tools: bool,
    }

    impl Attributable for MockProvider {
        fn role(&self) -> Role {
            Role::Provider
        }
        fn alias(&self) -> &str {
            &self.alias
        }
    }

    #[async_trait]
    impl ModelProvider for MockProvider {
        fn provider_type(&self) -> &str {
            &self.family
        }
        async fn chat(&self, _request: ChatRequest) -> Result<ChatResponse> {
            Ok(ChatResponse {
                content: format!("mock-reply-from-{}", self.alias),
                tool_calls: vec![],
                usage: TokenUsage::default(),
                reasoning_content: None,
            })
        }
        fn supports_native_tools(&self) -> bool {
            self.supports_tools
        }
    }

    fn mock(alias: &str, family: &str, tools: bool) -> Arc<MockProvider> {
        Arc::new(MockProvider {
            alias: alias.to_string(),
            family: family.to_string(),
            supports_tools: tools,
        })
    }

    #[tokio::test]
    async fn router_dispatches_to_registered_alias() {
        let mut router = Router::new("openai.default");
        router.register(
            "openai.default",
            mock("openai.default", "openai", false) as Arc<dyn ModelProvider>,
        );
        let resp = router
            .chat(ChatRequest {
                messages: vec![],
                model: "gpt-4o-mini".to_string(),
                temperature: None,
                max_tokens: None,
                tools: vec![],
            })
            .await
            .unwrap();
        assert_eq!(resp.content, "mock-reply-from-openai.default");
    }

    #[tokio::test]
    async fn router_unregistered_default_errors() {
        // 没 register 任何 provider 就调用 -> Err
        let router = Router::new("nonexistent.alias");
        let result = router
            .chat(ChatRequest {
                messages: vec![],
                model: "x".to_string(),
                temperature: None,
                max_tokens: None,
                tools: vec![],
            })
            .await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("nonexistent.alias"), "err = {err}");
    }

    #[test]
    fn router_supports_native_tools_delegates() {
        let mut router = Router::new("openai.default");
        router.register(
            "openai.default",
            mock("openai.default", "openai", true) as Arc<dyn ModelProvider>,
        );
        assert!(router.supports_native_tools());

        let mut router2 = Router::new("openai.default");
        router2.register(
            "openai.default",
            mock("openai.default", "openai", false) as Arc<dyn ModelProvider>,
        );
        assert!(!router2.supports_native_tools());
    }

    #[test]
    fn router_provider_type_returns_router() {
        let router = Router::new("openai.default");
        assert_eq!(router.provider_type(), "router");
    }
}
