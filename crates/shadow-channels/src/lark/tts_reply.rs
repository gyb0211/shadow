//! Lark TTS 语音回复 -- 将 agent 文本回复转为语音消息发送
//!
//! 流程:
//! 1. MiniMax TTS 合成 MP3
//! 2. ffmpeg 转码为 Opus（飞书语音消息要求）
//! 3. 上传到飞书文件服务器
//! 4. 发送 audio 类型消息

use anyhow::{Context, Result, bail};
use std::sync::Arc;

use super::LarkChannel;
use crate::transcription::TranscriptionManager;

impl LarkChannel {
    /// 判断文本是否适合语音回复
    ///
    /// 排除不适合语音的内容：代码、链接、JSON、错误信息、太短的文本
    pub fn is_suitable_for_voice(content: &str) -> bool {
        // 太短不转
        if content.len() <= 20 {
            return false;
        }
        // 纯 URL 不转
        if content.starts_with("http") {
            return false;
        }
        // JSON 不转
        if content.starts_with('{') || content.starts_with('[') {
            return false;
        }
        // 错误信息不转
        if content.starts_with("Error") || content.starts_with("error") {
            return false;
        }
        // 有代码块不转
        if content.contains("```") {
            return false;
        }
        // 工具调用不转
        if content.contains("tool_call") {
            return false;
        }

        true
    }

    /// 发送语音回复
    ///
    /// 完整流程: TTS合成 → ffmpeg转opus → 上传飞书 → 发送audio消息
    pub async fn send_voice_reply(
        &self,
        recipient: &str,
        text: &str,
        tts_api_key: &str,
    ) -> Result<()> {
        self.send_voice_reply_with_config(recipient, text, tts_api_key, "female-shaonv", "speech-02-hd", 1.0, 1.0, 0).await
    }

    /// 发送语音回复（带完整 TTS 配置）
    pub async fn send_voice_reply_with_config(
        &self,
        recipient: &str,
        text: &str,
        tts_api_key: &str,
        voice: &str,
        model: &str,
        speed: f64,
        vol: f64,
        pitch: i32,
    ) -> Result<()> {
        if text.is_empty() {
            bail!("Cannot synthesize empty text");
        }

        // 1. MiniMax TTS 合成 MP3
        let mp3_bytes = self.synthesize_tts(text, tts_api_key, voice, model, speed, vol, pitch).await?;

        // 2. ffmpeg 转码为 Opus
        let opus_bytes = Self::convert_to_opus(&mp3_bytes).await?;

        // 3. 上传到飞书
        let token = self.get_tenant_access_token().await?;
        let file_key = self.upload_audio_file(&token, &opus_bytes, 3000).await?;

        // 4. 发送 audio 消息
        self.send_audio_message(&token, recipient, &file_key).await?;

        ::shadow_log::record!(
            INFO,
            ::shadow_log::Event::new(module_path!(), ::shadow_log::Action::Note)
                .with_attrs(::serde_json::json!({"text_len": text.len(), "audio_size": opus_bytes.len()})),
            "voice reply sent"
        );

        Ok(())
    }

    /// 调用 MiniMax TTS API 合成语音
    async fn synthesize_tts(
        &self,
        text: &str,
        api_key: &str,
        voice: &str,
        model: &str,
        speed: f64,
        vol: f64,
        pitch: i32,
    ) -> Result<Vec<u8>> {
        let body = serde_json::json!({
            "model": model,
            "text": text,
            "stream": false,
            "voice_setting": {
                "voice_id": voice,
                "speed": speed,
                "vol": vol,
                "pitch": pitch
            },
            "audio_setting": {
                "sample_rate": 32000,
                "bitrate": 128000,
                "format": "mp3",
                "channel": 1
            },
            "language_boost": "Chinese"
        });

        let resp = self
            .http_client()
            .post("https://api.minimaxi.com/v1/t2a_v2")
            .header("Authorization", format!("Bearer {}", api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .timeout(std::time::Duration::from_secs(60))
            .send()
            .await
            .context("Failed to send MiniMax TTS request")?;

        let result: serde_json::Value = resp
            .json()
            .await
            .context("Failed to parse TTS response")?;

        let status_code = result["base_resp"]["status_code"].as_i64().unwrap_or(-1);
        if status_code != 0 {
            let msg = result["base_resp"]["status_msg"]
                .as_str()
                .unwrap_or("unknown");
            bail!("MiniMax TTS error: {}", msg);
        }

        let audio_hex = result["data"]["audio"]
            .as_str()
            .context("TTS response missing audio field")?;

        // hex 解码
        let bytes = (0..audio_hex.len())
            .step_by(2)
            .map(|i| {
                u8::from_str_radix(&audio_hex[i..i + 2], 16)
                    .map_err(|e| anyhow::anyhow!("hex decode error at {}: {}", i, e))
            })
            .collect::<Result<Vec<_>>>()?;

        Ok(bytes)
    }

    /// MP3 转 Opus（飞书语音消息要求）
    async fn convert_to_opus(mp3_data: &[u8]) -> Result<Vec<u8>> {
        let temp_dir = std::env::temp_dir();
        let mp3_path = temp_dir.join(format!("lark_tts_{}.mp3", uuid::Uuid::new_v4()));
        let opus_path = temp_dir.join(format!("lark_tts_{}.opus", uuid::Uuid::new_v4()));

        // 写入 MP3
        tokio::fs::write(&mp3_path, mp3_data)
            .await
            .context("Failed to write temp MP3")?;

        // ffmpeg 转码
        let output = tokio::process::Command::new("ffmpeg")
            .args([
                "-y",
                "-i", mp3_path.to_str().unwrap(),
                "-c:a", "libopus",
                "-b:a", "32k",
                "-ar", "16000",
                "-ac", "1",
                opus_path.to_str().unwrap(),
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output()
            .await
            .context("Failed to run ffmpeg")?;

        // 清理 MP3
        let _ = tokio::fs::remove_file(&mp3_path).await;

        if !output.status.success() {
            bail!("ffmpeg opus conversion failed");
        }

        let opus_bytes = tokio::fs::read(&opus_path)
            .await
            .context("Failed to read opus output")?;

        // 清理 opus 临时文件
        let _ = tokio::fs::remove_file(&opus_path).await;

        Ok(opus_bytes)
    }

    /// 上传音频文件到飞书
    async fn upload_audio_file(
        &self,
        token: &str,
        opus_data: &[u8],
        duration_ms: u32,
    ) -> Result<String> {
        let url = format!("{}/im/v1/files", self.api_base());

        // 写入临时文件用于 multipart 上传
        let temp_path = std::env::temp_dir().join(format!("lark_upload_{}.opus", uuid::Uuid::new_v4()));
        tokio::fs::write(&temp_path, opus_data).await?;

        let file_part = reqwest::multipart::Part::bytes(opus_data.to_vec())
            .file_name("voice.opus")
            .mime_str("audio/ogg")?;

        let form = reqwest::multipart::Form::new()
            .text("file_type".to_string(), "opus".to_string())
            .text("file_name".to_string(), "voice.opus".to_string())
            .text("duration".to_string(), duration_ms.to_string())
            .part("file", file_part);

        let resp = self
            .http_client()
            .post(&url)
            .header("Authorization", format!("Bearer {}", token))
            .multipart(form)
            .send()
            .await
            .context("Failed to upload audio file")?;

        // 清理临时文件
        let _ = tokio::fs::remove_file(&temp_path).await;

        let result: serde_json::Value = resp.json().await?;
        let code = result["code"].as_i64().unwrap_or(-1);
        if code != 0 {
            let msg = result["msg"].as_str().unwrap_or("unknown");
            bail!("Lark file upload failed: {}", msg);
        }

        let file_key = result["data"]["file_key"]
            .as_str()
            .context("Upload response missing file_key")?
            .to_string();

        Ok(file_key)
    }

    /// 发送 audio 类型消息
    async fn send_audio_message(
        &self,
        token: &str,
        recipient: &str,
        file_key: &str,
    ) -> Result<()> {
        let url = format!(
            "{}/im/v1/messages?receive_id_type=chat_id",
            self.api_base()
        );

        let content = serde_json::json!({
            "file_key": file_key
        });

        let body = serde_json::json!({
            "receive_id": recipient,
            "msg_type": "audio",
            "content": content.to_string()
        });

        let resp = self
            .http_client()
            .post(&url)
            .header("Authorization", format!("Bearer {}", token))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .context("Failed to send audio message")?;

        let result: serde_json::Value = resp.json().await?;
        let code = result["code"].as_i64().unwrap_or(-1);
        if code != 0 {
            let msg = result["msg"].as_str().unwrap_or("unknown");
            bail!("Lark audio message send failed: {}", msg);
        }

        Ok(())
    }
}