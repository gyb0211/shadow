//! Router -- 按 alias 路由到具体 family provider
//!
//! 3 层架构的顶层。Agent 只面对 `dyn Provider`, 不感知后端是 OpenAiProvider
//! 还是未来的 AnthropicNative。Router 接收请求, 按 model 字段中的 hint (MVP 阶段)
//! 或 default 转发到注册的 inner provider.
//!
//! MVP 行为:
//! - model = "hint:reasoning" → 查 routes 表, 路由到指定 provider + 替换 model 名
//! - model = "gpt-4o" → default provider, model 名原样透传
//! - 未注册 hint 调用返回 default provider
//!
//! 未来扩展 (Phase 2+):
//! - 按 ChatRequest 中的 alias 字段动态路由
//! - ReliableModelProvider 包裹 (重试/退避/key 轮换)

use crate::dispatch::ProviderDispatch;
use anyhow::Result;
use async_trait::async_trait;
use futures::stream::BoxStream;
use shadow_core::provider::ChatChunk;
use shadow_core::{Attributable, ChatRequest, ChatResponse, Provider, Role};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct Route {
    pub provider_name: String, // 要跟 model_providers 里的 name 对上
    pub model: String,         // 实际下发给该 provider 的 model 字符串
}

/// 路由器 -- 按 model hint 路由 Provider 调用
pub struct RouterModelProvider {
    alias: String,
    /// 例: "reasoning" → (2, "claude-opus-4")
    routes: HashMap<String, (usize, String)>,

    model_providers: Vec<(String, Box<dyn Provider>)>,

    default_index: usize,

    /// 保留默认模型名供 Phase 2 cost-optimized 路由使用
    #[allow(dead_code)]
    default_model: String,
}

impl RouterModelProvider {
    /// 构造器 -- 指定默认 alias (后续 register 必须包含此 alias)
    #[must_use]
    pub fn new(
        default_alias: impl Into<String>,
        model_providers: Vec<(String, Box<dyn Provider>)>,
        routes: Vec<(String, Route)>,
        default_model: String,
    ) -> Self {
        let name_to_index: HashMap<&str, usize> = model_providers
            .iter()
            .enumerate()
            .map(|(i, (name, _))| (name.as_str(), i))
            .collect();

        let resolve_routes = routes
            .into_iter()
            .filter_map(|(hint, route)| {
                name_to_index
                    .get(route.provider_name.as_str())
                    .copied()
                    .map(|index| (hint, (index, route.model)))
            })
            .collect();

        Self {
            alias: default_alias.into(),
            routes: resolve_routes,
            model_providers,
            default_index: 0,
            default_model,
        }
    }

    // 协议约定:
    //
    //     - model = "hint:reasoning" → 查 routes 表
    //     - model = "gpt-4o" → default provider, model 名原样透传下去
    fn resolve(&self, model: &str) -> (usize, String) {
        if let Some(hint) = model.strip_prefix("hint:")
            && let Some((idx, resolve_model)) = self.routes.get(hint)
        {
            return (*idx, resolve_model.clone());
        }

        (self.default_index, model.to_string())
    }

    /// 借用默认 provider (用于非路由方法: provider_type / list_models / ...)
    fn default_provider(&self) -> &dyn Provider {
        let (_, provider) = &self.model_providers[self.default_index];
        &**provider
    }
}

impl Attributable for RouterModelProvider {
    fn role(&self) -> Role {
        Role::Provider
    }
    fn alias(&self) -> &str {
        &self.alias
    }
}

#[async_trait]
impl Provider for RouterModelProvider {
    fn provider_type(&self) -> &str {
        self.default_provider().provider_type()
    }

    async fn chat(&self, mut request: ChatRequest) -> Result<ChatResponse> {
        let (provider_idx, resolved_model) = self.resolve(&request.model);
        request.model = resolved_model;
        let (_, provider) = &self.model_providers[provider_idx];
        ProviderDispatch::from_ref(&**provider).chat(request).await
    }

    async fn chat_stream(
        &self,
        mut request: ChatRequest,
    ) -> Result<BoxStream<'static, Result<ChatChunk>>> {
        let (provider_idx, resolved_model) = self.resolve(&request.model);
        request.model = resolved_model;
        let (_, provider) = &self.model_providers[provider_idx];
        ProviderDispatch::from_ref(&**provider)
            .chat_stream(request)
            .await
    }

    async fn list_models(&self) -> Result<Vec<String>> {
        self.default_provider().list_models().await
    }

    fn supports_native_tools(&self) -> bool {
        self.default_provider().supports_native_tools()
    }

    fn default_temperature(&self) -> f64 {
        self.default_provider().default_temperature()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 最小 mock provider -- 用于路由测试
    struct MockProvider {
        name: String,
        models: Vec<String>,
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
            Ok(ChatResponse {
                content: format!("mock reply from {}", self.name),
                tool_calls: vec![],
                usage: shadow_core::TokenUsage::default(),
                reasoning_content: None,
            })
        }

        async fn list_models(&self) -> Result<Vec<String>> {
            Ok(self.models.clone())
        }

        fn supports_native_tools(&self) -> bool {
            true
        }
    }

    #[tokio::test]
    async fn router_routes_to_default() {
        let provider = Box::new(MockProvider {
            name: "default".to_string(),
            models: vec!["gpt-4o".to_string()],
        }) as Box<dyn Provider>;

        let router = RouterModelProvider::new(
            "test",
            vec![("default".to_string(), provider)],
            vec![],
            "gpt-4o".to_string(),
        );

        let req = ChatRequest {
            messages: vec![],
            model: "gpt-4o".to_string(),
            temperature: None,
            max_tokens: None,
            tools: vec![],
        };
        let resp = router.chat(req).await.unwrap();
        assert!(resp.content.contains("default"));
    }

    #[tokio::test]
    async fn router_routes_by_hint() {
        let default_provider = Box::new(MockProvider {
            name: "cheap".to_string(),
            models: vec!["gpt-4o-mini".to_string()],
        }) as Box<dyn Provider>;

        let reasoning_provider = Box::new(MockProvider {
            name: "smart".to_string(),
            models: vec!["o1".to_string()],
        }) as Box<dyn Provider>;

        let router = RouterModelProvider::new(
            "test",
            vec![
                ("cheap".to_string(), default_provider),
                ("smart".to_string(), reasoning_provider),
            ],
            vec![(
                "reasoning".to_string(),
                Route {
                    provider_name: "smart".to_string(),
                    model: "o1".to_string(),
                },
            )],
            "gpt-4o-mini".to_string(),
        );

        // hint:reasoning → smart provider
        let req = ChatRequest {
            messages: vec![],
            model: "hint:reasoning".to_string(),
            temperature: None,
            max_tokens: None,
            tools: vec![],
        };
        let resp = router.chat(req).await.unwrap();
        assert!(resp.content.contains("smart"));

        // 无 hint → default (cheap) provider
        let req2 = ChatRequest {
            messages: vec![],
            model: "gpt-4o-mini".to_string(),
            temperature: None,
            max_tokens: None,
            tools: vec![],
        };
        let resp2 = router.chat(req2).await.unwrap();
        assert!(resp2.content.contains("cheap"));
    }

    #[tokio::test]
    async fn router_delegates_list_models() {
        let provider = Box::new(MockProvider {
            name: "default".to_string(),
            models: vec!["gpt-4o".to_string(), "gpt-4o-mini".to_string()],
        }) as Box<dyn Provider>;

        let router = RouterModelProvider::new(
            "test",
            vec![("default".to_string(), provider)],
            vec![],
            "gpt-4o".to_string(),
        );

        let models = router.list_models().await.unwrap();
        assert_eq!(models, vec!["gpt-4o", "gpt-4o-mini"]);
    }

    #[test]
    fn router_alias_and_role() {
        let provider = Box::new(MockProvider {
            name: "default".to_string(),
            models: vec![],
        }) as Box<dyn Provider>;

        let router = RouterModelProvider::new(
            "my-router",
            vec![("default".to_string(), provider)],
            vec![],
            "gpt-4o".to_string(),
        );

        assert_eq!(router.alias(), "my-router");
        assert_eq!(router.role(), Role::Provider);
    }
}
