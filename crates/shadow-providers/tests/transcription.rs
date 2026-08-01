//! TranscriptionProvider 集成测试

use shadow_providers::{TranscriptionProvider, LocalWhisperProvider};
use std::path::PathBuf;

#[test]
fn test_audio_format_detection() {
    use shadow_providers::audio_format;

    // 测试格式检测
    assert_eq!(audio_format::mime_for_extension("wav"), Some("audio/wav"));
    assert_eq!(audio_format::mime_for_extension("mp3"), Some("audio/mpeg"));
    assert_eq!(audio_format::mime_for_extension("flac"), Some("audio/flac"));
    assert_eq!(audio_format::mime_for_extension("ogg"), Some("audio/ogg"));
    assert_eq!(audio_format::mime_for_extension("oga"), Some("audio/ogg"));
    assert_eq!(audio_format::mime_for_extension("xyz"), None);

    // 测试文件名解析
    assert_eq!(
        audio_format::mime_from_filename("test.wav"),
        Some("audio/wav")
    );
    assert_eq!(
        audio_format::mime_from_filename("test.WAV"),
        Some("audio/wav")
    );
    assert_eq!(
        audio_format::mime_from_filename("test.mp3"),
        Some("audio/mpeg")
    );

    // 测试文件名标准化
    assert_eq!(
        audio_format::normalize_filename("audio.oga"),
        "audio.ogg"
    );
    assert_eq!(
        audio_format::normalize_filename("audio.wav"),
        "audio.wav"
    );
}

#[test]
fn test_local_whisper_provider_creation() {
    // 测试默认创建（会失败如果没有 whisper.cpp）
    match LocalWhisperProvider::with_defaults() {
        Ok(provider) => {
            assert_eq!(provider.name(), "local_whisper");
            let formats = provider.supported_formats();
            assert!(formats.contains(&"wav".to_string()));
            assert!(formats.contains(&"mp3".to_string()));
            assert!(formats.contains(&"flac".to_string()));
        }
        Err(e) => {
            // 预期行为：如果没有安装 whisper.cpp
            println!("whisper.cpp not installed: {}", e);
        }
    }
}

#[test]
fn test_local_whisper_provider_validation() {
    // 测试路径验证
    let result = LocalWhisperProvider::new(
        "/nonexistent/whisper",
        "/nonexistent/model.bin",
        "/tmp",
        false,
    );

    assert!(result.is_err(), "应该失败：二进制不存在");

    if let Err(e) = result {
        let error_msg = format!("{}", e);
        assert!(error_msg.contains("not found") || error_msg.contains("whisper.cpp"));
    }
}

#[test]
fn test_supported_formats() {
    use shadow_providers::audio_format;

    let formats = shadow_providers::default_supported_formats();
    
    // 验证所有常见格式都支持
    assert!(formats.contains(&"wav".to_string()));
    assert!(formats.contains(&"mp3".to_string()));
    assert!(formats.contains(&"flac".to_string()));
    assert!(formats.contains(&"ogg".to_string()));
    assert!(formats.contains(&"opus".to_string()));
    assert!(formats.contains(&"m4a".to_string()));
    assert!(formats.contains(&"webm".to_string()));
}

// 注意：实际的转录测试需要 whisper.cpp 和模型文件
// 这些测试需要在有依赖的 CI/CD 环境中运行

/// 创建一个简单的 WAV 文件（PCM 16-bit, 16kHz, mono）
fn create_silent_wav(sample_rate: u32, seconds: u32) -> Vec<u8> {
    use std::io::Write;
    let mut data = Vec::new();
    
    // RIFF header
    write!(data, "RIFF").unwrap();
    let file_size = 36 + (sample_rate * seconds * 2) as u32;
    data.extend_from_slice(&file_size.to_le_bytes());
    
    // WAVE format
    write!(data, "WAVE").unwrap();
    write!(data, "fmt ").unwrap();
    let fmt_size: u32 = 16;
    data.extend_from_slice(&fmt_size.to_le_bytes());
    let audio_format: u16 = 1; // PCM
    data.extend_from_slice(&audio_format.to_le_bytes());
    let channels: u16 = 1; // Mono
    data.extend_from_slice(&channels.to_le_bytes());
    data.extend_from_slice(&sample_rate.to_le_bytes());
    let byte_rate = sample_rate * 2;
    data.extend_from_slice(&byte_rate.to_le_bytes());
    let block_align: u16 = 2;
    data.extend_from_slice(&block_align.to_le_bytes());
    let bits_per_sample: u16 = 16;
    data.extend_from_slice(&bits_per_sample.to_le_bytes());
    
    // data chunk
    write!(data, "data").unwrap();
    let data_size = (sample_rate * seconds * 2) as u32;
    data.extend_from_slice(&data_size.to_le_bytes());
    
    // 静音数据（全零）
    let sample_count = (sample_rate * seconds) as usize;
    data.resize(data.len() + sample_count * 2, 0);
    
    data
}