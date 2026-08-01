# Shadow 本地语音转文本（STT）

## 架构

```
Channel → TranscriptionProvider → whisper.cpp → 转录文本
```

## 功能特性

### 1. TranscriptionProvider Trait

定义统一的语音转文本接口：

```rust
#[async_trait]
pub trait TranscriptionProvider: Send + Sync + Attributable {
    fn name(&self) -> &str;
    async fn transcribe(&self, audio_data: &[u8], file_name: &str) -> Result<String>;
    fn supported_formats(&self) -> Vec<String>;
}
```

### 2. LocalWhisperProvider（本地 whisper.cpp）

**特点**:
- 完全离线运行
- 直接调用 whisper.cpp 命令行
- 支持多种音频格式
- 自动检测语言
- 自动清理临时文件

**支持格式**: wav, flac, mp3, ogg, opus, m4a, mp4, webm

## 安装 whisper.cpp

```bash
# 克隆并编译
git clone https://github.com/ggerganov/whisper.cpp
cd whisper.cpp
make

# 下载模型（可选，base 适合测试）
wget https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin
mv ggml-base.bin ~/.shadow/models/

# 安装到系统路径（可选）
sudo cp ./main /usr/local/bin/whisper
```

## 使用示例

### Rust 代码

```rust
use shadow_providers::{LocalWhisperProvider, TranscriptionProvider};

// 使用默认路径
let provider = LocalWhisperProvider::with_defaults()?;

// 或者自定义路径
let provider = LocalWhisperProvider::new(
    "/usr/local/bin/whisper",
    "/home/user/.shadow/models/ggml-base.bin",
    "/tmp/shadow_stt",
    false,  // 不保留临时文件
)?;

// 转录音频
let audio_data = std::fs::read("voice.wav")?;
let transcript = provider.transcribe(&audio_data, "voice.wav").await?;

println!("识别结果: {}", transcript);
```

### 配置示例（TOML）

```toml
[transcription]
provider = "local_whisper"

[transcription.local_whisper]
binary_path = "/usr/local/bin/whisper"
model_path = "/home/user/.shadow/models/ggml-base.bin"
temp_dir = "/tmp/shadow_stt"
keep_temp_files = false
```

## 测试

```bash
# 运行所有测试
cargo test -p shadow-providers

# 只运行转录相关测试
cargo test -p shadow-providers --test transcription

# 测试音频格式检测
cargo test -p shadow-providers test_audio_format_detection
```

## 模型选择

| 模型 | 大小 | 速度 | 准确度 | 推荐场景 |
|------|------|------|--------|----------|
| tiny | 39 MB | 最快 | 较低 | 实时转录 |
| base | 74 MB | 快 | 中等 | 通用 |
| small | 246 MB | 中等 | 较高 | 平衡 |
| medium | 769 MB | 慢 | 高 | 高质量 |
| large | 1.5 GB | 最慢 | 最高 | 最佳准确度 |

下载模型:
```bash
wget https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin
wget https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin
```

## 与 ZeroClaw 对比

| 功能 | ZeroClaw | Shadow |
|------|----------|-------|
| LocalWhisper | ✅ HTTP API wrapper | ✅ 直接调用 whisper.cpp |
| 通道集成 | ✅ TranscriptionManager | 待实现 |
| 支持格式 | 11 种 | 8 种 |
| 测试覆盖 | 单元测试 | 集成测试 |
| 配置方式 | 复杂（provider hierarchy） | 简洁（直接配置） |

## 后续计划

1. **Channel 集成**: 在 Discord/Telegram channel 中添加音频附件转录
2. **在线 Provider**: 添加 OpenAI Whisper、Groq 等云端 API 支持
3. **配置系统**: 集成到 shadow-config
4. **TTS 补全**: 实现文本转语音（Piper/EdgeTTS）

## 故障排查

### whisper.cpp 找不到

```bash
# 检查二进制是否存在
which whisper

# 或手动检查
ls -la ~/.local/bin/whisper
ls -la /usr/local/bin/whisper
```

### 模型文件找不到

```bash
# 检查模型路径
ls -la ~/.shadow/models/ggml-*.bin

# 下载模型
mkdir -p ~/.shadow/models
wget https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin \
  -O ~/.shadow/models/ggml-base.bin
```

### 权限问题

```bash
# 确保 whisper.cpp 可执行
chmod +x ~/.local/bin/whisper
chmod +x /usr/local/bin/whisper
```

## 参考

- whisper.cpp: https://github.com/ggerganov/whisper.cpp
- ZeroClaw STT 实现: `crates/zeroclaw-channels/src/transcription.rs`
- Whisper 文档: https://github.com/openai/whisper