use shadow_core::{Attributable, ChatMessage, ChatRequest, ChatResponse, ModelInfo, ModelProvider, ProviderCapabilities, Role, StreamChunk, StreamEvent, StreamOptions, ToolSpec, ToolsPayload};
use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::AtomicUsize;
use std::time::Instant;
use async_trait::async_trait;
use futures::stream::BoxStream;
use serde_json::Value;
use crate::ProviderDispatch;

pub struct ReliableModelProviderEntry {
    display_name: String,
    cooldown_key: String,
    provider: Box<dyn ModelProvider>,
}

impl ReliableModelProviderEntry {
    pub fn new(
        display_name: impl Into<String>,
        cooldown_key: impl Into<String>,
        provider: Box<dyn ModelProvider>,
    ) -> Self {
        Self {
            display_name: display_name.into(),
            cooldown_key: cooldown_key.into(),
            provider,
        }
    }
}

pub struct ReliableModelProvider {
    alias: String,
    model_providers: Vec<ReliableModelProviderEntry>,
    max_retries: u32,
    base_backoff_ms: u64,
    api_keys: Vec<String>,
    key_index: AtomicUsize,
    model_fallbacks: HashMap<String, Vec<String>>,
    rate_limit_cooldowns: Mutex<HashMap<String, Instant>>,
}

impl ReliableModelProvider {
    pub fn new(
        alias: &str,
        model_providers: Vec<(String, Box<dyn ModelProvider>)>,
        max_retries: u32,
        base_backoff_ms: u64,
    ) -> Self {
        let model_providers = model_providers
            .into_iter()
            .map(|(display_name, provider)| {
                ReliableModelProviderEntry::new(display_name.clone(), display_name, provider)
            })
            .collect();

        Self::new_with_entries(alias, model_providers, max_retries, base_backoff_ms)
    }

    pub fn new_with_entries(
        alias: &str,
        model_providers: Vec<ReliableModelProviderEntry>,
        max_retries: u32,
        base_backoff_ms: u64,
    ) -> Self {
        Self {
            alias: alias.to_string(),
            model_providers,
            max_retries,
            base_backoff_ms: base_backoff_ms.max(50),
            api_keys: Vec::new(),
            key_index: AtomicUsize::new(0),
            model_fallbacks: HashMap::new(),
            rate_limit_cooldowns: Mutex::new(HashMap::new()),
        }
    }
}

impl Attributable for ReliableModelProvider {
    fn role(&self) -> Role {
        Role::System
    }

    fn alias(&self) -> &str {
        &self.alias
    }
}

#[async_trait]
impl ModelProvider for ReliableModelProvider {
    async fn chat_with_system(&self, system_prompt: Option<&str>, message: &str, model: &str, temperature: Option<f64>) -> anyhow::Result<String> {
        ProviderDispatch::from_ref(self.model_providers[0].provider.as_ref())
            .chat_with_system(system_prompt, message, model, temperature)
            .await
    }
}
