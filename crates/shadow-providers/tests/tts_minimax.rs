//! MiniMax TTS Provider 集成测试
//!
//! 注意：synthesize 测试需要有效的 API Key
//! 运行: API_KEY=xxx cargo test --test tts_minimax -- --ignored

use shadow_providers::{TtsProvider, MiniMaxTtsProvider};

#[test]
fn test_provider_creation() {
    let provider = MiniMaxTtsProvider::new(
        "test-key",
        "speech-02-hd",
        "female-shaonv",
    );
    assert!(provider.is_ok());

    let provider = provider.unwrap();
    assert_eq!(provider.name(), "minimax");
    assert_eq!(provider.output_format(), "mp3");
}

#[test]
fn test_empty_key_rejected() {
    let result = MiniMaxTtsProvider::new("", "speech-02-hd", "female-shaonv");
    assert!(result.is_err());
}

#[test]
fn test_supported_voices() {
    let provider = MiniMaxTtsProvider::new(
        "test-key",
        "speech-02-hd",
        "female-shaonv",
    ).unwrap();

    let voices = provider.supported_voices();
    assert!(!voices.is_empty());
    assert!(voices.contains(&"female-shaonv".to_string()));
    assert!(voices.contains(&"male-qn-qingse".to_string()));
}

#[test]
fn test_builder_chain() {
    let provider = MiniMaxTtsProvider::new(
        "test-key",
        "speech-02-hd",
        "female-shaonv",
    ).unwrap()
        .with_speed(1.2)
        .with_volume(1.5)
        .with_pitch(2)
        .with_emotion("happy")
        .with_language_boost("Chinese")
        .use_beijing_endpoint();

    assert_eq!(provider.name(), "minimax");
}

#[test]
fn test_hex_decode() {
    // "Hi" in hex
    let hex = "4869";
    let bytes = minimax_hex_decode(hex);
    assert_eq!(bytes, vec![0x48, 0x69]);
}

/// 辅助函数：hex 解码（复用内部逻辑）
fn minimax_hex_decode(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect()
}

// === 需要真实 API Key 的测试 ===

#[tokio::test]
#[ignore]
async fn test_synthesize_real() {
    let api_key = std::env::var("MINIMAX_API_KEY")
        .or_else(|_| {
            // 尝试从配置文件读取
            let config = std::fs::read_to_string(
                std::env::var("HOME").map(|h| format!("{h}/.shadow/config.toml")).unwrap()
            ).unwrap_or_default();
            // 简单提取 api_key
            for line in config.lines() {
                if line.contains("api_key") && line.contains("sk-") {
                    let start = line.find("\"sk-").map(|i| i + 1);
                    let end = line.rfind("\"").filter(|&e| e > start.unwrap_or(0));
                    if let (Some(s), Some(e)) = (start, end) {
                        return Ok(line[s..e].to_string());
                    }
                }
            }
            Err(std::env::VarError::NotPresent)
        })
        .expect("MINIMAX_API_KEY not set");

    let provider = MiniMaxTtsProvider::new(
        &api_key,
        "speech-02-hd",
        "female-shaonv",
    )
    .unwrap()
    .with_language_boost("Chinese");

    let audio = provider
        .synthesize("你好，这是语音合成测试。", "")
        .await
        .expect("TTS should succeed");

    assert!(!audio.is_empty(), "Audio data should not be empty");

    // 保存到临时文件
    let out_path = "/tmp/shadow_tts_test.mp3";
    std::fs::write(out_path, &audio).unwrap();
    println!("Audio saved to {} ({} bytes)", out_path, audio.len());
}

#[tokio::test]
#[ignore]
async fn test_synthesize_empty_text() {
    let provider = MiniMaxTtsProvider::new(
        "fake-key",
        "speech-02-hd",
        "female-shaonv",
    ).unwrap();

    let result = provider.synthesize("", "").await;
    assert!(result.is_err());
}