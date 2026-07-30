use async_trait::async_trait;
use shadow_core::Attributable;
use std::collections::HashMap;

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

pub struct TranscriptionManager {
    transcription_provider: HashMap<String, Box<dyn TranscriptionProvider>>,
    max_audio_bytes: Option<usize>,
    agent_transcription_provider: String,
}
