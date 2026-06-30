# Shadow 影子

> Trait 驱动的 AI Agent 运行时 — ZeroClaw 架构精简复刻

Shadow 是一个基于 Rust 的 AI Agent 运行时, 采用微内核 + trait 驱动设计, 支持多 LLM Provider、工具调用、记忆持久化和结构化日志。

## 特性

- 🧩 **Trait 驱动**: 所有核心能力 (Provider/Tool/Memory/Observer/Channel) 均为 trait, 可插拔替换
- 🔧 **工具调用**: 内置 Shell / FileRead / FileWrite 工具, Agent 自动循环调用直到任务完成
- 🧠 **对话记忆**: 多轮对话历史保持, 支持 Markdown 文件持久化记忆后端
- 📝 **结构化日志**: `record!` 宏 + JSONL 持久化 + 归因系统 (谁干了什么)
- 🔌 **多 Provider**: OpenAI 兼容 API (支持 OpenAI/OpenRouter/Ollama/DeepSeek/GLM 等)
- 📦 **双模式构建**: 完整版 (Agent loop + 工具) / kernel-only (最小内核, 直连 Provider)
- ⚡ **零成本归因**: 所有对象实现 `Attributable` trait, 编译期确定角色

## 架构

```
┌─────────────────────────────────────────────────────┐
│                    shadow CLI                        │
│                  (src/main.rs)                       │
├─────────────────────────────────────────────────────┤
│                  shadow-runtime                      │
│  ┌───────────┐  ┌──────────┐  ┌──────────────────┐  │
│  │  Agent     │  │  Tools   │  │  AgentBuilder    │  │
│  │  chat()    │  │ Shell    │  │  (builder 模式)  │  │
│  │  tool loop │  │ FileRead │  │                  │  │
│  │  history   │  │ FileWrite│  │                  │  │
│  └─────┬─────┘  └──────────┘  └──────────────────┘  │
├────────┼────────────────────────────────────────────┤
│         │           agent-core (微内核)              │
│  ┌──────┴──────────────────────────────────────┐    │
│  │  Tool trait  │ ModelProvider │  Memory       │    │
│  │  Attributable│ Observer      │  Channel      │    │
│  │  ChatMessage │ ChatResponse  │  ToolCall     │    │
│  └──────────────────────────────────────────────┘    │
├─────────────────────────────────────────────────────┤
│  shadow-providers  │ shadow-memory │ shadow-log     │
│  (OpenAI 兼容)     │ (Markdown)    │ (JSONL)        │
│  shadow-config     │               │                │
│  (TOML 配置)       │               │                │
└─────────────────────────────────────────────────────┘
```

### Crate 依赖关系

```
agent-core (零依赖内部 crate)
  ├── shadow-config   (配置: TOML schema + 多 provider)
  ├── shadow-log      (日志: record! 宏 + JSONL + 广播)
  ├── shadow-providers (Provider: OpenAI 兼容)
  ├── shadow-memory   (记忆: Markdown / None)
  └── shadow-runtime  (运行时: Agent + Tools, 依赖以上全部)
      └── shadow (二进制, CLI 入口)
```

## 快速开始

### 安装

```bash
git clone https://github.com/gyb0211/shadow.git
cd shadow
cargo build --release
# 二进制: target/release/shadow
```

### 配置

配置文件位于 `~/.shadow/config.toml`, 首次运行自动创建。

```toml
[agent]
alias = "default"
model_provider = "openai.default"    # 引用 providers 表
model = "gpt-4o-mini"
temperature = 0.7
autonomy = "supervised"               # full / supervised / read_only

[providers.openai.default]
api_key = "sk-xxx"
model = "gpt-4o-mini"
base_url = "https://api.openai.com/v1"

# 多 provider 示例
[providers.custom.glm]
api_key = "xxx"
model = "glm-4-flash"
base_url = "https://open.bigmodel.cn/api/paas/v4"

[providers.custom.deepseek]
api_key = "xxx"
model = "deepseek-chat"
base_url = "https://api.deepseek.com"

[memory]
backend = "markdown"                  # none / markdown
```

### 使用

```bash
# 交互式多轮对话 (完整版, 带工具和历史记忆)
shadow chat

# 单次对话
shadow chat -m "你好"

# 配置管理
shadow config list
shadow config path

# 记忆管理
shadow memory list
shadow memory get <key>
shadow memory forget <key>
shadow memory clear
```

### 交互式对话命令

| 命令 | 说明 |
|------|------|
| `/quit` `/exit` | 退出对话 |
| `/clear` | 清空对话历史 |

## 双模式构建

### 完整版 (默认)

```bash
cargo build                          # 默认开启 runtime feature
./target/debug/shadow chat           # Agent loop + 工具 + 历史 + 日志
```

### Kernel-Only 模式

```bash
cargo build --no-default-features    # 关闭 runtime, 最小内核
./target/debug/shadow chat           # 直连 Provider, 无 Agent loop
```

## 内置工具

| 工具 | 名称 | 说明 |
|------|------|------|
| Shell | `shell` | 执行 shell 命令, 返回 stdout + stderr |
| FileRead | `file_read` | 读取文件内容, 自动截断超大文件 (前 100KB) |
| FileWrite | `file_write` | 写入文件, 自动创建父目录 |

### 工具调用流程

```
用户消息 → LLM → 有 tool_calls?
                      ├── 是 → 执行工具 → 结果放回 history → 回到 LLM
                      └── 否 → 返回最终回复
```

Agent 内置最大 10 次工具调用循环, 防止无限执行。

## 核心设计

### Trait 驱动

所有核心能力定义为 trait, 位于 `agent-core`:

- `Tool` — 工具能力 (name + description + parameters + execute)
- `ModelProvider` — LLM 后端 (chat + list_models)
- `Memory` — 记忆后端 (store + recall + get + list + forget)
- `Observer` — 事件观察 (record_event)
- `Channel` — 消息渠道 (send)

### 归因系统

每个参与事件的对象实现 `Attributable` trait:

```rust
pub trait Attributable: Send + Sync {
    fn role(&self) -> Role;       // Agent / Tool / Provider / Memory / Channel / System
    fn alias(&self) -> &str;      // 具体名称
}
```

`Arc<T>`, `Box<T>`, `&T` 自动实现 `Attributable` (blanket impl)。

### 日志系统

```rust
// 唯一日志发射点
shadow_log::record!(INFO, Action::Start, "agent 启动");

// 自动写入 ~/.shadow/state/runtime-trace.jsonl (JSONL 格式)
// 支持进程内广播 (SSE / 实时订阅)
```

## 支持的 Provider

| Family | 默认 base_url | 说明 |
|--------|---------------|------|
| openai | `https://api.openai.com/v1` | OpenAI 官方 |
| openrouter | `https://openrouter.ai/api/v1` | OpenRouter |
| ollama | `http://localhost:11434/v1` | 本地 Ollama |
| deepseek | `https://api.deepseek.com` | DeepSeek |
| glm / zhipu | `https://open.bigmodel.cn/api/paas/v4` | 智谱 GLM |
| moonshot | `https://api.moonshot.cn/v1` | Moonshot (Kimi) |
| qwen | `https://dashscope.aliyuncs.com/compatible-mode/v1` | 通义千问 |
| minimax | `https://api.minimax.chat/v1` | MiniMax |
| doubao | `https://ark.cn-beijing.volces.com/api/v3` | 豆包 |
| custom | (需配置 base_url) | 任意 OpenAI 兼容 API |

## 项目结构

```
shadow/
├── agent-core/           # 核心层: trait 定义 (零内部依赖)
│   └── src/
│       ├── attribution.rs  # Attributable + Role
│       ├── provider.rs      # ModelProvider + ChatMessage/ChatResponse
│       ├── tool.rs          # Tool trait + ToolResult + ToolSpec
│       ├── memory.rs        # Memory trait
│       ├── observer.rs      # Observer trait + ObserverEvent
│       └── channel.rs       # Channel trait + CliChannel
├── crates/
│   ├── shadow-config/     # 配置: TOML schema + 多 provider 解析
│   ├── shadow-log/        # 日志: record! 宏 + JSONL + 广播
│   ├── shadow-providers/  # Provider: OpenAI 兼容实现
│   ├── shadow-memory/     # 记忆: Markdown + None 后端
│   └── shadow-runtime/    # 运行时: Agent + Tools
│       └── src/
│           ├── agent.rs      # Agent (chat + tool loop + history)
│           └── tools/        # Shell / FileRead / FileWrite
├── src/main.rs           # CLI 入口 (clap)
├── Cargo.toml            # workspace 根
└── docs/                 # 文档
```

## 开发

```bash
# 构建
cargo build

# 测试
cargo test --workspace

# 构建 kernel-only 版
cargo build --no-default-features

# 构建 release 版 (优化体积)
cargo build --release
```

## License

MIT
