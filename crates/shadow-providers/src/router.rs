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

use crate::dispatch::ProviderDispatch;
use anyhow::Result;
use async_trait::async_trait;
use futures::stream::BoxStream;
use serde_json::Value;
use shadow_core::provider::{StreamEvent, StreamOptions, StreamResult};
use shadow_core::{
    Attributable, ChatMessage, ChatRequest, ChatResponse, ModelProvider, Role,
};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct Route {
    pub provider_name: String, // 要跟 model_providers 里的 name 对上
    pub model: String,         // 实际下发给该 provider 的 model 字符串
}

/// 路由器 -- 按 alias 路由 ModelProvider 调用
pub struct RouterModelProvider {
    alias: String,
    ///  例："reasoning" → (2, "claude-opus-4")
    routes: HashMap<String, (usize, String)>,

    model_providers: Vec<(String, Box<dyn ModelProvider>)>,

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
        model_providers: Vec<(String, Box<dyn ModelProvider>)>,
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

    //    协议约定：
    //
    //     - model = "hint:reasoning" → 查 routes 表
    //     - model = "hint:cheapest"  → 走 resolve_cost_optimized()（另一个方法，MVP 可以砍）
    //     - model = "gpt-4o" → default provider，model 名原样透传下去
    fn resolve(&self, model: &str) -> (usize, String) {
        if let Some(hint) = model.strip_prefix("hint:")
            && let Some((idx, resolve_model)) = self.routes.get(hint)
        {
            return (*idx, resolve_model.clone());
        }

        (self.default_index, model.to_string())
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
impl ModelProvider for RouterModelProvider {
    async fn chat_with_system(
        &self,
        system_prompt: Option<&str>,
        message: &str,
        model: &str,
        temperature: Option<f64>,
    ) -> Result<String> {
        let (provider_idx, resolve_model) = self.resolve(model);
        let (_, model_provider) = &self.model_providers[provider_idx];
        ProviderDispatch::from_ref(&**model_provider)
            .chat_with_system(system_prompt, message, &resolve_model, temperature)
            .await
    }

    async fn chat_with_history(
        &self,
        messages: &[ChatMessage],
        model: &str,
        temperature: Option<f64>,
    ) -> Result<String> {
        let (provider_idx, resolve_model) = self.resolve(model);
        let (_, model_provider) = &self.model_providers[provider_idx];
        ProviderDispatch::from_ref(&**model_provider)
            .chat_with_history(messages, &resolve_model, temperature)
            .await
    }

    async fn chat(
        &self,
        request: ChatRequest<'_>,
        model: &str,
        temperature: Option<f64>,
    ) -> Result<ChatResponse> {
        let (provider_idx, resolve_model) = self.resolve(model);
        let (_, model_provider) = &self.model_providers[provider_idx];
        ProviderDispatch::from_ref(&**model_provider)
            .chat(request, &resolve_model, temperature)
            .await
    }

    async fn chat_with_tools(
        &self,
        messages: &[ChatMessage],
        tools: &[Value],
        model: &str,
        temperature: Option<f64>,
    ) -> Result<ChatResponse> {
        let (provider_idx, resolve_model) = self.resolve(model);
        let (_, model_provider) = &self.model_providers[provider_idx];
        ProviderDispatch::from_ref(&**model_provider)
            .chat_with_tools(messages, tools, &resolve_model, temperature)
            .await
    }

    fn stream_chat(
        &self,
        request: ChatRequest<'_>,
        model: &str,
        temperature: Option<f64>,
        options: StreamOptions,
    ) -> BoxStream<'static, StreamResult<StreamEvent>> {
        let (provider_idx, resolve_model) = self.resolve(model);
        let (_, model_provider) = &self.model_providers[provider_idx];
        ProviderDispatch::from_ref(&**model_provider).stream_chat(
            request,
            &resolve_model,
            temperature,
            options,
        )
    }

    fn stream_chat_with_system(
        &self,
        system_prompt: Option<&str>,
        message: &str,
        model: &str,
        temperature: Option<f64>,
        options: StreamOptions,
    ) -> BoxStream<'static, StreamResult<StreamEvent>> {
        let (provider_idx, resolve_model) = self.resolve(model);
        let (_, model_provider) = &self.model_providers[provider_idx];
        ProviderDispatch::from_ref(&**model_provider).stream_chat_with_system(
            system_prompt,
            message,
            &resolve_model,
            temperature,
            options,
        )
    }

    fn stream_chat_with_history(
        &self,
        messages: &[ChatMessage],
        model: &str,
        temperature: Option<f64>,
        options: StreamOptions,
    ) -> BoxStream<'static, StreamResult<StreamEvent>> {
        let (provider_idx, resolve_model) = self.resolve(model);
        let (_, model_provider) = &self.model_providers[provider_idx];
        ProviderDispatch::from_ref(&**model_provider).stream_chat_with_history(
            messages,
            &resolve_model,
            temperature,
            options,
        )
    }
}
