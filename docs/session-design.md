# SessionStore 扩展设计 -- 元信息 sidecar + append_message API

> 分支: `feature/kernel-workspace`
> 提交: `0bf8c2c feat(kernel): SessionStore 扩展 -- 元信息 sidecar + append_message API`
> 关联: 微内核基础设施补齐 (session / memory / workspace / tool calls)

---

## 1. 背景

旧 `SessionStore` trait 只有 4 个方法 (`load` / `save` / `delete` / `list`), 存在三个问题:

### 1.1 `save` 语义模糊

```rust
// 旧 API
async fn save(&self, session: &Session) -> Result<()>;
```

`save` 既可以表示 "新建", 也可以表示 "追加", 还可以表示 "全量覆盖"。
实现方只能选一种, 调用方却想要不同行为:

| 调用方       | 期望行为           | 旧 save 实际行为   |
| ------------ | ------------------ | ------------------- |
| 运行时对话流 | **追加** 新消息    | 取决于实现 (歧义) |
| 初始化/修复  | **全量覆盖**       | 取决于实现 (歧义) |
| 恢复历史    | **全量覆盖**       | 取决于实现 (歧义) |

实际影响: `shadow-runtime/src/agent.rs` 每轮对话构造 `Session { id, messages: [user, assistant] }` 调用 `save`, **依赖 JsonlSessionStore 的实现恰好是 append-only**。一旦实现改成 truncate+write, 多轮对话会只剩最后一轮 -- 隐式契约脆弱。

### 1.2 元信息缺失

旧 `Session` 只有 `id` + `messages`:

```rust
pub struct Session {
    pub id: String,
    pub messages: Vec<ChatMessage>,
}
```

无法支持:
- TUI 列出会话时显示 **标题 / 最后更新时间 / 消息数** (必须 load 整个 jsonl 才能算)
- 多 agent / 多 profile 时区分 **会话归属**
- UI 按 **创建时间排序** (现在只能按文件 mtime, 粒度粗且不可靠)

### 1.3 `list` 不返回元信息

```rust
async fn list(&self) -> Result<Vec<String>>;  // 只返回 ID
```

UI 想展示 "最近 10 个会话 + 每个的消息数 + 标题" 必须对每个 ID 再 `load` 一遍 -- O(n) 全量 JSON 解析, 会话多了就卡。

---

## 2. 设计目标

1. **语义清晰**: 追加 vs 全量覆盖 用不同 API, 不再混用 `save`
2. **元信息可用**: 标题 / 时间戳 / agent 归属 / 消息数 都能存能查
3. **UI 友好**: 不用 load 整个消息历史就能列出会话概览
4. **向后兼容**: 旧 session 文件 (无元信息) 不破坏, 自动降级
5. **多用户预留**: 元信息含 `agent_alias`, 为未来多 agent / 多 profile 留接口
6. **微内核零依赖**: `shadow-core` 不依赖其他内部 crate, trait 层稳定

---

## 3. API 变更

### 3.1 `Session` 结构 -- 增加 4 个元信息字段

```rust
// 旧
pub struct Session {
    pub id: String,
    pub messages: Vec<ChatMessage>,
}

// 新
pub struct Session {
    pub id: String,
    pub messages: Vec<ChatMessage>,
    /// 人类可读的会话标题 (UI 展示用)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// 创建时间 (RFC 3339)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    /// 最后更新时间 (RFC 3339)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    /// 关联的 agent 别名 (多 agent / 多 profile 时区分)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_alias: Option<String>,
}
```

**设计要点**:
- 元信息字段全 `Option`, **旧代码 `Session { id, messages }` + `..Default::default()` 仍可工作** (但显式构造会编译失败, 强制迁移)
- `skip_serializing_if = "Option::is_none"` 保证序列化产物干净 -- 没填的字段不出现在 JSON 里
- 时间用 RFC 3339 字符串而非 `chrono::DateTime<Utc>`, 避免把 chrono 类型泄漏到 trait API (序列化友好, 跨进程/跨语言可读)

### 3.2 新增 `SessionMetadata` -- 轻量列表用

```rust
pub struct SessionMetadata {
    pub id: String,
    pub title: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub message_count: usize,           // <-- 不含 messages, 但有数量
    pub agent_alias: Option<String>,
}
```

**为什么不直接用 `Session`?**
- `Session.messages` 是 `Vec<ChatMessage>`, 长会话可能几 MB
- UI 列表场景只需要 "标题 + 时间 + 几条消息" -- 90% 的数据没必要加载
- `SessionMetadata` 通常 < 200 字节, 一次列 100 个会话也无压力

### 3.3 `SessionStore` trait -- 6 个方法

```rust
#[async_trait]
pub trait SessionStore: Attributable {
    /// 加载完整会话 (含所有消息); 不存在返回 None
    async fn load(&self, id: &str) -> Result<Option<Session>>;

    /// 追加单条消息 -- 运行时对话流推荐用此 API
    ///
    /// 第一次调用自动创建 session (生成元信息 sidecar),
    /// 后续调用更新 `updated_at` 与 `message_count`.
    async fn append_message(&self, id: &str, message: &ChatMessage) -> Result<()>;

    /// 全量覆盖保存 -- 初始化 / 修复 / 恢复历史用
    ///
    /// 注意: 此方法 truncate 现有 messages 文件并重写元信息 sidecar,
    /// 行为与 `append_message` 不同, 不要混用.
    async fn save(&self, session: &Session) -> Result<()>;

    /// 删除会话; 不存在视为成功
    async fn delete(&self, id: &str) -> Result<()>;

    /// 列出所有会话 ID (按修改时间降序)
    async fn list(&self) -> Result<Vec<String>>;

    /// 列出所有会话含元信息 (不加载 messages, UI 友好)
    ///
    /// 旧 session (无 `.meta.json` sidecar) 用文件 mtime + jsonl 行数推导元信息.
    async fn list_with_metadata(&self) -> Result<Vec<SessionMetadata>>;
}
```

**方法语义对照表**:

| 方法                 | 读/写 | 何时用                            |
| -------------------- | ----- | --------------------------------- |
| `load`               | 读    | 用户点开某会话, 加载完整历史      |
| `append_message`     | 写    | 运行时每轮对话追加 user+assistant |
| `save`               | 写    | 初始化空会话 / 修复损坏文件       |
| `delete`             | 写    | 用户删除会话                      |
| `list`               | 读    | 只需 ID 列表的场景 (少见)         |
| `list_with_metadata` | 读    | TUI / Web UI 列出会话侧边栏       |

### 3.4 `save` vs `append_message` 语义分离 (核心改进)

**旧设计**: 只有一个 `save`, 调用方靠隐式约定判断行为 -- 脆弱。

**新设计**: 两个 API 语义明确不冲突:

```
┌─────────────────────┬─────────────────────────────────┐
│ append_message      │ save                            │
├─────────────────────┼─────────────────────────────────┤
│ 追加一行到 .jsonl   │ truncate .jsonl 然后重写        │
│ 只更新 meta 的      │ 完全重写 meta sidecar           │
│   updated_at +      │                                 │
│   message_count     │                                 │
│ 第一次自动建 meta   │ 总是覆盖 meta                   │
│ 不需要 Session 结构 │ 必须传完整 Session              │
└─────────────────────┴─────────────────────────────────┘
```

---

## 4. 文件布局 -- sidecar 元信息

### 4.1 双文件设计

```
{workspace}/sessions/
├── {id}.jsonl          # 消息历史 (一行一条 JSON, 流式追加)
├── {id}.meta.json      # 元信息 (单 JSON, 可选) <-- 新增
└── ...
```

**为什么不把元信息塞进 `.jsonl` 第一行?**
- 流式追加变复杂 (要先读第一行再决定是否覆盖)
- 损坏一行会污染整个文件
- sidecar 模式下, 元信息可以独立读写, 不碰消息文件

**为什么不用 SQLite 单库?**
- 当前规模单库没必要, 文件系统更直观
- 跨 profile 复制/备份 = 直接 `cp` 目录
- 后续上量再迁移 (Workspace 抽象已隔离路径)

### 4.2 `.meta.json` 格式示例

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "title": "调试 session_store 元信息",
  "created_at": "2026-07-03T10:15:30.123456789+00:00",
  "updated_at": "2026-07-03T14:22:11.987654321+00:00",
  "message_count": 12,
  "agent_alias": "shadow-default"
}
```

### 4.3 时间戳格式

统一用 `chrono::Utc::now().to_rfc3339()` 产出 RFC 3339 字符串:

```
2026-07-03T14:22:11.987654321+00:00
```

**理由**:
- 序列化零成本 (String 就是 String, 不需要自定义 serde)
- 跨语言可读 (Python/JS/Go 都能直接解析)
- 精度足够 (纳秒级)
- 排序 = 字典序 (`String::cmp` 直接对)

---

## 5. 向后兼容 -- 旧 session 自动降级

### 5.1 三种兼容场景

| 场景                         | 旧行为              | 新行为 (无 sidecar 时)                            |
| ---------------------------- | ------------------- | ------------------------------------------------- |
| `load` 旧 session            | 返回 Session        | 元信息字段全 `None`, messages 正常                |
| `list_with_metadata` 旧 session | (方法不存在)        | 用 **mtime 推导** `created_at`/`updated_at`, **行数** 推导 `message_count` |
| `append_message` 旧 session  | (方法不存在)        | 读出已有行数作为初始 `message_count`, 后续正常更新 |

### 5.2 推导实现

```rust
// 旧 session 的 meta 推导 (无 sidecar 时降级)
fn read_meta_or_derive(&self, id: &str) -> SessionMetadata {
    self.read_meta(id).unwrap_or_else(|| SessionMetadata {
        id: id.to_string(),
        title: None,
        created_at: self.file_mtime_rfc3339(id),  // 从文件 mtime 推
        updated_at: self.file_mtime_rfc3339(id),
        message_count: self.count_messages(id),    // 数 jsonl 行数
        agent_alias: None,
    })
}
```

### 5.3 迁移路径 (用户无感)

1. 用户从旧版本升级 → 旧 `.jsonl` 文件原封不动
2. 打开 TUI → `list_with_metadata` 推导出元信息, 显示正常
3. 用户继续对话 → `append_message` 触发首次 sidecar 写入
4. 此后该 session 有 sidecar, 元信息精度提升 (真实 created_at)

**不需要主动迁移脚本**, 旧 session 用一次就升级。

---

## 6. 调用方迁移

### 6.1 `agent.rs` -- 运行时对话流

**迁移前** (隐式依赖 append-only 实现):

```rust
let session = Session {
    id,
    messages: vec![
        ChatMessage { role: "user".into(), content: user_message.into(), .. },
        ChatMessage { role: "assistant".into(), content: final_content.clone(), .. },
    ],
};
if let Err(e) = store.save(&session).await {
    warn!("保存会话失败: {e}");
}
```

**问题**:
1. 构造整个 `Session` 结构 -- 浪费 (只需要追加 2 条)
2. 依赖 `save` 实现恰好是 append-only -- 脆弱
3. 新增 4 个 `Option` 字段后, 这个构造编译失败 -- 强制迁移

**迁移后** (语义清晰):

```rust
let user_msg = ChatMessage { role: "user".into(), content: user_message.into(), .. };
let assistant_msg = ChatMessage { role: "assistant".into(), content: final_content.clone(), .. };

if let Err(e) = store.append_message(&id, &user_msg).await {
    warn!("追加用户消息失败: {e}");
}
if let Err(e) = store.append_message(&id, &assistant_msg).await {
    warn!("追加助手消息失败: {e}");
}
```

**收益**:
- 不再构造 `Session` 结构, 直接追加
- `save` 改实现也不影响运行时 (语义隔离)
- 错误粒度更细 (用户消息失败 vs 助手消息失败分别告警)

### 6.2 其他调用方

| 调用方                  | 用法                                       |
| ----------------------- | ------------------------------------------ |
| `main.rs` 启动时        | `JsonlSessionStore::new(workspace_root)` 不变 |
| `shadow-tui/src/lib.rs` | 同上                                       |
| 未来: TUI 会话侧边栏    | 用 `list_with_metadata` 取代 `list` + 多次 `load` |

---

## 7. 实现细节

### 7.1 `append_message` 流程

```
1. 确保 sessions/ 目录存在
2. 打开 {id}.jsonl (create + append 模式)
3. serde_json 序列化消息 → writeln! 一行
4. drop(file)  (确保 flush)
5. 读已有 meta sidecar:
   ├─ 有: updated_at = now, message_count = count_messages(id)
   └─ 无: 新建, created_at = updated_at = now, message_count = 1
6. 写回 meta sidecar
```

**为什么 `count_messages` 重新数而不是 +1?**
- 防止 sidecar 与 jsonl 失同步 (手动编辑 / 文件损坏 / sidecar 丢失)
- jsonl 通常 < 1000 行, 数一遍很快
- 一致性 > 微小性能差异

### 7.2 `save` 流程

```
1. 确保 sessions/ 目录存在
2. 打开 {id}.jsonl (create + write + truncate 模式)
3. 遍历 session.messages, 每条序列化为一行
4. drop(file)
5. 构造 meta:
   - title / agent_alias 从 session 取
   - created_at 缺失则用 now 兜底
   - updated_at = now
   - message_count = session.messages.len()
6. 写回 meta sidecar (覆盖)
```

### 7.3 `current_session_id` (非 trait 方法)

按文件 mtime 找最近修改的 `.jsonl`, **跳过 `.meta.json` sidecar**:

```rust
if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
    continue;  // 跳过 .meta.json
}
```

**理由**: sidecar 与 jsonl 同步更新, mtime 接近; 不跳过的话可能错误返回 `xxx.meta` 作为 ID。

### 7.4 `Attributable` 实现

```rust
impl Attributable for JsonlSessionStore {
    fn role(&self) -> Role { Role::Session }
    fn alias(&self) -> &str { "jsonl" }
}
```

参与归因系统 (与 Provider / Memory / Channel / Tool 同层级), 用于日志追踪 "哪条 session 写了什么"。

---

## 8. 测试覆盖

`crates/shadow-core/src/session_store.rs::tests` 共 15 个测试, 覆盖:

### 8.1 基础读写

| 测试                              | 验证                                     |
| --------------------------------- | ---------------------------------------- |
| `save_load_roundtrip`             | save → load 消息一致                     |
| `load_nonexistent_returns_none`   | 文件不存在返回 None                      |
| `save_then_load_returns_metadata` | save 写 sidecar, load 回填元信息         |
| `save_truncates_and_overwrites`   | 同 id 多次 save 只保留最后一次 (核心!)   |

### 8.2 append_message

| 测试                              | 验证                                     |
| --------------------------------- | ---------------------------------------- |
| `append_message_creates_session_with_meta` | 第一次 append 自动建 sidecar     |
| `append_message_accumulates`      | 多次 append 消息累积                     |

### 8.3 list / list_with_metadata

| 测试                                  | 验证                                       |
| ------------------------------------- | ------------------------------------------ |
| `multiple_sessions_list_and_delete`   | 多会话 list + delete 单个                  |
| `list_with_metadata_returns_all`      | 返回所有会话含 title / message_count       |
| `list_with_metadata_legacy_session`   | **旧 session (无 sidecar) 推导兼容**       |
| `legacy_session_load_returns_none_metadata` | 旧 session load 时元信息字段全 None  |

### 8.4 current_session_id

| 测试                                      | 验证                                |
| ----------------------------------------- | ----------------------------------- |
| `current_session_id_returns_latest`       | 返回最近修改的会话                  |
| `current_session_id_ignores_meta_sidecar` | 跳过 `.meta.json`, 不返回 `xxx.meta` |

### 8.5 杂项

| 测试                          | 验证                                    |
| ----------------------------- | --------------------------------------- |
| `attributable_implementation` | role() / alias() 正确                   |
| `list_empty_dir`              | 空目录返回空列表                        |
| `delete_nonexistent_is_ok`    | 删不存在的不报错                        |
| `delete_removes_both_files`   | delete 同时清掉 `.jsonl` + `.meta.json` |

**全部 30 个 shadow-core 测试通过, workspace 编译通过。**

---

## 9. 使用示例

### 9.1 创建新会话 (运行时对话流)

```rust
use shadow_core::{JsonlSessionStore, SessionStore};
use shadow_core::provider::ChatMessage;

let store = JsonlSessionStore::new("/home/user/.shadow");
let session_id = uuid::Uuid::new_v4().to_string();

// 每轮对话追加 user + assistant
let user_msg = ChatMessage {
    role: "user".into(),
    content: "你好".into(),
    ..Default::default()
};
store.append_message(&session_id, &user_msg).await?;

let assistant_msg = ChatMessage {
    role: "assistant".into(),
    content: "你好! 有什么可以帮你的?".into(),
    ..Default::default()
};
store.append_message(&session_id, &assistant_msg).await?;
```

### 9.2 列出所有会话 (UI 友好)

```rust
let metas = store.list_with_metadata().await?;
for m in &metas {
    println!(
        "[{}] {} ({} 条消息, 更新于 {})",
        m.id,
        m.title.as_deref().unwrap_or("(无标题)"),
        m.message_count,
        m.updated_at.as_deref().unwrap_or("?"),
    );
}
// 不加载任何 messages, O(1) per session
```

### 9.3 加载完整会话

```rust
if let Some(session) = store.load("550e8400-...").await? {
    println!("标题: {:?}", session.title);
    println!("归属: {:?}", session.agent_alias);
    for msg in &session.messages {
        println!("{}: {}", msg.role, msg.content);
    }
}
```

### 9.4 初始化会话带元信息 (一次性)

```rust
use shadow_core::Session;

let session = Session {
    id: "imported-001".into(),
    messages: vec![/* ... */],
    title: Some("从外部导入的会话".into()),
    agent_alias: Some("shadow-importer".into()),
    created_at: None,  // save 会用 now 兜底
    updated_at: None,
};
store.save(&session).await?;  // truncate+write, 覆盖 meta
```

---

## 10. 未来扩展 (预留接口)

### 10.1 多 agent / 多 profile 隔离

`agent_alias` 字段已就位, 未来 `list_with_metadata` 可加过滤:

```rust
// 未来 trait 扩展 (未实现)
async fn list_by_agent(&self, alias: &str) -> Result<Vec<SessionMetadata>>;
```

### 10.2 标题自动生成

当前 `title` 由调用方显式设。未来可在 `append_message` 内部触发 "前 N 条消息总结成标题" 的异步任务 (LLM 调用) -- 不破坏 trait。

### 10.3 向 SQLite 迁移

文件布局已通过 `Workspace::sessions_dir()` 隔离, 未来把 JsonlSessionStore 换成 `SqliteSessionStore` 只需:
- 实现 trait
- Workspace 路径从 `sessions/{id}.jsonl` 改为 `sessions.db`
- 调用方零改动 (依赖 trait, 不依赖具体类型)

### 10.4 软删除 / 归档

`delete` 当前是物理删除。未来可加:

```rust
// 未来 trait 扩展 (未实现)
async fn archive(&self, id: &str) -> Result<()>;  // 移到 archive/
async fn restore(&self, id: &str) -> Result<()>;
```

---

## 11. 不做的事 (YAGNI)

明确排除的复杂化:

- **消息级元信息** (每条消息的 timestamp / token 计数): 应放在 `ChatMessage` 自身, 不污染 session 层
- **多版本 / 历史快照**: git 风格的版本控制不在 MVP 范围
- **跨 session 搜索**: 应由 Memory 层 (recall) 提供, 不在 SessionStore 职责内
- **加密**: 文件级加密应由 OS / 文件系统层负责 (FileVault / LUKS), 不在应用层

---

## 12. 相关文档

- [multi-user-design.md](./multi-user-design.md) -- 多用户/profile 设计, `agent_alias` 的最终用途
- [memory-design.md](./memory-design.md) -- Memory 层设计, 与 Session 的协作模式 (MemoryStrategy)
- [tool-design.md](./tool-design.md) -- Tool 系统, 工具调用产生 ChatMessage 进入 Session
