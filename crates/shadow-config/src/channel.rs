use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum StreamMode {
    #[default]
    Off,

    Partial,

    #[serde(rename = "multi_message")]
    MultiMessage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum LarkReceiveMode {
    #[default]
    Websocket,
    Webhook,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptionConfig {
    #[serde(default)]
    pub enabled: bool,
    pub api_key: Option<String>,
    pub api_url: String,
    pub model: String,
    pub language: Option<String>,
    pub initial_prompt: Option<String>,
    pub max_audio_bytes: Option<usize>,
    pub max_duration_secs: u64,
}
