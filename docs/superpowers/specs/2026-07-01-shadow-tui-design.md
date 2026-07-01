# Shadow TUI 设计文档

- **日期**: 2026-07-01
- **作者**: brainstorm session(wan + claude)
- **状态**: 设计已对齐,待写实施计划
- **目标分支**: `feature/shadow-tui`(实施时新建)

## 1. 目标与范围

把 `shadow chat` 命令从纯行式 `println!` 升级成一个**全功能 dashboard TUI**:
- chat 主视图(消息流 + 工具调用可视化)
- config 编辑视图
- memory 浏览/搜索视图
- 三个视图通过 ⌘K 命令面板切换

参考 ZeroClaw 的 `zerocode` 子模块(37,223 行 ratatui 代码),但本项目只做精简版,目标单 crate ~1500 行。

### 明确不在范围内(YAGNI)
- 真实 LLM 流式 token 接口(Provider trait 仍是 request/response)
- 主题切换器(先固定 GitHub Dark)
- Windows 平台专门适配(后期再说)
- 多 agent 并行会话

## 2. 已确认决策(brainstorm 阶段)

| 维度 | 选择 |
|---|---|
| 范围 | 全功能 dashboard (chat + config + memory) |
| 导航 | 全屏 + ⌘K 命令面板(类 VSCode/Zellij) |
| 消息样式 | 终端式:角色前缀 + 颜色,工具调用框线包内联 |
| 整体布局 | 单栏:顶 status / 中消息流 / 底输入 / 底 hint |
| 主题 | GitHub Dark:user=#58a6ff · assistant=#7ee787 · tool=#8b949e |
| 集成 | 替换 `shadow chat` 默认;`--plain` / `-m <msg>` / 非 TTY 走原行式 |
| 技术栈 | ratatui + crossterm + tokio |
| 位置 | 新 crate `crates/shadow-tui` |
| 工具输出传递 | 扩展 `ObserverEvent::ToolCall` 加 `output_preview` 字段(路 2) |

## 3. 架构

### 3.1 Crate 位置与依赖
```
crates/shadow-tui/
├── Cargo.toml
└── src/
    ├── lib.rs              # pub fn run_tui(config) -> Result<()>
    ├── runner.rs           # 主循环 + 终端初始化/还原
    ├── app.rs              # AppState 状态机
    ├── event.rs            # AppEvent enum
    ├── theme.rs            # 配色常量
    ├── observer.rs         # UiObserver impl
    ├── terminal.rs         # Terminal trait (测试可注入)
    ├── views/
    │   ├── mod.rs
    │   ├── chat.rs
    │   ├── config.rs
    │   └── memory.rs
    └── widgets/
        ├── mod.rs
        ├── message_list.rs
        ├── input_box.rs
        ├── status_bar.rs
        └── command_palette.rs
```

**依赖**:
```toml
[dependencies]
shadow-core.workspace    = true   # Provider/Memory/Tool/Observer/Attributable
shadow-runtime.workspace = true   # Agent + AgentBuilder
shadow-config.workspace  = true   # Config load/save
shadow-log.workspace     = true   # record! 宏
ratatui                  = "0.28"
crossterm                = "0.28"
tokio.workspace          = true   # mpsc + task
anyhow.workspace         = true
async-trait.workspace    = true
parking_lot.workspace    = true
atty                     = "2"    # isatty 检测
```

### 3.2 线程模型(事件驱动)
```
┌──────────────────────┐         mpsc          ┌─────────────────────┐
│  Tokio task          │  AppEvent.tx → rx      │  Main thread (TUI)  │
│  ─────────────────── │ ─────────────────────> │  ────────────────── │
│  Agent::chat(...)    │   AgentMessage         │  crossterm input    │
│  Provider::chat      │   AgentToolCall        │  ratatui draw       │
│  Tool::execute       │   AgentDone/Error      │  AppState 更新       │
│  observer.record     │                        │                     │
└──────────────────────┘                        └─────────────────────┘
```

- 主线程独占终端 raw mode 与 ratatui 绘制,**绝不阻塞**
- 后台 task 跑 `Agent::chat`(含 provider HTTP + 工具执行),通过 mpsc 推回事件
- 用户提交输入 → 主线程 spawn 一个新 task 去跑 Agent

### 3.3 main.rs 集成
```rust
Commands::Chat { message, plain } => {
    if message.is_none() && !plain && atty::is(atty::Stream::Stdin) {
        shadow_tui::run_tui(config).await?;
    } else {
        chat_plain(config, message).await?;  // 原 chat_direct / chat_via_agent
    }
}
```
- 新增 `--plain/-p` flag 强制走行式
- `-m <msg>` 单次模式保留
- 非 TTY(管道/CI)自动走行式

## 4. 核心组件

### 4.1 AppState(中心状态机)
```rust
pub struct AppState {
    pub view: View,                    // Chat / Config / Memory
    pub palette: Option<PaletteState>, // None=关闭
    pub chat: ChatState,
    pub config_view: ConfigViewState,
    pub memory_view: MemoryViewState,
    pub status_top: StatusLine,
    pub status_bottom: StatusLine,
    pub running: bool,
    pub last_error: Option<String>,
}

pub enum View { Chat, Config, Memory }

pub struct ChatState {
    pub messages: Vec<ChatMessage>,    // 含 user/assistant/tool 全部
    pub input: InputBuffer,            // 多行编辑
    pub input_history: Vec<String>,    // 历史提交(↑↓ 切换)
    pub scroll_offset: usize,          // 0 = 滚到底
    pub agent_busy: bool,              // 后台正在跑
}
```

### 4.2 AppEvent
```rust
pub enum AppEvent {
    Key(crossterm::event::KeyEvent),
    AgentMessage(String),              // 完整 assistant 回复
    AgentToolCall {
        name: String,
        success: bool,
        output_preview: String,        // 截断 200 字符
        duration_ms: u64,
    },
    AgentDone,
    AgentError(String),
    Status(String),                    // 顶部状态栏文字
    MemoryLoaded(Vec<MemoryEntry>),
}
```

### 4.3 Views

**ChatView** — 消息流 + 输入框(默认视图)
- 上 70%:`MessageList`(只读,自动滚到底,↑↓ 翻页)
- 下 30%:`InputBox`(多行编辑,Enter 提交,⌥↵ 换行)

**ConfigView** — 表格编辑
- 列出 `[agent]`、`[providers.openai.minimax]`、`[memory]` 全部键值
- 选中行 + Enter 弹 `InputBox` 编辑
- 写回通过 `shadow_config::save()` 立即持久化

**MemoryView** — 列表 + 搜索
- 顶:搜索框(`/` 触发,按 key/content 模糊匹配)
- 列表:key + 时间戳 + content 预览
- `Enter` 展开 · `d` 删除 · `c` 清空(二次确认)

### 4.4 Widgets

| Widget | 职责 | 关键行为 |
|---|---|---|
| `MessageList` | 渲染消息流 | user=蓝前缀 · assistant=绿前缀 · tool=灰框线 |
| `InputBox` | 多行输入 | Enter 提交 · ⌥↵ 换行 · ↑↓ 历史导航 · slash 命令 |
| `StatusBar` | 顶/底状态条 | 顶:model/alias · 底:hint + hist N/M |
| `CommandPalette` | ⌘K 弹层 | 模糊匹配 · Enter 执行 · Esc 关闭 |

### 4.5 Theme
```rust
// crates/shadow-tui/src/theme.rs
pub const USER:       Color = Color::Rgb(0x58, 0xa6, 0xff); // 蓝
pub const ASSISTANT:  Color = Color::Rgb(0x7e, 0xe7, 0x87); // 绿
pub const TOOL_DIM:   Color = Color::Rgb(0x6e, 0x76, 0x81); // 工具框线
pub const TOOL_TEXT:  Color = Color::Rgb(0x8b, 0x94, 0x9e); // 工具内容
pub const ERROR:      Color = Color::Rgb(0xf8, 0x53, 0x73); // 错误
pub const TEXT:       Color = Color::Rgb(0xe6, 0xed, 0xf3); // 主文字
pub const DIM:        Color = Color::Rgb(0x6e, 0x76, 0x81); // 次要文字
pub const BG:         Color = Color::Rgb(0x0d, 0x11, 0x17); // 背景
```

## 5. 数据流

### 5.1 关键设计:复用 `Observer` 抽象
`Agent` 已有 `observer: Arc<dyn Observer>` 字段,在 chat 循环中调用 `record_event(&ObserverEvent)`,事件类型完备(`LlmRequest` / `LlmResponse` / `ToolCall` / `Error`)。

TUI 不改 `Provider` trait,只需写 `UiObserver` 把 Observer 事件转发到 mpsc。

### 5.2 shadow-core 改动(必须)
扩展 `ObserverEvent::ToolCall` 加 `output_preview` 字段:

**Before** (`crates/shadow-core/src/observer.rs`):
```rust
pub enum ObserverEvent {
    ToolCall { tool: String, success: bool, duration_ms: u64 },
    // ...
}
```

**After**:
```rust
pub enum ObserverEvent {
    ToolCall {
        tool: String,
        success: bool,
        duration_ms: u64,
        output_preview: String,   // 新增, 截断到 200 字符
    },
    // ...
}
```

`#[non_exhaustive]` 已有,加字段对外部 match 不破坏(只对构造点破坏)。需同步更新:
- `crates/shadow-runtime/src/agent.rs` 在 `execute_tool_call` 后 record 时填入
- 现有测试中构造 `ObserverEvent::ToolCall` 的地方

### 5.3 事件流(用户提交一次输入)
```
[主线程] InputBox Enter
    │
    ├─ push user 消息到 ChatState (立即渲染)
    ├─ chat.agent_busy = true
    ├─ spawn task: agent.chat(input).await
    │       │
    │       ├─ [Agent loop iteration N]
    │       │   ├─ observer.record_event(LlmRequest)       ─┐
    │       │   ├─ provider.chat(request)                    │
    │       │   ├─ observer.record_event(LlmResponse)        │ mpsc.tx
    │       │   ├─ 若有 tool_calls:                          │ ───> [主线程]
    │       │   │   └─ for each tool:                        │           │
    │       │   │       ├─ tool.execute()                    │           ▼
    │       │   │       └─ observer.record_event(ToolCall {  │   AppState 更新
    │       │   │           output_preview, ..               │   (追加消息/工具框)
    │       │   │       })                                   │
    │       │   └─ (循环直到无 tool_calls)                    │
    │       └─ return final_content                          │
    │                                                        │
    └─ < return value 发 AppEvent::AgentDone > ──────────────┘

[主线程] AgentDone
    └─ chat.agent_busy = false
       最终 assistant 消息已通过 AgentMessage 显示
```

### 5.4 mpsc 配置
```rust
let (tx, rx) = tokio::sync::mpsc::channel::<AppEvent>(256);
```
- buffer 256(单次 chat 最多几十条事件)
- 满了 `try_send` 返回 err,tracing::warn! 静默 drop

### 5.5 UiObserver
`Observer` trait 以 `Attributable` 为 super-trait,所以 `UiObserver` 必须同时实现两者:

```rust
struct UiObserver { tx: mpsc::Sender<AppEvent> }

impl Attributable for UiObserver {
    fn role(&self) -> Role { Role::System }
    fn alias(&self) -> &str { "ui-observer" }
}

#[async_trait]
impl Observer for UiObserver {
    fn record_event(&self, event: &ObserverEvent) {
        let app = match event {
            ObserverEvent::LlmRequest { model, .. } =>
                AppEvent::Status(format!("→ {model}")),
            ObserverEvent::LlmResponse { duration_ms, tokens, .. } =>
                AppEvent::Status(format!("← {duration_ms}ms · {tokens} tok")),
            ObserverEvent::ToolCall { tool, success, output_preview, duration_ms } =>
                AppEvent::AgentToolCall {
                    name: tool.clone(),
                    success: *success,
                    output_preview: output_preview.clone(),
                    duration_ms: *duration_ms,
                },
            ObserverEvent::Error { message } =>
                AppEvent::AgentError(message.clone()),
            _ => return,
        };
        let _ = self.tx.try_send(app);
    }
    fn as_any(&self) -> &dyn Any { self }
}
```

### 5.6 命令面板数据流(纯本地)
```
⌘K 按下 → AppState.palette = Some(...)
键入 → 实时过滤 COMMANDS 列表
Enter → 执行命令
   ├─ "chat" / "config" / "memory" → 切 View
   ├─ "clear history" → agent.clear_history()
   ├─ "quit" → running = false
   └─ "set model X" → 改 AgentConfig + 重建 agent
```

### 5.7 Config / Memory 异步加载
切到 Config/Memory view 时,主线程 spawn task:
```rust
task::spawn(async move {
    let entries = memory.list().await?;
    tx.send(AppEvent::MemoryLoaded(entries)).await
});
```
加载期间 view 显示 `加载中...`。

## 6. 错误处理

### 6.1 终端状态保护(最高优先级)
```rust
pub fn run_tui(config: Config) -> Result<()> {
    // panic hook 在进入前就装好
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        prev_hook(info);
    }));

    let result = run_tui_inner(config);
    let _ = disable_raw_mode();
    let _ = execute!(io::stdout(), LeaveAlternateScreen);
    result
}
```

### 6.2 错误分级

| 来源 | 处理 |
|---|---|
| Provider HTTP 错误 | 不退出,在 chat 流插红色 assistant `[错误] ...` 行 |
| 工具执行失败 | 已有 `ToolCall { success: false }`,正常渲染(灰框 + ❌) |
| 配置文件损坏 | `load_or_init` 已 `unwrap_or_default` |
| mpsc 发送失败 | `try_send` 静默 drop + tracing::warn! |
| Panic | panic hook 恢复终端后打到 stderr |

### 6.3 用户取消
- Agent 跑的时候按 `Esc` 或 `Ctrl+C` → `CancellationToken::cancel()`
- task 在下一次 await 点中止(已发出的 HTTP 请求无法取消,等返回)

## 7. 测试策略

### 7.1 单元测试 — AppState 纯逻辑
```rust
#[test]
fn palette_filter_matches_substring() { /* ... */ }

#[test]
fn slash_clear_truncates_history() { /* ... */ }
```
覆盖:状态切换、palette 过滤、slash 命令、历史截断、事件聚合。

### 7.2 Widget 快照测试
```rust
fn render_to_buffer(widget: impl Widget, w: u16, h: u16) -> Buffer {
    let mut buf = Buffer::empty(Rect::new(0, 0, w, h));
    widget.render(Rect::new(0, 0, w, h), &mut buf);
    buf
}

#[test]
fn message_renders_user_blue_assistant_green() {
    let w = MessageList::new(vec![Message::user("hi"), Message::assistant("hello")]);
    let buf = render_to_buffer(w, 60, 4);
    assert_eq!(buf.cell((0, 0)).style().fg(), Some(USER));
    assert_eq!(buf.cell((0, 1)).style().fg(), Some(ASSISTANT));
}
```
覆盖:颜色映射、工具框线、消息换行、滚动边界。

### 7.3 集成测试 — 假终端 + 假 provider
```rust
#[tokio::test]
async fn user_input_triggers_agent_and_renders_response() {
    let provider = Arc::new(StubProvider::replying("你好"));
    let (tx, rx) = mpsc::channel(64);
    let observer = Arc::new(UiObserver::new(tx));
    let agent = Agent::builder().provider(provider).observer(observer).build();
    let mut term = FakeTerminal::new();
    term.input_keys("hi\n");
    run_until_idle(agent, &mut term, rx).await;
    assert!(term.snapshot().contains("assistant ❯ 你好"));
}
```

### 7.4 Terminal trait(测试可注入)
```rust
trait Terminal {
    fn draw(&self, state: &AppState) -> Result<()>;
    fn poll_event(&self, timeout: Duration) -> Option<AppEvent>;
}
// 真终端: CrosstermTerminal
// 测试:   FakeTerminal { outputs: Vec<String>, inputs: VecDeque<KeyEvent> }
```

### 7.5 手动验收清单
- [ ] chat:输入中文/英文、多行、滚动历史
- [ ] tool:shell/file 调用,看灰框 + 输出
- [ ] ⌘K palette:fuzzy 过滤、Enter 切 view、Esc 关闭
- [ ] config:改 `temperature` 后 `~/.shadow/config.toml` 立即更新
- [ ] memory:加载、搜索、删除
- [ ] Ctrl+C:中止 agent、终端正常退出
- [ ] Panic 后 shell 状态恢复
- [ ] 非 TTY:`echo hi | shadow chat` 走行式

### 7.6 不测的(明确排除)
- 真实 LLM API 调用
- crossterm 平台特定行为
- Provider 重试逻辑

## 8. 实施顺序(粗略,实施计划里细化)

1. **shadow-core 改动** — `ObserverEvent::ToolCall` 加 `output_preview` + 改 agent.rs
2. **shadow-tui 骨架** — Cargo.toml + lib.rs + run_tui 入口 + 终端初始化/还原
3. **AppState + AppEvent + Terminal trait** — 纯逻辑可单测
4. **ChatView + MessageList + InputBox + StatusBar** — 跑通最简 chat
5. **UiObserver** — 接通 Agent → mpsc → UI
6. **CommandPalette** — ⌘K 弹层
7. **ConfigView** — 表格 + 编辑
8. **MemoryView** — 列表 + 搜索
9. **main.rs 集成** — `--plain` flag + isatty 检测
10. **测试 + 手动验收**

## 9. 风险与未决项

- **ratatui 版本**:0.28 是写文档时最新,实施时锁定具体版本
- **`atty` crate 已 unmaintained**:备选 `std::io::IsTerminal`(Rust 1.70+),实施时确认 edition 2024 可用
- **`output_preview` 长度**:200 字符是否够用,实施时实测调整
- **多次 agent 并发**:当前设计假设串行(用户提交 → 等完成 → 下一次)。若要并行需要 session 概念,YAGNI 暂不做
- **⌘K 在 Linux 终端**:Meta 键可能不传过来,Linux 用 Ctrl+K 替代,实施时按平台分发
