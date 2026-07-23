//! 影子 LLM provider 实现
//!
//! 架构 (借鉴 zeroclaw 3 层):
//! - **Router** (顶层): 按 alias 路由到具体 family provider
//! - **Reliable** (中层): 重试/退避/key 轮换/限流
//! - **Compat** (底层): 把家族差异 (auth, API path, payload) 适配为统一 OpenAI 形态

pub mod catalog;
pub mod dispatch;
pub mod error;
pub mod factory;
mod models_dev;
pub mod openai;
pub mod rate_limit;
pub mod reliable;
pub mod reliable_bak;
pub mod router;
pub mod router_bak;

pub use dispatch::*;
pub use error::{ChatError, RetryClass};
pub use openai::OpenAiCompatibleModelProvider;
pub use rate_limit::TokenBucket;
use std::collections::HashMap;

use anyhow::Result;
use reliable::ReliableModelProvider;
use shadow_config::{Config, ModelProviderConfig, ReliableConfig, ModelRouteConfig};
use shadow_core::ModelProvider;

pub struct ModelProviderInfo {
    pub name: &'static str,
    pub display_name: &'static str,
    pub local: bool,
    pub category: ModelProviderCategory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelProviderCategory {
    Primary,
    OpenAiCompatible,
    FastInference,
    ModelHosting,
    ChineseAi,
    CloudEndpoint,
}

pub fn list_model_providers() -> Vec<ModelProviderInfo> {
    let mut out: Vec<ModelProviderInfo> = Vec::new();
    push_family(
        &mut out,
        ModelProviderCategory::Primary,
        &[
            ("openrouter", "OpenRouter", false),
            ("anthropic", "Anthropic", false),
            ("openai", "OpenAI", false),
            ("ollama", "Ollama", true),
            ("gemini", "Google Gemini", true),
        ],
    );

    push_family(
        &mut out,
        ModelProviderCategory::OpenAiCompatible,
        &[
            ("zai", "Z.AI", false),
            ("glm", "GLM", false),
            ("minimax", "MiniMax", false),
            ("qwen", "Qwen", false),
            ("deepseek", "Deepseek", false),
            ("qwen", "Qwen", false),
            ("custom", "Custom(OpenAI-compatible)", false),
        ],
    );

    out
}

fn push_family(
    out: &mut Vec<ModelProviderInfo>,
    category: ModelProviderCategory,
    families: &[(&'static str, &'static str, bool)],
) {
    out.extend(
        families
            .iter()
            .map(|&(name, display_name, local)| ModelProviderInfo {
                name,
                display_name,
                local,
                category,
            }),
    );
}

pub fn create_model_provider(
    name: &str,
    api_key: Option<&str>,
    api_url: Option<&str>,
) -> Result<Box<dyn ModelProvider>> {
    create_model_provider_inner(
        None,
        name,
        "default",
        api_key,
        api_url,
        &ModelProviderRuntimeOptions::default(),
    )
}

fn create_model_provider_inner(
    config: Option<&shadow_config::schema::Config>,
    raw_name: &str,
    alias: &str,
    api_key: Option<&str>,
    api_url: Option<&str>,
    options: &ModelProviderRuntimeOptions,
) -> Result<Box<dyn ModelProvider>> {
    if let Some(idx) = raw_name.find(":") {}

    // todo!() url形式的模型配置
    let provider_kind = options
        .provider_kind
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(raw_name);

    let resolved_credential = resolve_model_provider_credential(provider_kind, api_key)
        .map(|v| String::from_utf8(v.into_bytes()).unwrap_or_default());

    let key = resolved_credential.as_ref().map(String::as_str);

    let resolved_url = api_url
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .or_else(|| {
            options
                .provider_api_url
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty())
        });

    factory::dispatch_family_factory(config, provider_kind, alias, key, resolved_url, options)
}

fn resolve_model_provider_credential(
    _name: &str,
    credential_override: Option<&str>,
) -> Option<String> {
    credential_override
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToString::to_string)
}

pub fn create_routed_model_provider_with_options(
    config: &Config,
    primary_name: &str,
    api_key: Option<&str>,
    api_url: Option<&str>,
    reliability: &ReliableConfig,
    model_routes: &[ModelRouteConfig],
    default_model: &str,
    options: &ModelProviderRuntimeOptions,
) -> anyhow::Result<Box<dyn ModelProvider>> {
    if model_routes.is_empty() {
        return create_resilient_model_provider_from_ref(
            config,
            primary_name,
            api_key,
            api_url,
            reliability,
            options,
        );
    }

    // todo return RouterModelProvider

    return create_resilient_model_provider_from_ref(
        config,
        primary_name,
        api_key,
        api_url,
        reliability,
        options,
    );
}

fn create_resilient_model_provider_from_ref(
    config: &Config,
    name: &str,
    api_key: Option<&str>,
    api_url: Option<&str>,
    reliability: &ReliableConfig,
    options: &ModelProviderRuntimeOptions,
) -> anyhow::Result<Box<dyn ModelProvider>> {
    match name.split_once('.') {
        Some((family, alias)) => create_resilient_model_provider_for_alias(
            config,
            family,
            alias,
            api_key,
            api_url,
            reliability,
            options,
        ),
        None => create_resilient_model_provider_with_options(
            name,
            api_key,
            api_url,
            reliability,
            options,
        ),
    }
}

pub fn create_resilient_model_provider_for_alias(
    config: &Config,
    family: &str,
    alias: &str,
    api_key: Option<&str>,
    api_url: Option<&str>,
    reliability: &ReliableConfig,
    options: &ModelProviderRuntimeOptions,
) -> anyhow::Result<Box<dyn ModelProvider>> {
    // todo 先用普通的代替
    create_resilient_model_provider_with_options(
        format!("{family}.{alias}").as_str(),
        api_key,
        api_url,
        reliability,
        options,
    )
}

pub fn create_resilient_model_provider_with_options(
    name: &str,
    api_key: Option<&str>,
    api_url: Option<&str>,
    reliability: &ReliableConfig,
    options: &ModelProviderRuntimeOptions,
) -> anyhow::Result<Box<dyn ModelProvider>> {
    let model_provider =
        create_model_provider_inner(None, name, "default", api_key, api_url, options)?;
    let reliable = ReliableModelProvider::new(
        name,
        vec![(name.to_string(), model_provider)],
        reliability.provider_retries,
        reliability.provider_backoff_ms,
    );
    // .with_api_keys(reliability.api_keys.clone());
    Ok(Box::new(reliable))
}

pub fn provider_runtime_options_for_alias(
    config: &Config,
    family: &str,
    alias: &str,
) -> ModelProviderRuntimeOptions {
    let entry = config.providers.models.find(family, alias);
    let mut options = model_provider_runtime_options_from_model_provider_entry(config, entry);
    if options.provider_api_url.is_none()
        && let Some(uri) = config.providers.models.resolved_endpoint_uri(family, alias)
    {
        options.provider_api_url = Some(uri.to_string())
    }
    options
}

pub fn provider_runtime_options_for_agent(
    config: &Config,
    agent_alias: &str,
) -> ModelProviderRuntimeOptions {
    let entry = config.model_provider_for_agent(agent_alias);
    let mut options = model_provider_runtime_options_from_model_provider_entry(config, entry);
    if let Some(agent) = config.agents.get(agent_alias)
        && let Some((family, alias)) = agent.model_provider.split_once('.')
    {
        if options.provider_api_url.is_none()
            && let Some(uri) = config.providers.models.resolved_endpoint_uri(family, alias)
        {
            options.provider_api_url = Some(uri.to_string());
        }
    }
    options
}

pub fn model_provider_runtime_options_from_model_provider_entry(
    config: &Config,
    entry: Option<&ModelProviderConfig>,
) -> ModelProviderRuntimeOptions {
    // let merge_system_to_user = entry.and_then(|e| e.uri.as_deref())
    //     .map(str::trim).filter(|u| !u.is_empty())
    //     .and_then(|u|{
    //         config.providers.models.iter_entries()
    //             .map(|(_,_,base)| base)
    //             .find(|c| {
    //                 c.uri.as_deref().map(str::trim).filter(|u|!u.is_empty())
    //                     .map(|u| u.trim_end_matches('/')) == Some(u.trim_end_matches('/'))
    //             })
    //     }).map(|c| c.merge_system_into_user).unwrap_or(false);
    //
    // let tls_ca_cert_path = entry.and_then(|c|c.tls_ca_cert_path.clone());
    ModelProviderRuntimeOptions {
        provider_kind: entry.and_then(|e| {
            e.kind
                .as_deref()
                .map(str::trim)
                .filter(|k| !k.is_empty())
                .map(str::to_string)
        }),
        provider_api_url: entry.and_then(|e| e.uri.clone()),
        native_tools: entry.and_then(|e| e.native_tools),
        provider_timeout_secs: Some(entry.and_then(|e| e.timeout_secs).unwrap_or(120)),
        reasoning_effort: config.runtime.reasoning_effort.clone(),
        api_path: None,
        extra_headers: entry.map(|e| e.extra_headers.clone()).unwrap_or_default(),
    }
}

/// Provider 运行时选项 -- 注入 HTTP 层细节
///
/// 由 factory (shadow-providers::create_provider) 接收, 透传给 Compat 层.
/// 设计为 Option-heavy: MVP 阶段大部分字段为 None, 未来 Reliable 层 / 推理控制会填充.
///
/// 注: `extra_headers` 用 `HashMap<String, String>` 而非 `reqwest::HeaderMap`,
/// 是为了让 shadow-core 保持 HTTP-agnostic (不依赖 reqwest). shadow-providers
/// 在调用 reqwest 时做一次转换.
#[derive(Debug, Clone, Default)]
pub struct ModelProviderRuntimeOptions {
    pub provider_kind: Option<String>,

    pub provider_api_url: Option<String>,

    pub native_tools: Option<bool>,

    /// HTTP 请求超时 (None = reqwest 默认)
    pub provider_timeout_secs: Option<u64>,
    /// 推理强度 (如 "low" / "medium" / "high"), OpenAI o-series / Anthropic 用
    pub reasoning_effort: Option<String>,
    /// 自定义 API path 后缀 (None = 各 family 默认, 如 "/chat/completions")
    pub api_path: Option<String>,
    /// 附加 HTTP headers (会与 auth header 合并)
    pub extra_headers: HashMap<String, String>,
}

pub fn create_model_provider_with_options(
    name: &str,
    api_key: Option<&str>,
    opts: &ModelProviderRuntimeOptions,
) -> anyhow::Result<Box<dyn ModelProvider>> {
    create_model_provider_inner(None, name, "default", api_key, None, opts)
}
