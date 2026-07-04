//! Router -- 按 alias 路由到具体 family provider + 跨 provider fallback
//!
//! 3 层架构的顶层。Agent 只面对 `dyn Provider`, 不感知后端是 OpenAiProvider
//! 还是未来的 AnthropicNative。Router 接收请求, 按 model 字段中的 hint (MVP 阶段)
//! 或 default 转发到注册的 inner provider.
//!
//! 行为:
//! - model = "hint:reasoning" → 查 routes 表, 路由到指定 provider + 替换 model 名
//! - model = "gpt-4o" → default provider, model 名原样透传
//! - 未注册 hint 调用返回 default provider
//! - 主 provider chat/chat_stream 返回 Err → 按 fallback_chains 依次尝试备选 provider
//!
//! 关键: Router 看到的 inner 通常是 Reliable-wrapped, 内部已耗尽重试才到这.
//! Router 不分类错误, 任何 Err 都触发 fallback (除非 chain 也耗尽).

use crate::dispatch::ProviderDispatch;
use anyhow::Result;
use async_trait::async_trait;
use futures::stream::BoxStream;
use shadow_core::provider::ChatChunk;
use shadow_core::{Attributable, ChatRequest, ChatResponse, Provider, Role};
use std::collections::HashMap;
use tracing::{debug, info, warn};

#[derive(Debug, Clone)]
pub struct Route {
    pub provider_name: String, // 要跟 model_providers 里的 name 对上
    pub model: String,         // 实际下发给该 provider 的 model 字符串
}

/// 路由器 -- 按 model hint 路由 Provider 调用 + 跨 provider fallback
pub struct RouterModelProvider {
    alias: String,
    /// 例: "reasoning" → (2, "claude-opus-4")
    routes: HashMap<String, (usize, String)>,

    model_providers: Vec<(String, Box<dyn Provider>)>,

    default_index: usize,

    /// hint (或 "default") → 备选 provider index 列表
    ///
    /// 当主 provider 失败时, 按 chain 顺序依次尝试. chain 中的 provider 用各自
    /// 默认 model (不再做 hint→model 替换).
    fallback_chains: HashMap<String, Vec<usize>>,

    /// 保留默认模型名 (chat_via_router 用)
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
        Self::with_fallback_chains(
            default_alias,
            model_providers,
            routes,
            default_model,
            Vec::new(),
        )
    }

    /// 完整构造器 (含 fallback_chains)
    ///
    /// `fallback_chains: Vec<(hint_or_"default", Vec<provider_name>)>` --
    /// 主 provider 失败时按此顺序尝试备选. 未指定的 hint 回退到 "default" chain.
    #[must_use]
    pub fn with_fallback_chains(
        default_alias: impl Into<String>,
        model_providers: Vec<(String, Box<dyn Provider>)>,
        routes: Vec<(String, Route)>,
        default_model: String,
        fallback_chains: Vec<(String, Vec<String>)>,
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

        let resolve_chains = fallback_chains
            .into_iter()
            .filter_map(|(key, names)| {
                let idxs: Vec<usize> = names
                    .iter()
                    .filter_map(|n| name_to_index.get(n.as_str()).copied())
                    .collect();
                if idxs.is_empty() {
                    None
                } else {
                    Some((key, idxs))
                }
            })
            .collect();

        Self {
            alias: default_alias.into(),
            routes: resolve_routes,
            model_providers,
            default_index: 0,
            fallback_chains: resolve_chains,
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

    /// 取 hint 的 fallback chain. 优先 hint 专属 chain, 否则回退到 "default".
    fn fallback_chain_for(&self, model: &str) -> Option<&[usize]> {
        let hint = model.strip_prefix("hint:").unwrap_or("default");
        self.fallback_chains
            .get(hint)
            .or_else(|| self.fallback_chains.get("default"))
            .map(Vec::as_slice)
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

    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse> {
        let (provider_idx, resolved_model) = self.resolve(&request.model);
        let mut last_err: Option<anyhow::Error>;

        // 先尝试主 provider
        {
            let (_, provider) = &self.model_providers[provider_idx];
            let mut req = request.clone();
            req.model = resolved_model;
            match ProviderDispatch::from_ref(&**provider).chat(req).await {
                Ok(resp) => return Ok(resp),
                Err(err) => {
                    warn!(provider_idx, error = %err, "主 provider 失败, 尝试 fallback chain");
                    last_err = Some(err);
                }
            }
        }

        // 走 fallback chain
        if let Some(chain) = self.fallback_chain_for(&request.model) {
            for &idx in chain {
                if idx == provider_idx {
                    continue; // 主 provider 已试过, 不重复
                }
                let (name, provider) = &self.model_providers[idx];
                info!(fallback = %name, "切换到 fallback provider");
                // chain 中的 provider 用 request.model 原值 (不再做 hint→model 替换)
                match ProviderDispatch::from_ref(&**provider).chat(request.clone()).await {
                    Ok(resp) => return Ok(resp),
                    Err(err) => {
                        warn!(fallback = %name, error = %err, "fallback provider 也失败");
                        last_err = Some(err);
                    }
                }
            }
        }

        Err(last_err.unwrap_or_else(|| anyhow::anyhow!("router: 所有 provider 均失败 (chat)")))
    }

    async fn chat_stream(
        &self,
        request: ChatRequest,
    ) -> Result<BoxStream<'static, Result<ChatChunk>>> {
        // 流式: 只在 pre-stream 阶段尝试 fallback. Ok(stream) 后不再 fallback.
        let (provider_idx, resolved_model) = self.resolve(&request.model);
        let mut last_err: Option<anyhow::Error>;

        {
            let (_, provider) = &self.model_providers[provider_idx];
            let mut req = request.clone();
            req.model = resolved_model;
            match ProviderDispatch::from_ref(&**provider).chat_stream(req).await {
                Ok(stream) => return Ok(stream),
                Err(err) => {
                    warn!(provider_idx, error = %err, "主 provider stream 建立失败, 尝试 fallback");
                    last_err = Some(err);
                }
            }
        }

        if let Some(chain) = self.fallback_chain_for(&request.model) {
            for &idx in chain {
                if idx == provider_idx {
                    continue;
                }
                let (name, provider) = &self.model_providers[idx];
                debug!(fallback = %name, "stream 切换到 fallback provider");
                match ProviderDispatch::from_ref(&**provider)
                    .chat_stream(request.clone())
                    .await
                {
                    Ok(stream) => return Ok(stream),
                    Err(err) => {
                        warn!(fallback = %name, error = %err, "stream fallback 失败");
                        last_err = Some(err);
                    }
                }
            }
        }

        Err(last_err.unwrap_or_else(|| anyhow::anyhow!("router: 所有 stream provider 均失败")))
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

    // ── Phase 4: 跨 provider fallback chain ──

    /// 失败 mock -- 总是返回指定错误
    struct FailProvider {
        name: String,
        err_msg: String,
    }

    impl Attributable for FailProvider {
        fn role(&self) -> Role {
            Role::Provider
        }
        fn alias(&self) -> &str {
            &self.name
        }
    }

    #[async_trait]
    impl Provider for FailProvider {
        fn provider_type(&self) -> &str {
            "fail"
        }
        async fn chat(&self, _: ChatRequest) -> Result<ChatResponse> {
            Err(anyhow::anyhow!("{}", self.err_msg))
        }
        async fn list_models(&self) -> Result<Vec<String>> {
            Err(anyhow::anyhow!("{}", self.err_msg))
        }
    }

    fn req_with_model(model: &str) -> ChatRequest {
        ChatRequest {
            messages: vec![],
            model: model.to_string(),
            temperature: None,
            max_tokens: None,
            tools: vec![],
        }
    }

    #[tokio::test]
    async fn fallback_chain_walks_on_error() {
        // primary 失败, fallback 成功
        let primary = Box::new(FailProvider {
            name: "primary".to_string(),
            err_msg: "primary down".to_string(),
        }) as Box<dyn Provider>;
        let backup = Box::new(MockProvider {
            name: "backup".to_string(),
            models: vec![],
        }) as Box<dyn Provider>;

        let router = RouterModelProvider::with_fallback_chains(
            "test",
            vec![
                ("primary".to_string(), primary),
                ("backup".to_string(), backup),
            ],
            vec![],
            "gpt-4o".to_string(),
            vec![("default".to_string(), vec!["backup".to_string()])],
        );

        let resp = router.chat(req_with_model("gpt-4o")).await.unwrap();
        assert!(resp.content.contains("backup"));
    }

    #[tokio::test]
    async fn fallback_chain_exhausted_returns_last_error() {
        // 所有 provider 都失败
        let primary = Box::new(FailProvider {
            name: "primary".to_string(),
            err_msg: "primary down".to_string(),
        }) as Box<dyn Provider>;
        let backup = Box::new(FailProvider {
            name: "backup".to_string(),
            err_msg: "backup down".to_string(),
        }) as Box<dyn Provider>;

        let router = RouterModelProvider::with_fallback_chains(
            "test",
            vec![
                ("primary".to_string(), primary),
                ("backup".to_string(), backup),
            ],
            vec![],
            "gpt-4o".to_string(),
            vec![("default".to_string(), vec!["backup".to_string()])],
        );

        let err = router.chat(req_with_model("gpt-4o")).await.unwrap_err();
        // 最后一个错误是 backup
        assert!(err.to_string().contains("backup down"));
    }

    #[tokio::test]
    async fn fallback_chain_uses_default_for_unknown_hint() {
        // hint 无专属 chain, 回退到 "default" chain
        let primary = Box::new(FailProvider {
            name: "primary".to_string(),
            err_msg: "primary down".to_string(),
        }) as Box<dyn Provider>;
        let backup = Box::new(MockProvider {
            name: "backup".to_string(),
            models: vec![],
        }) as Box<dyn Provider>;

        let router = RouterModelProvider::with_fallback_chains(
            "test",
            vec![
                ("primary".to_string(), primary),
                ("backup".to_string(), backup),
            ],
            vec![],
            "gpt-4o".to_string(),
            vec![("default".to_string(), vec!["backup".to_string()])],
        );

        // 用一个无 hint 的 model, 走 default provider + default chain
        let resp = router.chat(req_with_model("gpt-4o")).await.unwrap();
        assert!(resp.content.contains("backup"));
    }

    #[tokio::test]
    async fn fallback_chain_hint_specific_overrides_default() {
        // hint "reasoning" 有专属 chain, 优先于 default chain
        let primary = Box::new(MockProvider {
            name: "primary".to_string(),
            models: vec![],
        }) as Box<dyn Provider>;
        let smart = Box::new(MockProvider {
            name: "smart".to_string(),
            models: vec![],
        }) as Box<dyn Provider>;

        let router = RouterModelProvider::with_fallback_chains(
            "test",
            vec![
                ("primary".to_string(), primary),
                ("smart".to_string(), smart),
            ],
            vec![(
                "reasoning".to_string(),
                Route {
                    provider_name: "smart".to_string(),
                    model: "o1".to_string(),
                },
            )],
            "gpt-4o".to_string(),
            // hint=reasoning 的 fallback chain 是空 (没有备选), default chain 也不存在
            vec![],
        );

        // hint:reasoning → smart provider → ok
        let resp = router.chat(req_with_model("hint:reasoning")).await.unwrap();
        assert!(resp.content.contains("smart"));
    }

    #[tokio::test]
    async fn no_fallback_chain_means_primary_only() {
        // 没配 fallback chain, primary 失败就立即返回错误
        let primary = Box::new(FailProvider {
            name: "primary".to_string(),
            err_msg: "no chain".to_string(),
        }) as Box<dyn Provider>;

        let router = RouterModelProvider::new(
            "test",
            vec![("primary".to_string(), primary)],
            vec![],
            "gpt-4o".to_string(),
        );

        let err = router.chat(req_with_model("gpt-4o")).await.unwrap_err();
        assert!(err.to_string().contains("no chain"));
    }
}
