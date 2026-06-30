//! 影子 LLM provider 实现
//!
//! 当前实现: OpenAI 兼容 (支持 OpenAI/OpenRouter/Ollama 等)

pub mod openai;

use agent_core::ModelProvider;
use anyhow::Result;
use std::sync::Arc;

/// 工厂函数 -- 按类型名创建 provider
pub fn create_provider(
    provider_type: &str,
    api_key: Option<&str>,
    base_url: Option<&str>,
) -> Result<Arc<dyn ModelProvider>> {
    match provider_type {
        "openai" | "openrouter" | "ollama" | "compatible" => {
            openai::OpenAiProvider::new(provider_type, api_key, base_url).map(|p| Arc::new(p) as Arc<dyn ModelProvider>)
        }
        _ => anyhow::bail!("未知的 provider 类型: {provider_type}"),
    }
}
