//! 文本转语音（Text-to-Speech）Provider 抽象
//!
//! 支持多种 TTS 后端：
//! - MiniMax T2A（在线，高质量）
//! - Piper（本地离线）
//! - Edge TTS（免费）

use anyhow::Result;
use async_trait::async_trait;
use shadow_core::kennel::attribution::Attributable;

pub mod minimax;
pub use minimax::MiniMaxTtsProvider;

/// 文本转语音 Provider trait
#[async_trait]
pub trait TtsProvider: Send + Sync + Attributable {
    /// Provider 名称（如 "minimax", "piper", "edge"）
    fn name(&self) -> &str;

    /// 合成语音
    ///
    /// 参数:
    /// - `text`: 要合成的文本
    /// - `voice`: 音色 ID
    ///
    /// 返回: 音频字节数据
    async fn synthesize(&self, text: &str, voice: &str) -> Result<Vec<u8>>;

    /// 返回音频格式（如 "mp3", "wav", "opus"）
    fn output_format(&self) -> &str;

    /// 支持的音色列表
    fn supported_voices(&self) -> Vec<String>;
}

/// 默认 HTTP 超时时间（TTS 合成可能较慢）
const DEFAULT_TTS_TIMEOUT_SECS: u64 = 60;