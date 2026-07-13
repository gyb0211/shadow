# Shadow 数据库与存储设计文档

> 当前用 rusqlite 手写 SQL。是否改 ORM? 评估在最后一节。

## 1. 数据层现状总览

| 存储位置 | 数据类型 | 格式 | 访问方式 |
|---------|---------|------|---------|
| `~/.shadow/sessions/{id}.jsonl` | 会话消息 | JSONL (append-only) | `JsonlSessionStore` |
| `~/.shadow/sessions/{id}.meta.json` | 会话元信息 | JSON | 同上 |
| `~/.shadow/skills/{name}/SKILL.md` | 技能定义 | Markdown + frontmatter | `SkillsService::load_skills_from_dir` |
| `~/.shadow/memory/brain.db` (SQLite) | 记忆条目 + FTS5 + embedding_cache | 单文件 | `SqliteMemory::with_embedder` |
| `~/.shadow/memory/{alias}.md` (markdown) | 记忆条目 | frontmatter + body | `MarkdownMemory::new(alias, workspace)` |
| `~/.shadow/cron/jobs.db` (SQLite) | cron jobs + cron runs | 单文件 | `CronScheduler` (注释中) |
| `~/.shadow/config.toml` | 用户配置 | TOML | `shadow_config::Config::load_or_init` |
| `~/.shadow/state/runtime-trace.jsonl` | 日志事件 | JSONL | `shadow_log::LogCaptureLayer` |

## 2. SQLite 数据库 Schema

### 2.1 shadow-memory/src/sqlite.rs

**memories 表** (核心记忆存储):

```sql
CREATE TABLE IF NOT EXISTS memories (
    id          TEXT PRIMARY KEY,
    key         TEXT NOT NULL UNIQUE,      -- (被 V3 migration 替换为 UNIQUE (agent_id, key))
    content     TEXT NOT NULL,
    category    TEXT NOT NULL DEFAULT 'core',
    embedding   BLOB,                       -- f32 向量小端序
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL,
    session_id  TEXT,                       -- ALTER TABLE v2 加的
    namespace   TEXT DEFAULT 'default',     -- ALTER TABLE v2 加的
    agent_id    TEXT,                       -- V3 migration 加的
    importance  REAL,                       -- V3 加
    kind        TEXT,                       -- V3 加 (Episodic/Semantic/Procedural)
    pinned      INTEGER,                    -- V3 加
    tenant_id   TEXT,                       -- V3 加
    superseded_by  TEXT                     -- V3 加 (conflict detection)
);
CREATE INDEX idx_memories_category ON memories(category);
CREATE INDEX idx_memories_key ON memories(key);
CREATE INDEX idx_memories_session ON memories(session_id);
CREATE INDEX idx_memories_namespace ON memories(namespace);
CREATE INDEX idx_memories_agent_id ON memories(agent_id);  -- V3 加
```

**memories_fts 虚表** (FTS5 全文检索):

```sql
CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(
    key, content, content=memories, content_rowid=rowid
);

-- 同步触发器
CREATE TRIGGER memories_ai AFTER INSERT ON memories BEGIN
    INSERT INTO memories_fts(rowid, key, content) VALUES (new.rowid, new.key, new.content);
END;
CREATE TRIGGER memories_ad AFTER DELETE ON memories BEGIN
    INSERT INTO memories_fts(memories_fts, rowid, key, content) VALUES ('delete', old.rowid, old.key, old.content);
END;
CREATE TRIGGER memories_au AFTER UPDATE ON memories BEGIN
    INSERT INTO memories_fts(memories_fts, rowid, key, content) VALUES ('delete', old.rowid, old.key, old.content);
    INSERT INTO memories_fts(rowid, key, content) VALUES (new.rowid, new.key, new.content);
END;
```

**embedding_cache 表** (LRU 嵌入缓存):

```sql
CREATE TABLE IF NOT EXISTS embedding_cache (
    content_hash TEXT PRIMARY KEY,
    embedding    BLOB NOT NULL,
    created_at   TEXT NOT NULL,
    accessed_at  TEXT NOT NULL
);
CREATE INDEX idx_cache_accessed ON embedding_cache(accessed_at);
```

**agents 表** (V3 agent UUID 映射):

```sql
CREATE TABLE IF NOT EXISTS agents (
    id    TEXT PRIMARY KEY,    -- UUID v4
    alias TEXT NOT NULL UNIQUE -- 与 Config::agents HashMap key 对应
);
```

### 2.2 sql 复杂度分析

shadow-memory/src/sqlite.rs 共 1752 行，**约有 50+ 处手写 SQL 字符串** (从 grep 统计):

| SQL 类型 | 数量 | 复杂度 |
|---------|------|--------|
| INSERT | 8 | 简单 |
| UPDATE | 6 | 简单-中 |
| SELECT (基础) | 15 | 简单 |
| SELECT (JOIN) | 5 | 中 |
| SELECT (FTS5 bm25) | 3 | 复杂 |
| DELETE | 4 | 简单 |
| CREATE/ALTER/DROP | 10 | 中 |
| 触发器 | 3 | 中 |

**示例复杂 SQL**:

```sql
-- 向量混合检索
"SELECT m.id, bm25(memories_fts) as score
 FROM memories_fts f
 INNER JOIN memories m ON m.rowid = f.rowid
 WHERE memories_fts MATCH ?1
   AND m.agent_id = ?2
 ORDER BY score
 LIMIT ?3"
```

```sql
-- 嵌入缺失扫描 (reindex)
"SELECT id, content FROM memories WHERE embedding IS NULL"
```

## 3. 存储后端 (Backends)

### 3.1 Markdown 后端 (`markdown.rs` 366 行)

```rust
pub struct MarkdownMemory {
    alias: String,
    workspace_dir: PathBuf,   // 默认 ~/.shadow/memory/{alias}/ 或 agent workspace
}
```

文件布局:
```
~/.shadow/memory/{alias}/
├── {uuid}.md       // 每条记忆一个文件
│                  // 包含 frontmatter (key/category/timestamp)
│                  // + body (content)
```

无需迁移、人工可读、git-friendly。缺点：检索效率差 (无 FTS5/向量)。

### 3.2 SQLite 后端 (`sqlite.rs` 1752 行)

```rust
pub struct SqliteMemory {
    name: String,                       // "sqlite"
    workspace_dir: PathBuf,             // ~/.shadow/memory/
    embedder: Arc<dyn EmbeddingProvider>,  // Noop 或 OpenAI
    vector_weight: f32,
    keyword_weight: f32,
    embedding_cache_size: usize,
    sqlite_open_timeout: Option<u64>,
    search_mode: SearchMode,
}
```

文件: `{workspace_dir}/brain.db`，单文件 + WAL 模式。

### 3.3 None 后端 (`none.rs` 111 行)

```rust
pub struct NoneMemory { alias: String }
```

所有方法返回 `Ok(())` / `false` / 空。零存储，用于禁用记忆的场景。

## 4. 类型映射: Rust <-> SQL

| Rust 类型 | SQL 类型 | 序列化 |
|-----------|----------|--------|
| `String` (id, key, content) | `TEXT` | `row.get(idx)` |
| `String` (timestamp, RFC 3339) | `TEXT` | `row.get(idx)` |
| `Vec<f32>` (embedding) | `BLOB` (小端) | `vec_to_bytes` / `bytes_to_vec` |
| `Option<String>` | nullable `TEXT` | `row.get::<_, Option<String>>(idx)` |
| `Option<f64>` (importance) | nullable `REAL` | |
| `MemoryCategory` enum | `TEXT` (字符串) | `serde_str` 或手动 match |
| `MemoryKind` enum | `TEXT` | |
| `bool` (pinned) | `INTEGER` (0/1) | `if pinned {1} else {0}` |
| `UUID` (id, agent_id) | `TEXT` | `uuid::Uuid::to_string()` |

每条 SQL row → `MemoryEntry` 都是手写的 `Row -> MemoryEntry` 映射逻辑，散布在 sqlite.rs 各方法中。

## 5. 三种手写 Row 映射示例

### 5.1 直接 row.get

```rust
let mut stmt = conn.prepare("SELECT id, content, category FROM memories WHERE key = ?1")?;
let entry = stmt.query_row(params![key], |row| {
    Ok(MemoryEntry {
        id: row.get(0)?,
        key: row.get(1)?,
        // ...
    })
})?;
```

### 5.2 命名版本 (用 AS alias)

```rust
let sql = "SELECT id AS id, content AS content FROM memories ORDER BY created_at DESC";
stmt.query_map(params![], |row| MemoryEntry::from_row(row))?
```

### 5.3 try_from_row 辅助函数

Shadow 的 sqlite.rs 中有些方法定义了 `fn row_to_entry(row: &Row) -> Result<MemoryEntry>` 帮助函数，但分散、不一致。

## 6. 连接管理

shadow-memory/src/sqlite.rs 的连接模式:

```rust
pub struct SqliteMemory {
    conn: parking_lot::Mutex<Connection>,  // 单一互斥连接
    // ...
}

impl SqliteMemory {
    async fn store(&self, ...) -> Result<()> {
        let conn = self.conn.lock();    // 同步加锁
        conn.execute(...)?;
        Ok(())
    }
}
```

**特点**:
- 单一连接 + Mutex (Sync)
- 所有操作都是 `conn.execute()` 同步
- 没有连接池 (r2d2 / sqlx pool / deadpool)
- 没有 spawn_blocking

**ZeroClaw 也是这种模式** (`parking_lot::Mutex<Connection>` 或 `RwLock<Connection>`)。

## 7. 迁移系统

### 7.1 当前架构 (shadow-memory)

```rust
add_memories_column_if_missing(conn, "session_id", "ALTER TABLE memories ADD COLUMN session_id TEXT")?;
add_memories_column_if_missing(conn, "namespace", "ALTER TABLE memories ADD COLUMN namespace TEXT DEFAULT 'default'")?;
// ... 一连串 ALTER + CREATE INDEX
```

每个迁移是手写的 `add_column_if_missing` + `execute_batch_retry`。

### 7.2 V3 Migration (agent_id + composite unique)

在 `zeroclaw_config::schema::v2::migrate_sqlite_memory_to_v3` 中：

```sql
-- 1. 重建表结构 (SQLite 不支持 ALTER UNIQUE CONSTRAINT)
CREATE TABLE memories_v3 (
    id TEXT PRIMARY KEY,
    agent_id TEXT NOT NULL,
    key TEXT NOT NULL,
    content TEXT NOT NULL,
    -- ... 同 V2 + 新列
    UNIQUE (agent_id, key)  -- 复合唯一
);
INSERT INTO memories_v3 SELECT ..., (SELECT id FROM agents WHERE alias = 'default') FROM memories;
DROP TABLE memories;
ALTER TABLE memories_v3 RENAME TO memories;
```

## 8. 各选项的"换 ORM"评估

### 8.1 ORM 候选对比

| ORM | 类型 | 异步 | 编译时检查 | Schema 迁移 | SQL 表达力 | 学习曲线 |
|-----|------|------|-----------|------------|-----------|---------|
| **rusqlite** (当前) | 微库 (无 ORM) | 同步 (自己包 spawn_blocking) | 无 (字符串 SQL) | 手写 | 100% SQL | 低 |
| **sqlx** | 编译时检查 ORM | 同步 + async (runtime) | ✅ 编译时验证 | sqlx-migrate | 100% SQL | 中 |
| **diesel** | 编译时 ORM | 同步 | ✅ 编译时 | diesel_migrations | DSL | 高 |
| **sea-orm** | Active Record ORM | async | 部分 | sea-orm-cli | DSL | 高 |
| **ormlite** | Active Record ORM | async | 部分 | 手写 migration | DSL | 中 |

### 8.2 三大 ORM 对项目的影响

#### sqlx (推荐选项 A)

**优势**:
- **编译时 SQL 校验**: `sqlx::query!("SELECT * FROM memories WHERE id = ?")` 在编译时连真实 SQLite db 验证 SQL + 列名
- **async 原生支持**: `query_as!` 直接 await，不需要 spawn_blocking
- **零运行时反射**: 纯 SQL + 派生宏，无 ORM DSL 学习
- **保留**: `JsonlSessionStore` 等不变，只换 SqliteMemory
- **migration 子命令**: `sqlx migrate add` + `sqlx migrate run`

**劣势**:
- 编译时需要 `DATABASE_URL` 可访问 (有 offline 模式但要 `.sqlx/` cache)
- 触发器 + 自定义函数 (FTS5 bm25, vec_to_bytes) 仍要 raw SQL
- 编译变慢 (~20%)

**改动量**: 大约把 `execute()/query_row()/query_map()` 换成 `query_as!()/query_as::<T>()/query!()`，约 30+ 处。

**风险**: 触发器定义、PRAGMA 设置、init_schema 流程不变，主要换 query 调用。

#### diesel + sqlite

**优势**:
- 编译时 ORM 模型验证
- 自动化 migration

**劣势**:
- **async 不支持** (async 只在 nightly diesel-async)
- DSL 学习成本高
- `Vec<f32>` BLOB、bm25 FTS5 表达力差，要 escape hatch 用 sql_function
- 本项目已经是 `parking_lot::Mutex<Connection>` 同步访问，diesel 同步模型 OK，但失去 `async fn`

**改动量**: 需要重写所有表为 diesel struct + schema，schema.rs 也要改。

#### sea-orm

**优势**:
- async-native
- Active Record 风格

**劣势**:
- FTS5 + bm25 几乎不支持
- trigger / RAISE 等高级 SQLite 特性不支持
- 编译时检查弱
- 比 sqlx 慢
- 项目已经用 sqlx 风格分离 SQL 更合适

**改动量**: 需重写全部 (schema, migration, query)。

### 8.3 推荐选择: **保留 rusqlite + 加 macro 抽象**

经过权衡，**不建议完全改 ORM**，因为：

1. **ZeroClaw 也不改 ORM** -- 同样用 rusqlite (AGENTS.md "Beta" stability tier，未引入 ORM)。
2. **FTS5 + bm25 + 触发器 + vec_to_bytes/embedding_cache** 是 ORM 不友好的部分。
3. **Shadow 已经是 async + parking_lot::Mutex + 手动 spawn_blocking** 的模式。

但**建议加一个中间层抽象** 减少手写 SQL 负担：

#### 推荐选项 B: schema 集中 + typed query 包装

新增 `crates/shadow-storage` (或扩展 `shadow-memory`):

```rust
// 1. SQL 集中到 schema.rs
pub mod sql {
    pub const INSERT_MEMORY: &str = include_str!("sql/insert_memory.sql");
    pub const FTS_RECALL: &str = include_str!("sql/fts_recall.sql");
    pub const VECTOR_RECALL: &str = include_str!("sql/vector_recall.sql");
    // ...
}

// 2. 类型化 row 映射 trait
pub trait FromRow: Sized {
    fn from_row(row: &Row) -> rusqlite::Result<Self>;
}
impl FromRow for MemoryEntry { ... }

// 3. 类型化 prepare/execute 助手
pub fn query<T: FromRow>(conn: &Connection, sql: &str, params: impl Params) -> Result<Vec<T>>;
```

**改动量**: ~200 行 (单文件抽象)，零运行时开销，保持 rusqlite 直接调用。

#### 推荐选项 C: 选 sqlx 但限定范围

如果坚持要 ORM，**只在 SqliteMemory 内部**用 sqlx:

- `shadow-memory/Cargo.toml` 加 `sqlx = { version = "0.8", features = ["sqlite", "macros", "migrate"] }`
- `shadow-storage-layer/Cargo.toml` 类似
- SqliteMemory 重写为 sqlx::SqlitePool + query! 宏
- 其他组件 (AgentScopedMemory 等) 不变

**优势**: 编译时检查、async-native。
**劣势**: 触发器定义需要 `sqlx::query()` 不带宏，FTS5 bm25 仍用 raw SQL。

## 9. 当前痛点分析

| 痛点 | 影响 | ORM 能解决? |
|------|------|------------|
| sql.rs 1752 行 SQL 字符串散落 | 维护难 | 部分 (sqlx) |
| Row 映射手动且重复 | 易错 | 是 (FromRow trait / ORM 派生) |
| schema 迁移手写 | 多版本并存风险 | 部分 (sqlx migrate) |
| Connection Mutex 是 sync 的 | async 函数锁竞争 | 是 (sqlx pool) |
| 触发器 + FTS5 + vec_to_bytes 无法用 ORM 表达 | 仍需 raw SQL | 部分 (escape hatch) |
| 缺乏 connection pool | 单连接限并发 | 是 (sqlx pool) |
| 缺乏编译时 SQL 校验 | 运行时才发现 | 是 (sqlx macros) |
| TextBlob embedding 序列化 | Vec<f32> 转 bytes | 否 |

### 9.1 实际最痛的不是 ORM，而是缺少:

1. **async connection pool** - 当前单连接 + Mutex 会阻塞其他 async 任务
2. **schema 集中** - SQL 字符串散落 50+ 处，参数顺序靠手工不靠类型
3. **Row → Struct 映射模板** - 60+ 列每次都是手写 row.get
4. **schema migration 表** - 当前没有 `schema_migrations` 表，每次启动都 `CREATE IF NOT EXISTS` + `add_column_if_missing`

## 10. 推荐方案: 介于手写与 ORM 之间

引入 **`shadow-storage` 层**，封装 SQL + Row 映射，给影子后端带来 80% 的 ORM 收益，零额外依赖。

### 10.1 目录结构

```
crates/shadow-storage/   # 新增
├── src/
│   ├── lib.rs
│   ├── schema.rs        # SQL 集中 (include_str!)
│   ├── row.rs           # FromRow trait
│   ├── pool.rs          # Async 连接池 (或单一互斥, 视选用)
│   ├── migrations.rs    # schema_migrations 表 + 顺序
│   └── sql/             # 静态 SQL 文件
│       ├── insert_memory.sql
│       ├── recall_fts.sql
│       ├── recall_vector.sql
│       ├── recall_hybrid.sql
│       └── ...
├── Cargo.toml
```

### 10.2 shadow-storage::schema API

```rust
pub mod sql {
    pub const INSERT_MEMORY: &str = include_str!("sql/insert_memory.sql");
    pub const UPSERT_AGENT: &str = include_str!("sql/upsert_agent.sql");
    // ... 30+ SQL 文件
}

pub trait FromRow: Sized {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self>;
}

pub fn query<T: FromRow>(
    conn: &rusqlite::Connection,
    sql: &str,
    params: impl rusqlite::Params,
) -> rusqlite::Result<Vec<T>>;

pub fn query_one<T: FromRow>(
    conn: &rusqlite::Connection,
    sql: &str,
    params: impl rusqlite::Params,
) -> rusqlite::Result<Option<T>>;
```

### 10.3 改动量评估

| 选项 | 改动行数 | 编译依赖 | 运行时依赖 |
|------|---------|---------|-----------|
| 保留 rusqlite | 0 | 0 | 0 |
| 引入 shadow-storage 抽象 | +500 行新 crate | 0 (复用 rusqlite) | 0 |
| 切换 sqlx | -1500 行 (sql 改 query!) +500 schema.rs | +sqlx ~0.8 | +sqlx ~0.8 |
| 切换 diesel | -1500 +800 | +diesel ~2.0 | +diesel ~2.0 |
| 切换 sea-orm | -1700 +900 | +sea-orm ~1.0 | +sea-orm ~1.0 |

### 10.4 选型建议

| 阶段 | 建议 |
|------|------|
| 当前 (P0) | **保留 rusqlite + 引入 shadow-storage 抽象** (SQL 集中 + FromRow trait) |
| 下一阶段 (P1) | **如连接池 / async 阻塞成瓶颈**, 升级到 rusqlite pool + `tokio::task::spawn_blocking` 包装 |
| 远期 (P3) | 如果扩展到 Postgres / Qdrant + 团队规模变大, 切换到 **sqlx** (因为它的多 backend 抽象 + 编译时校验) |

**最终结论**:

**不建议改 ORM**。当前 rusqlite 与 ZeroClaw 保持一致, FTS5/触发器/向量 BLOB 不适合 ORM。
**建议加 `shadow-storage` 抽象层**, 集中 SQL + row 映射, 仍是 rusqlite 但消除 80% 重复代码, 零额外依赖, 零运行时开销。
