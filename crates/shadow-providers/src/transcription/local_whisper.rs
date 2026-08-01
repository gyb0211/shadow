//! 本地 Whisper.cpp 语音转文本 Provider
//!
//! 直接调用 whisper.cpp 命令行工具，完全离线运行
//!
//! 依赖:
//! - whisper.cpp 二进制文件
//! - Whisper 模型文件（.bin 格式）
//!
//! 安装:
//! ```bash
//! git clone https://github.com/ggerganov/whisper.cpp
//! cd whisper.cpp
//! make
//! ./main -m models/ggml-base.bin -f input.wav --no-timestamps -otxt
//! ```

use super::TranscriptionProvider;
use shadow_core::kennel::attribution::Attributable;
use anyhow::{Context, Result};
use async_trait::async_trait;
use std::path::PathBuf;
use shadow_core::ToolKind;

/// 本地 Whisper.cpp Provider
pub struct LocalWhisperProvider {
    /// whisper.cpp 二进制路径
    binary_path: PathBuf,
    /// 模型文件路径（.bin）
    model_path: PathBuf,
    /// 临时音频文件目录
    temp_dir: PathBuf,
    /// 是否保留临时音频文件（调试用）
    keep_temp_files: bool,
}

impl LocalWhisperProvider {
    /// 创建新的 LocalWhisperProvider
    ///
    /// 参数:
    /// - `binary_path`: whisper.cpp 二进制路径
    /// - `model_path`: 模型文件路径
    /// - `temp_dir`: 临时文件目录
    /// - `keep_temp_files`: 是否保留临时文件
    pub fn new(
        binary_path: impl Into<PathBuf>,
        model_path: impl Into<PathBuf>,
        temp_dir: impl Into<PathBuf>,
        keep_temp_files: bool,
    ) -> Result<Self> {
        let binary_path = binary_path.into();
        let model_path = model_path.into();
        let temp_dir = temp_dir.into();

        // 验证二进制存在
        if !binary_path.exists() {
            anyhow::bail!(
                "whisper.cpp binary not found at: {}",
                binary_path.display()
            );
        }

        // 验证模型存在
        if !model_path.exists() {
            anyhow::bail!(
                "Whisper model not found at: {}",
                model_path.display()
            );
        }

        // 创建临时目录
        std::fs::create_dir_all(&temp_dir)
            .context("Failed to create temp directory")?;

        Ok(Self {
            binary_path,
            model_path,
            temp_dir,
            keep_temp_files,
        })
    }

    /// 使用默认路径创建 Provider
    ///
    /// 默认路径:
    /// - binary: ~/.local/bin/whisper 或 /usr/local/bin/whisper
    /// - model: ~/.shadow/models/ggml-base.bin
    /// - temp_dir: /tmp/shadow_stt
    pub fn with_defaults() -> Result<Self> {
        let binary_path = find_whisper_binary().unwrap_or_else(|| {
            PathBuf::from("/usr/local/bin/whisper")
        });

        let model_path = if let Some(home) = std::env::var_os("HOME") {
            PathBuf::from(home)
                .join(".shadow/models/ggml-base.bin")
        } else {
            PathBuf::from("/usr/local/share/whisper/ggml-base.bin")
        };

        Self::new(binary_path, model_path, "/tmp/shadow_stt", false)
    }
}

/// 查找 whisper.cpp 二进制
fn find_whisper_binary() -> Option<PathBuf> {
    let candidates = vec![
        "~/.local/bin/whisper",
        "~/.local/bin/whisper-cli",
        "/usr/local/bin/whisper",
        "/usr/local/bin/whisper-cli",
        "/opt/homebrew/bin/whisper-cli",
        "/usr/local/Cellar/whisper-cpp",
        "/usr/bin/whisper",
    ];

    // Homebrew whisper-cpp: 在 /usr/local/Cellar/whisper-cpp/<version>/bin/whisper-cli
    if let Ok(entries) = std::fs::read_dir("/usr/local/Cellar/whisper-cpp") {
        for entry in entries.flatten() {
            let path = entry.path().join("bin/whisper-cli");
            if path.exists() {
                return Some(path);
            }
        }
    }

    for path in candidates {
        let expanded = shellexpand::tilde(path).to_string();
        let path_buf = PathBuf::from(&expanded);
        if path_buf.exists() {
            return Some(path_buf);
        }
    }

    None
}

impl Attributable for LocalWhisperProvider {
    fn role(&self) -> shadow_core::kennel::attribution::Role {
        shadow_core::kennel::attribution::Role::Provider(
            shadow_core::kennel::attribution::ProviderKind::Transcription(
                shadow_core::kennel::attribution::TranscriptionProviderKind::Google,
            ),
        )
    }

    fn alias(&self) -> &str {
        "local_whisper"
    }
}

#[async_trait]
impl TranscriptionProvider for LocalWhisperProvider {
    fn name(&self) -> &str {
        "local_whisper"
    }

    fn supported_formats(&self) -> Vec<String> {
        // whisper.cpp 支持的格式
        vec![
            "wav", "flac", "mp3", "ogg", "opus", "m4a", "mp4", "webm",
        ]
        .into_iter()
        .map(String::from)
        .collect()
    }

    async fn transcribe(&self, audio_data: &[u8], file_name: &str) -> Result<String> {
        use super::audio_format;
        
        // 1. 验证音频格式
        let mime = audio_format::mime_from_filename(file_name)
            .ok_or_else(|| {
                anyhow::anyhow!("Unsupported audio format: {}", file_name)
            })?;

        // 2. 保存音频到临时文件
        let ext = file_name.rsplit_once('.').map(|(_, e)| e).unwrap_or("wav");
        let temp_file = self.temp_dir.join(format!(
            "stt_{}.{}",
            uuid::Uuid::new_v4(),
            ext,
        ));

        tokio::fs::write(&temp_file, audio_data)
            .await
            .context("Failed to write temp audio file")?;

        // 3. 如果不是 WAV，用 ffmpeg 转码为 WAV（16kHz, mono, S16_LE）
        let wav_file = if ext.eq_ignore_ascii_case("wav") {
            temp_file.clone()
        } else {
            let wav_path = temp_file.with_extension("wav");
            let ffmpeg_result = tokio::process::Command::new("ffmpeg")
                .args([
                    "-y",                    // 覆盖输出
                    "-i", temp_file.to_str().unwrap(),
                    "-ar", "16000",         // 16kHz
                    "-ac", "1",             // 单声道
                    "-sample_fmt", "s16",   // 16-bit
                    wav_path.to_str().unwrap(),
                ])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .output()
                .await;

            match ffmpeg_result {
                Ok(out) if out.status.success() => {
                    // 转码成功，删除原始文件
                    let _ = tokio::fs::remove_file(&temp_file).await;
                    wav_path
                }
                _ => {
                    // ffmpeg 不可用或失败，直接用原文件尝试（whisper.cpp 可能支持）
                    temp_file
                }
            }
        };

        // 4. 调用 whisper.cpp
        let output = tokio::process::Command::new(&self.binary_path)
            .arg("-m")
            .arg(&self.model_path)
            .arg("-f")
            .arg(&wav_file)
            .arg("--no-timestamps")
            .arg("-otxt")  // 输出为纯文本
            .arg("-l")     // 自动检测语言
            .arg("auto")
            .output()
            .await
            .context("Failed to execute whisper.cpp")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!(
                "whisper.cpp failed (exit {:?}): {}",
                output.status.code(),
                stderr
            );
        }

        // 5. 读取转录结果
        // whisper-cli -otxt 输出文件名是在输入文件全名后追加 .txt
        // 例如 input.wav → input.wav.txt
        let txt_file = if wav_file.extension().and_then(|e| e.to_str()) == Some("wav") {
            // 对于转码后的 .wav 文件，with_extension("txt") 会变成 xxx.txt
            // 但 whisper 输出的是 xxx.wav.txt
            let mut txt = wav_file.as_os_str().to_owned();
            txt.push(".txt");
            std::path::PathBuf::from(txt)
        } else {
            wav_file.with_extension("txt")
        };
        let transcript = tokio::fs::read_to_string(&txt_file)
            .await
            .context("Failed to read transcription output")?;

        let transcript = transcript.trim().to_string();

        // 6. 清理临时文件
        if !self.keep_temp_files {
            let _ = tokio::fs::remove_file(&wav_file).await;
            let _ = tokio::fs::remove_file(wav_file.with_extension("wav.txt")).await;
        }

        if transcript.is_empty() {
            Ok("(silence - no speech detected)".to_string())
        } else {
            // 繁体转简体（whisper 中文输出常为繁体）
            let simplified = t2s_convert(&transcript);
            Ok(simplified)
        }
    }
}

/// 繁体中文转简体中文
///
/// 使用 Python opencc 库做转换。
/// 如果 Python 或 opencc 不可用，返回原文（不做转换）。
fn t2s_convert(text: &str) -> String {
    // 快速检查：如果没有中文字符，直接返回
    let has_chinese = text.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c));
    if !has_chinese {
        return text.to_string();
    }

    let output = tokio::task::block_in_place(|| {
        std::process::Command::new("python3")
            .args([
                "-c",
                "from opencc import OpenCC; import sys; cc=OpenCC('t2s'); sys.stdout.write(cc.convert(sys.stdin.read()))",
            ])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
    });

    match output {
        Ok(mut child) => {
            use std::io::Write;
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(text.as_bytes());
            }
            match child.wait_with_output() {
                Ok(out) if out.status.success() => {
                    String::from_utf8_lossy(&out.stdout).to_string()
                }
                _ => text.to_string(),
            }
        }
        Err(_) => text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mime_for_extension() {
        use super::audio_format;

        assert_eq!(audio_format::mime_for_extension("wav"), Some("audio/wav"));
        assert_eq!(audio_format::mime_for_extension("mp3"), Some("audio/mpeg"));
        assert_eq!(audio_format::mime_for_extension("WAV"), Some("audio/wav"));
        assert_eq!(audio_format::mime_for_extension("xyz"), None);
    }

    #[test]
    fn test_normalize_filename() {
        use super::audio_format;

        assert_eq!(
            audio_format::normalize_filename("audio.oga"),
            "audio.ogg"
        );
        assert_eq!(
            audio_format::normalize_filename("audio.wav"),
            "audio.wav"
        );
    }
}