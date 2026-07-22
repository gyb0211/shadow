//! 影子 LLM provider 实现
//!
//! 架构 (借鉴 zeroclaw 3 层):
//! - **Router** (顶层): 按 alias 路由到具体 family provider
//! - **Reliable** (中层): 重试/退避/key 轮换/限流
//! - **Compat** (底层): 把家族差异 (auth, API path, payload) 适配为统一 OpenAI 形态

pub mod dispatch;
pub mod error;
pub mod factory;
pub mod openai;
pub mod rate_limit;
pub mod reliable;
pub mod router;
pub mod catalog;
mod models_dev;

use std::collections::HashMap;
pub use dispatch::*;
pub use error::{ChatError, RetryClass};
pub use openai::OpenAiCompatibleModelProvider;
pub use rate_limit::TokenBucket;

use anyhow::Result;
use shadow_core::{ModelProvider, ModelProviderRuntimeOptions};
use std::sync::Arc;
use shadow_config::{Config, ModelProviderConfig};

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
    push_family(&mut out, ModelProviderCategory::Primary, &[
        ("openrouter", "OpenRouter", false),
        ("anthropic", "Anthropic", false),
        ("openai", "OpenAI", false),
        ("ollama", "Ollama", true),
        ("gemini", "Google Gemini", true),
    ]);

    push_family(&mut out, ModelProviderCategory::OpenAiCompatible, &[
        ("zai", "Z.AI", false),
        ("glm", "GLM", false),
        ("minimax", "MiniMax", false),
        ("qwen", "Qwen", false),
        ("deepseek", "Deepseek", false),
        ("qwen", "Qwen", false),
        ("custom", "Custom(OpenAI-compatible)", false),
    ]);


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
            })
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

pub fn provider_runtime_options_for_alias(config: &Config, family: &str, alias: &str) -> ModelProviderRuntimeOptions {
    let entry = config.providers.models.find(family, alias);
    let mut options = model_provider_runtime_options_from_model_provider_entry(config, entry);
    if options.provider_api_url.is_none() && let Some(uri) = config.providers.models.resolved_endpoint_uri(family, alias){

    }
    options
}

pub fn model_provider_runtime_options_from_model_provider_entry(
    config: &Config,
    entry: Option<&ModelProviderConfig>
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
    ModelProviderRuntimeOptions{
        provider_kind: entry.and_then(|e| {e.kind.as_deref().map(str::trim).filter(|k| !k.is_empty()).map(str::to_string)}),
        provider_api_url: entry.and_then(|e| e.uri.clone()),
        native_tools: entry.and_then(|e| e.native_tools),
        provider_timeout_secs: Some(entry.and_then(|e| e.timeout_secs).unwrap_or(120)),
        reasoning_effort: config.runtime.reasoning_effort.clone(),
        api_path: None,
        extra_headers: entry.map(|e| e.extra_headers.clone()).unwrap_or_default(),
    }
}