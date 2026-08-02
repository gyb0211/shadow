//! MiniMax 文本转语音 Provider
//!
//! 使用 MiniMax T2A v2 API 合成高质量语音
//!
//! API 文档: https://platform.minimaxi.com/docs/api-reference/speech-t2a-http
//!
//! 支持模型:
//! - speech-2.8-hd (最新 HD)
//! - speech-2.8-turbo (最新 Turbo)
//! - speech-02-hd / speech-02-turbo
//! - speech-01-hd / speech-01-turbo
//!
//! 支持音色:
//! - male-qn-qingse (青涩男声)
//! - male-qn-jingying (精英男声)
//! - female-shaonv (少女女声)
//! - female-yujie (御姐女声)
//! - female-chengshu (成熟女声)
//! - preschool_boy / preschool_girl (童声)
//! - ...更多见 MiniMax 文档

use super::TtsProvider;
use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use shadow_core::kennel::attribution::Attributable;
use std::time::Duration;

/// MiniMax T2A v2 API 端点
const T2A_ENDPOINT: &str = "https://api.minimaxi.com/v1/t2a_v2";

/// 备用端点（北京）
const T2A_ENDPOINT_BJ: &str = "https://api-bj.minimaxi.com/v1/t2a_v2";

/// MiniMax TTS Provider
pub struct MiniMaxTtsProvider {
    /// API Key
    api_key: String,
    /// 模型名称
    model: String,
    /// HTTP 客户端
    client: reqwest::Client,
    /// 默认音色
    default_voice: String,
    /// 默认语速 (0.5-2.0)
    speed: f64,
    /// 默认音量 (0-10)
    vol: f64,
    /// 默认音调 (-12 到 +12)
    pitch: i32,
    /// 默认情感
    emotion: String,
    /// 语言增强
    language_boost: Option<String>,
    /// 使用北京端点
    use_beijing: bool,
}

/// 音色设置
#[derive(Debug, Clone, Serialize)]
struct VoiceSetting {
    voice_id: String,
    speed: f64,
    vol: f64,
    pitch: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    emotion: Option<String>,
}

/// 音频设置
#[derive(Debug, Clone, Serialize)]
struct AudioSetting {
    sample_rate: u32,
    bitrate: u32,
    format: String,
    channel: u32,
}

/// T2A 请求体
#[derive(Debug, Serialize)]
struct T2aRequest {
    model: String,
    text: String,
    stream: bool,
    voice_setting: VoiceSetting,
    audio_setting: AudioSetting,
    #[serde(skip_serializing_if = "Option::is_none")]
    language_boost: Option<String>,
}

/// T2A 响应体
#[derive(Debug, Deserialize)]
struct T2aResponse {
    data: Option<T2aData>,
    base_resp: T2aBaseResp,
}

/// T2A 响应数据
#[derive(Debug, Deserialize)]
struct T2aData {
    /// hex 编码的音频数据
    audio: String,
    status: i32,
}

/// T2A 响应状态
#[derive(Debug, Deserialize)]
struct T2aBaseResp {
    status_code: i32,
    status_msg: String,
}

impl MiniMaxTtsProvider {
    /// 创建 MiniMax TTS Provider
    ///
    /// 参数:
    /// - `api_key`: MiniMax API Key
    /// - `model`: 模型名称（如 "speech-2.8-hd"）
    /// - `default_voice`: 默认音色 ID
    pub fn new(
        api_key: impl Into<String>,
        model: impl Into<String>,
        default_voice: impl Into<String>,
    ) -> Result<Self> {
        let api_key = api_key.into();
        if api_key.trim().is_empty() {
            bail!("MiniMax TTS API key must not be empty");
        }

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(super::DEFAULT_TTS_TIMEOUT_SECS))
            .build()
            .context("Failed to build HTTP client for MiniMax TTS")?;

        Ok(Self {
            api_key,
            model: model.into(),
            client,
            default_voice: default_voice.into(),
            speed: 1.0,
            vol: 1.0,
            pitch: 0,
            emotion: String::new(),
            language_boost: None,
            use_beijing: false,
        })
    }

    /// 设置默认语速 (0.5-2.0)
    pub fn with_speed(mut self, speed: f64) -> Self {
        self.speed = speed;
        self
    }

    /// 设置默认音量 (0-10)
    pub fn with_volume(mut self, vol: f64) -> Self {
        self.vol = vol;
        self
    }

    /// 设置默认音调 (-12 到 +12)
    pub fn with_pitch(mut self, pitch: i32) -> Self {
        self.pitch = pitch;
        self
    }

    /// 设置默认情感
    /// 可选值: happy, sad, angry, fearful, disgusted, surprised, neutral
    pub fn with_emotion(mut self, emotion: impl Into<String>) -> Self {
        self.emotion = emotion.into();
        self
    }

    /// 设置语言增强
    /// 可选值: Chinese, Chinese,Yue (粤语), English, auto 等
    pub fn with_language_boost(mut self, lang: impl Into<String>) -> Self {
        self.language_boost = Some(lang.into());
        self
    }

    /// 使用北京端点（国内加速）
    pub fn use_beijing_endpoint(mut self) -> Self {
        self.use_beijing = true;
        self
    }

    /// 获取端点 URL
    fn endpoint(&self) -> &str {
        if self.use_beijing {
            T2A_ENDPOINT_BJ
        } else {
            T2A_ENDPOINT
        }
    }

    /// hex 字符串解码为字节数组
    fn hex_decode(hex: &str) -> Result<Vec<u8>> {
        let hex = hex.trim();
        if hex.len() % 2 != 0 {
            bail!("Hex string has odd length");
        }
        let mut bytes = Vec::with_capacity(hex.len() / 2);
        for i in (0..hex.len()).step_by(2) {
            let byte = u8::from_str_radix(&hex[i..i + 2], 16)
                .with_context(|| format!("Invalid hex at position {}", i))?;
            bytes.push(byte);
        }
        Ok(bytes)
    }
}

impl Attributable for MiniMaxTtsProvider {
    fn role(&self) -> shadow_core::kennel::attribution::Role {
        use shadow_core::kennel::attribution::*;
        Role::Provider(ProviderKind::Tts(TtsProviderKind::Plugin))
    }

    fn alias(&self) -> &str {
        "minimax_tts"
    }
}

#[async_trait]
impl TtsProvider for MiniMaxTtsProvider {
    fn name(&self) -> &str {
        "minimax"
    }

    fn output_format(&self) -> &str {
        "mp3"
    }

    async fn synthesize(&self, text: &str, voice: &str) -> Result<Vec<u8>> {
        if text.is_empty() {
            bail!("Text must not be empty");
        }

        if text.len() > 10000 {
            bail!("Text too long (max 10000 characters, got {})", text.len());
        }

        let voice_id = if voice.is_empty() {
            self.default_voice.clone()
        } else {
            voice.to_string()
        };

        let emotion = if self.emotion.is_empty() {
            None
        } else {
            Some(self.emotion.clone())
        };

        let request = T2aRequest {
            model: self.model.clone(),
            text: text.to_string(),
            stream: false,
            voice_setting: VoiceSetting {
                voice_id,
                speed: self.speed,
                vol: self.vol,
                pitch: self.pitch,
                emotion,
            },
            audio_setting: AudioSetting {
                sample_rate: 32000,
                bitrate: 128000,
                format: "mp3".to_string(),
                channel: 1,
            },
            language_boost: self.language_boost.clone(),
        };

        let resp = self
            .client
            .post(self.endpoint())
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .context("Failed to send MiniMax TTS request")?;

        let status = resp.status();
        let body: T2aResponse = resp
            .json()
            .await
            .context("Failed to parse MiniMax TTS response")?;

        if body.base_resp.status_code != 0 {
            bail!(
                "MiniMax TTS API error (status_code={}): {}",
                body.base_resp.status_code,
                body.base_resp.status_msg
            );
        }

        if !status.is_success() {
            bail!("MiniMax TTS HTTP error: {}", status);
        }

        let data = body.data.context("MiniMax TTS response missing data field")?;

        // 解码 hex 音频
        let audio_bytes = Self::hex_decode(&data.audio)
            .context("Failed to decode hex audio data")?;

        Ok(audio_bytes)
    }

    fn supported_voices(&self) -> Vec<String> {
        // MiniMax 常用音色
        vec![
            "male-qn-qingse".to_string(),      // 青涩男声
            "male-qn-jingying".to_string(),     // 精英男声
            "male-qn-badao".to_string(),        // 霸道男声
            "male-qn-daxuesheng".to_string(),   // 大学生男声
            "female-shaonv".to_string(),        // 少女女声
            "female-yujie".to_string(),         // 御姐女声
            "female-chengshu".to_string(),      // 成熟女声
            "female-tianmei".to_string(),       // 甜美女声
            "preschool_boy".to_string(),        // 男孩童声
            "preschool_girl".to_string(),       // 女孩童声
            "narrator_lady".to_string(),        // 女性旁白
            "narrator_man".to_string(),         // 男性旁白
            "news_lady".to_string(),            // 女性新闻
            "news_man".to_string(),             // 男性新闻
            "speaker_young_man".to_string(),    // 年轻男性演讲
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hex_decode() {
        let hex = "48656c6c6f";
        let bytes = MiniMaxTtsProvider::hex_decode(hex).unwrap();
        assert_eq!(bytes, b"Hello");
    }

    #[test]
    fn test_hex_decode_empty() {
        let bytes = MiniMaxTtsProvider::hex_decode("").unwrap();
        assert!(bytes.is_empty());
    }

    #[test]
    fn test_hex_decode_invalid() {
        let result = MiniMaxTtsProvider::hex_decode("xyz");
        assert!(result.is_err());
    }

    #[test]
    fn test_provider_creation() {
        let provider = MiniMaxTtsProvider::new(
            "test-key",
            "speech-2.8-hd",
            "male-qn-qingse",
        );
        assert!(provider.is_ok());
        let provider = provider.unwrap();
        assert_eq!(provider.name(), "minimax");
        assert_eq!(provider.output_format(), "mp3");
    }

    #[test]
    fn test_provider_empty_key() {
        let result = MiniMaxTtsProvider::new("", "speech-2.8-hd", "male-qn-qingse");
        assert!(result.is_err());
    }

    #[test]
    fn test_supported_voices() {
        let provider = MiniMaxTtsProvider::new(
            "test-key",
            "speech-2.8-hd",
            "male-qn-qingse",
        ).unwrap();
        let voices = provider.supported_voices();
        assert!(!voices.is_empty());
        assert!(voices.contains(&"male-qn-qingse".to_string()));
        assert!(voices.contains(&"female-shaonv".to_string()));
    }

    #[test]
    fn test_builder_methods() {
        let provider = MiniMaxTtsProvider::new(
            "test-key",
            "speech-2.8-hd",
            "male-qn-qingse",
        ).unwrap()
            .with_speed(1.2)
            .with_volume(1.5)
            .with_pitch(2)
            .with_emotion("happy")
            .with_language_boost("Chinese")
            .use_beijing_endpoint();

        assert_eq!(provider.speed, 1.2);
        assert_eq!(provider.vol, 1.5);
        assert_eq!(provider.pitch, 2);
        assert_eq!(provider.emotion, "happy");
        assert_eq!(provider.language_boost, Some("Chinese".to_string()));
        assert!(provider.use_beijing);
    }
}