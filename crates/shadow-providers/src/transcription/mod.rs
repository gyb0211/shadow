//! 语音转文本（Speech-to-Text）Provider 抽象
//!
//! 支持多种 STT 后端：
//! - 本地 whisper.cpp（离线）
//! - OpenAI Whisper API
//! - Google Cloud STT
//! - Groq Whisper API

use async_trait::async_trait;
use anyhow::Result;
use shadow_core::kennel::attribution::Attributable;

pub mod local_whisper;
pub use local_whisper::LocalWhisperProvider;

/// 语音转文本 Provider trait
#[async_trait]
pub trait TranscriptionProvider: Send + Sync + Attributable {
    /// Provider 名称（如 "local_whisper", "openai", "groq"）
    fn name(&self) -> &str;

    /// 转录音频数据
    /// 
    /// 参数:
    /// - `audio_data`: 音频字节数据
    /// - `file_name`: 文件名（包含扩展名，用于格式检测）
    /// 
    /// 返回: 转录后的文本
    async fn transcribe(&self, audio_data: &[u8], file_name: &str) -> Result<String>;

    /// 支持的音频格式（文件扩展名列表）
    fn supported_formats(&self) -> Vec<String>;
}

/// 音频格式工具函数
pub mod audio_format {
    /// 根据文件扩展名获取 MIME 类型
    pub fn mime_for_extension(extension: &str) -> Option<&'static str> {
        match extension.to_ascii_lowercase().as_str() {
            "flac" => Some("audio/flac"),
            "mp3" | "mpeg" | "mpga" => Some("audio/mpeg"),
            "mp4" | "m4a" => Some("audio/mp4"),
            "ogg" | "oga" => Some("audio/ogg"),
            "opus" => Some("audio/opus"),
            "wav" => Some("audio/wav"),
            "webm" => Some("audio/webm"),
            _ => None,
        }
    }

    /// 从文件名解析 MIME 类型
    pub fn mime_from_filename(file_name: &str) -> Option<&'static str> {
        file_name
            .rsplit_once('.')
            .and_then(|(_, ext)| mime_for_extension(ext))
    }

    /// 标准化文件名（处理 .oga → .ogg 等）
    pub fn normalize_filename(file_name: &str) -> String {
        match file_name.rsplit_once('.') {
            Some((stem, ext)) if ext.eq_ignore_ascii_case("oga") => {
                format!("{}.ogg", stem)
            }
            _ => file_name.to_string(),
        }
    }
}

/// 默认音频格式列表
pub fn default_supported_formats() -> Vec<String> {
    vec![
        "flac", "mp3", "mpeg", "mpga", "mp4", "m4a",
        "ogg", "oga", "opus", "wav", "webm",
    ]
    .into_iter()
    .map(String::from)
    .collect()
}