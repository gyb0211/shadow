# Shadow Memory 设计文档

> 参考 ZeroClaw memory_traits.rs (557行) + memory_strategy.rs (118行) + memory_loader.rs (448行)

## 一、ZeroClaw 的 Memory 架构

### 1.1 Memory Trait (zeroclaw-api/src/memory_traits.rs)

```
trait Memory: Send + Sync + Attributable {
    // 核心 CRUD
    store(key, content, category, session_id) -> ()
    recall(query, limit, session_id, since, until) -> Vec<MemoryEntry>
    get(key) -> Option<MemoryEntry>
    list(category, session_id) -> Vec<MemoryEntry>
    forget(key) -> bool
    
    // Agent 作用域 (多 Agent 隔离)
    get_for_agent(key, agent_id) -> Option<MemoryEntry>
    forget_for_agent(key, agent_id) -> bool
    
    // 批量操作
    purge_namespace(namespace) -> usize
    purge_session(session_id) -> usize
    purge_session_for_agent(session_id, agent_id) -> usize
    purge_agent(agent_alias) -> usize
    export_agent(agent_alias) -> Vec<MemoryEntry>
    rename_agent(from, to) -> usize
    count_agent(agent_alias) -> usize
    
    // 维护
    count() -> usize
    health_check() -> bool
    reindex() -> usize
    
    // 高级
    store_procedural(messages, session_id) -> ()
    recall_namespaced(namespace, query, limit, ...) -> Vec<MemoryEntry>
    export(filter) -> Vec<MemoryEntry>
}
```

### 1.2 MemoryEntry 结构

```
struct MemoryEntry {
    id: String,
    key: String,
    content: String,
    category: MemoryCategory,  // Core / Daily / Conversation / Custom(String)
    timestamp: String,         // RFC 3339
    session_id: Option<String>,
    score: Option<f64>,        // 检索相关度
    namespace: String,         // 隔离 (默认 "default")
    importance: Option<f64>,   // 0.0-1.0 优先级
    superseded_by: Option<String>, // 被新条目取代
    agent_alias: Option<String>,   // 显示用
    agent_id: Option<String>,      // 作用域用
}
```

### 1.3 MemoryStrategy (memory_strategy.rs)

```
trait MemoryStrategy {
    // 对话前: 加载相关记忆到上下文
    before_chat(messages, session_id) -> Vec<MemoryEntry>
    
    // 对话后: 自动提取重要事实存储
    after_chat(messages, session_id) -> ()
    
    // 定期维护: 合并/归档/清理
    run_governance() -> ()
}
```

### 1.4 MemoryLoader (memory_loader.rs)

```
trait MemoryLoader {
    // 从 user message 提取搜索关键词
    extract_queries(message) -> Vec<String>
    
    // 调用 memory.recall() 检索
    load(memory, queries, limit) -> Vec<MemoryEntry>
    
    // 格式化为 system prompt 注入
    format(entries) -> String
}
```

### 1.5 后端实现

ZeroClaw 支持:
- NoneMemory (空)
- MarkdownMemory (文件)
- SqliteMemory (SQLite + FTS5)
- PostgresMemory (PostgreSQL + pgvector)
- QdrantMemory (向量数据库)

## 二、Shadow 当前状态

### 2.1 Memory Trait (shadow-core/src/memory.rs, 151行)

```
trait Memory: Attributable {
    store(entry: &MemoryEntry) -> ()
    recall(query, limit) -> Vec<MemoryEntry>
    get(key) -> Option<MemoryEntry>
    list() -> Vec<MemoryEntry>
    forget(key) -> ()
}
```

### 2.2 MemoryEntry

```
struct MemoryEntry {
    id: String,
    key: String,
    content: String,
    category: String,           // 简单字符串 (非枚举)
    timestamp: DateTime<Utc>,   // chrono 类型 (非 RFC 3339 字符串)
    session_id: Option<String>,
    agent_alias: Option<String>,
}
```

### 2.3 后端实现

- NoneMemory (35行) -- 完整
- MarkdownMemory (124行) -- 基本功能, 无 frontmatter 解析
- SqliteMemory (396行) -- FTS5 + trigram, 基本完整
- MemoryStrategy (151行) -- DefaultMemoryStrategy 已有

### 2.4 问题清单

| # | 问题 | 严重度 | 说明 |
|---|------|--------|------|
| 1 | store 签名不合理 | P0 | ZeroClaw: store(key, content, category, session_id) 分参数; Shadow: store(&MemoryEntry) 要调用方构造完整条目 |
| 2 | 缺 session_id 过滤 | P0 | recall() 不支持按 session 过滤, 无法做会话级记忆 |
| 3 | 缺时间范围过滤 | P1 | recall() 不支持 since/until |
| 4 | category 是字符串 | P1 | 应该是枚举 (Core/Daily/Conversation/Custom) |
| 5 | 缺 namespace | P1 | 无法隔离不同 agent 的记忆 |
| 6 | 缺 importance | P2 | 无法按优先级检索 |
| 7 | 缺 count/health_check | P1 | 无法做健康检查 |
| 8 | Markdown get() 不解析 frontmatter | P1 | 丢失 id/category/timestamp 元数据 |
| 9 | 无 MemoryStrategy 集成 | P0 | Agent.chat() 不调用记忆策略, 记忆系统形同虚设 |
| 10 | 无 MemoryLoader | P0 | 不会从用户消息提取关键词检索记忆 |

## 三、Shadow Memory 改进设计

### 3.1 新 Memory Trait

```rust
#[async_trait]
pub trait Memory: Attributable {
    /// 后端名称
    fn name(&self) -> &str;

    /// 存储记忆 -- 分参数, 调用方不需要构造完整 MemoryEntry
    async fn store(
        &self,
        key: &str,
        content: &str,
        category: MemoryCategory,
        session_id: Option<&str>,
    ) -> Result<()>;

    /// 检索记忆 -- 支持关键词 + session + 时间范围
    async fn recall(
        &self,
        query: &str,
        limit: usize,
        session_id: Option<&str>,
    ) -> Result<Vec<MemoryEntry>>;

    /// 获取单条记忆
    async fn get(&self, key: &str) -> Result<Option<MemoryEntry>>;

    /// 列出记忆 (可选按 category 过滤)
    async fn list(&self, category: Option<&MemoryCategory>) -> Result<Vec<MemoryEntry>>;

    /// 删除记忆, 返回是否删除成功
    async fn forget(&self, key: &str) -> Result<bool>;

    /// 记忆总数
    async fn count(&self) -> Result<usize>;

    /// 健康检查
    fn health_check(&self) -> bool;
}
```

### 3.2 新 MemoryEntry

```rust
pub struct MemoryEntry {
    pub id: String,
    pub key: String,
    pub content: String,
    pub category: MemoryCategory,
    pub timestamp: String,           // RFC 3339 字符串 (序列化友好)
    pub session_id: Option<String>,
    pub score: Option<f64>,          // 检索相关度
    pub agent_alias: Option<String>,
}

pub enum MemoryCategory {
    Core,           // 长期事实/偏好
    Daily,          // 日常会话
    Conversation,   // 对话上下文
    Custom(String), // 自定义
}
```

### 3.3 MemoryStrategy Trait

```rust
#[async_trait]
pub trait MemoryStrategy: Send + Sync {
    /// 对话前: 检索相关记忆, 返回注入 system prompt 的文本
    async fn before_chat(&self, user_message: &str, session_id: Option<&str>) -> Result<String>;

    /// 对话后: 从对话中提取重要事实存储
    async fn after_chat(&self, user_message: &str, assistant_response: &str, session_id: Option<&str>) -> Result<()>;
}
```

### 3.4 Agent 集成

Agent.chat_with_stream() 流程:
1. before_chat(user_message) → 获取相关记忆, 注入 system prompt
2. 正常 LLM 调用 + 工具循环
3. after_chat(user_message, response) → 自动提取事实存储

### 3.5 Markdown 后端改进

- 正确解析 frontmatter (id, key, category, timestamp, session_id)
- 正确序列化 frontmatter
- 按 session_id 子目录隔离

### 3.6 SQLite 后端改进

- store() 改为新签名
- recall() 加 session_id 过滤
- 加 count() / health_check()
- category 列改为存储枚举字符串

## 四、与 ZeroClaw 的差异 (刻意精简)

| 维度 | ZeroClaw | Shadow | 原因 |
|------|----------|--------|------|
| Agent 作用域 | get_for_agent / forget_for_agent | 不需要 | Shadow 单 Agent |
| namespace | 支持 | 不需要 | Shadow 无多命名空间 |
| purge_* | 5 种批量删除 | 不需要 | Shadow 记忆量小 |
| rename_agent | 支持 | 不需要 | Shadow 无 Agent 重命名 |
| store_procedural | 支持 | 不需要 | Shadow 无过程记忆 |
| export | GDPR 导出 | 不需要 | Shadow 非生产级 |
| reindex | 重建索引 | 不需要 | Shadow 无向量索引 |
| importance | 0.0-1.0 | 不需要 | Shadow 关键词检索够用 |
| superseded_by | 取代链 | 不需要 | Shadow 记忆简单覆盖 |
| MemoryCategory | 枚举 | 枚举 | 对齐 |
| MemoryStrategy | 有 | 有 | 对齐 |
| MemoryLoader | 有 | 合并到 Strategy | 简化 |
