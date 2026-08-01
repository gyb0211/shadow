use async_trait::async_trait;
use shadow_core::Attributable;
use std::collections::HashMap;

use anyhow::{Result, bail};

#[async_trait]
pub trait TranscriptionProvider: Send + Sync + Attributable {
    fn name(&self) -> &str;

    async fn transcribe(&self, audio_data: &[u8], file_name: &str) -> anyhow::Result<String>;

    fn supported_formats(&self) -> Vec<String> {
        vec![
            "flac", "mp3", "mpeg", "mpga", "mp4", "m4a", "ogg", "oga", "opus", "wav", "webm",
        ]
        .into_iter()
        .map(String::from)
        .collect()
    }
}

// ── LocalWhisper 适配器 ─────────────────────────────────────────

/// 将 shadow_providers::LocalWhisperProvider 适配为 channel TranscriptionProvider
pub struct LocalWhisperAdapter {
    provider: shadow_providers::LocalWhisperProvider,
}

impl LocalWhisperAdapter {
    pub fn new(provider: shadow_providers::LocalWhisperProvider) -> Self {
        Self { provider }
    }

    /// 使用默认配置创建（自动查找 whisper.cpp 和模型）
    pub fn with_defaults() -> Result<Self> {
        let provider = shadow_providers::LocalWhisperProvider::with_defaults()
            .map_err(|e| anyhow::anyhow!("Failed to init local whisper: {}", e))?;
        Ok(Self { provider })
    }
}

impl Attributable for LocalWhisperAdapter {
    fn role(&self) -> shadow_core::kennel::attribution::Role {
        use shadow_core::kennel::attribution::*;
        Role::Provider(ProviderKind::Transcription(TranscriptionProviderKind::Google))
    }

    fn alias(&self) -> &str {
        "local_whisper"
    }
}

#[async_trait]
impl TranscriptionProvider for LocalWhisperAdapter {
    fn name(&self) -> &str {
        "local_whisper"
    }

    async fn transcribe(&self, audio_data: &[u8], file_name: &str) -> Result<String> {
        use shadow_providers::TranscriptionProvider;
        self.provider.transcribe(audio_data, file_name).await
    }

    fn supported_formats(&self) -> Vec<String> {
        use shadow_providers::TranscriptionProvider;
        self.provider.supported_formats()
    }
}

pub struct TranscriptionManager {
    transcription_providers: HashMap<String, Box<dyn TranscriptionProvider>>,
    max_audio_bytes: Option<usize>,
    agent_transcription_provider: String,
}

impl TranscriptionManager {
    /// 创建空的 TranscriptionManager
    pub fn new() -> Self {
        Self {
            transcription_providers: HashMap::new(),
            max_audio_bytes: Some(25 * 1024 * 1024), // 25 MB 默认
            agent_transcription_provider: String::new(),
        }
    }

    /// 注册一个 transcription provider
    pub fn register(&mut self, alias: String, provider: Box<dyn TranscriptionProvider>) {
        if self.agent_transcription_provider.is_empty() {
            self.agent_transcription_provider = alias.clone();
        }
        self.transcription_providers.insert(alias, provider);
    }

    /// 使用 LocalWhisper 创建 TranscriptionManager（便捷方法）
    pub fn with_local_whisper() -> Result<Self> {
        let adapter = LocalWhisperAdapter::with_defaults()?;
        let mut manager = Self::new();
        manager.register("local_whisper".to_string(), Box::new(adapter));
        Ok(manager)
    }

    /// 设置 agent 使用的 transcription provider
    pub fn set_agent_provider(&mut self, alias: &str) {
        self.agent_transcription_provider = alias.to_string();
    }

    pub async fn transcribe(&self, audio_data: &[u8], file_name: &str) -> Result<String> {
        let provider_alias = self.agent_transcription_provider.as_str();
        if provider_alias.is_empty() {
            bail!(
                "Agent has no transcription_provider configured. Set \
                 `agent.<alias>.transcription_provider = \"<type>.<alias>\"` \
                 referencing a configured transcription provider."
            );
        }
        self.transcribe_with_provider(audio_data, file_name, provider_alias)
            .await
    }

    /// Transcribe audio using a specific named transcription_provider.
    pub async fn transcribe_with_provider(
        &self,
        audio_data: &[u8],
        file_name: &str,
        transcription_provider: &str,
    ) -> Result<String> {
        let p = self.transcription_providers.get(transcription_provider).ok_or_else(|| {
            let available: Vec<&str> = self.transcription_providers.keys().map(|k| k.as_str()).collect();
            ::shadow_log::record!(
                ERROR,
                ::shadow_log::Event::new(module_path!(), ::shadow_log::Action::Reject)
                    .with_outcome(::shadow_log::EventOutcome::Failure)
                    .with_attrs(::serde_json::json!({
                        "transcription_provider": transcription_provider,
                        "available": available,
                    })),
                "transcription: provider not configured"
            );
            anyhow::Error::msg(format!(
                "Transcription transcription_provider '{transcription_provider}' not configured. Available: {available:?}"
            ))
        })?;

        self.enforce_global_audio_limit(audio_data)?;

        use ::shadow_log::Instrument;
        let span = ::shadow_log::attribution_span!(p.as_ref());
        p.transcribe(audio_data, file_name).instrument(span).await
    }

    fn enforce_global_audio_limit(&self, audio_data: &[u8]) -> Result<()> {
        if let Some(max_audio_bytes) = self.max_audio_bytes
            && audio_data.len() > max_audio_bytes
        {
            bail!(
                "Audio file too large ({} bytes, global max {})",
                audio_data.len(),
                max_audio_bytes
            );
        }
        Ok(())
    }
}
