# Shadow Postgres 接入方案 (基于 ZeroClaw 调研)

## ZeroClaw 真实做法: 不改 ORM

**答案**: ZeroClaw 接 Postgres **不用 ORM**, 用原生 `postgres` crate 手写 SQL。

### Cargo.toml 选型

```toml
[dependencies]
postgres = { version = "0.19", features = ["with-chrono-0_4"], optional = true }

[features]
memory-postgres = ["dep:postgres", "zeroclaw-config/memory-postgres"]
```

- **可选 feature**: 不需要 PG 的用户不编译此 crate (零编译开销)
- **同步 crate**: `postgres = "0.19"` 是 Rust 原生 PostgreSQL 驱动 (类似 redis-rs)
- **零 ORM 依赖**: 没有 sqlx / diesel / sea-orm

### 文件规模

| 文件 | 行数 | 复杂度 |
|------|------|--------|
| `postgres.rs` | **1054 行** | 比 sqlite.rs 简单 40% |
| `sqlite.rs` (对照) | 4015 行 | 含触发器+embedding cache+reindex |

PG 不需要 FTS5 触发器、有 pg_trgm/tsvector、有 pgvector 扩展，所以代码量反而少。

## ZeroClaw Postgres 实现核心要点

### 1. 三个关键技术决策

#### 决策 A: 同步操作 + 手写 OS 线程包装

```rust
// postgres crate 是同步的, 不能在 tokio runtime 里直接 await
// 解决方案: 把同步调用包装在 std::thread 里, 再用 tokio::oneshot 桥接

async fn run_on_os_thread<F, T>(f: F) -> Result<T>
where
    F: FnOnce() -> Result<T> + Send + 'static,
{
    let (tx, rx) = oneshot::channel();
    std::thread::Builder::new()
        .name("postgres-memory-op".to_string())
        .spawn(move || {
            let result = f();
            let _ = tx.send(result);
        })?;
    rx.await.map_err(|_| anyhow::Error::msg("postgres thread terminated"))?
}
```

**为什么不上 sqlx 异步**:

> `postgres::Client::drop` calls `Runtime::block_on()` internally. That panics if called from inside an existing Tokio runtime.

也就是说:
- `postgres` crate 自带事件循环, 只能在 OS 线程运行
- sqlx 真正 async, 但要从 tokio 切到 sqlx 的代码迁移很大

#### 决策 B: DropOnThread 类型解决 Drop panics

```rust
struct DropOnThread<T: Send + 'static>(Option<T>);

impl<T: Send + 'static> Drop for DropOnThread<T> {
    fn drop(&mut self) {
        let Some(value) = self.0.take() else { return };
        let slot = std::mem::ManuallyDrop::new(value);
        if std::thread::Builder::new()
            .name("postgres-client-drop".to_string())
            .spawn(move || drop(std::mem::ManuallyDrop::into_inner(slot)))
            .is_err()
        {
            // 备选: 让 Client 泄露 (leak) 也不要在 Tokio 线程上 drop
            // (受控的 leak 优于不可恢复的 panic)
            ::zeroclaw_log::record!(WARN, ..., "postgres-client-drop thread spawn failed; leaking client to avoid nested-runtime panic");
        }
    }
}
```

**测试**: 专项测试验证 `DropOnThread` 在 Tokio runtime 内部 drop 时不会 panic。

#### 决策 C: 单 Mutex 连接 + Arc (无连接池)

```rust
pub struct PostgresMemory {
    alias: String,
    client: DropOnThread<Arc<Mutex<Client>>>,  // 单一互斥连接 (被 Arc 包裹)
    qualified_table: String,    // "public"."memories"
    qualified_agents: String,   // "public"."agents"
}
```

无连接池。原因:
- 单 agent 顺序访问
- async 接口但底层 sync
- `Arc<Mutex<Client>>` 允许多 future 共享 (虽然互斥)

### 2. Schema 设计

```sql
CREATE SCHEMA IF NOT EXISTS public;

CREATE TABLE IF NOT EXISTS "public"."memories" (
    id TEXT PRIMARY KEY,
    key TEXT NOT NULL,
    content TEXT NOT NULL,
    category TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,  -- pg: TIMESTAMPTZ, sqlite: TEXT
    updated_at TIMESTAMPTZ NOT NULL,
    session_id TEXT                    -- nullable 允许无 session
);
CREATE INDEX idx_memories_category ON "public"."memories"(category);
CREATE INDEX idx_memories_session_id ON "public"."memories"(session_id);
CREATE INDEX idx_memories_updated_at ON "public"."memories"(updated_at DESC);

-- PG 原生 FTS, 不同于 SQLite FTS5
CREATE INDEX idx_memories_content_fts 
    ON "public"."memories" 
    USING gin(to_tsvector('simple', content));  -- GIN 全文索引
CREATE INDEX idx_memories_key_fts 
    ON "public"."memories" 
    USING gin(to_tsvector('simple', key));
```

**关键差异**:

| 维度 | SQLite | Postgres |
|------|--------|----------|
| FTS | FTS5 虚表 + 触发器 | GIN 索引 + tsvector |
| 时间 | TEXT (RFC 3339) | TIMESTAMPTZ (原生) |
| 触发器 | 必须 (FTS5 同步) | 不需要 (GIN 自动同步) |
| 全文评分 | bm25(memories_fts) | ts_rank_cd(to_tsvector, plainto_tsquery) |
| Schema | file-local | CREATE SCHEMA public |

### 3. Schema 标识符安全

```rust
fn validate_identifier(value: &str, field_name: &str) -> Result<()> {
    if value.is_empty() { bail!("{field_name} must not be empty"); }
    let mut chars = value.chars();
    let Some(first) = chars.next() else { bail!("..."); };
    if !(first.is_ascii_alphabetic() || first == '_') { bail!("..."); }
    if !chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_') { bail!("..."); }
    Ok(())
}

fn quote_identifier(value: &str) -> String {
    format!("\"{value}\"")
}
```

**作用**: 用户在 config 写 `schema = "public;DROP TABLE x"` 时, 验证 + 引号转义阻止 SQL 注入。

### 4. pgvector 集成

```rust
fn try_enable_pgvector(client: &mut Client, qualified_table: &str, dimensions: usize) -> Result<()> {
    client.batch_execute("CREATE EXTENSION IF NOT EXISTS vector")?;
    client.batch_execute(&format!(r#"
        DO $$ BEGIN
            ALTER TABLE {qualified_table} ADD COLUMN IF NOT EXISTS namespace TEXT DEFAULT 'default';
            ALTER TABLE {qualified_table} ADD COLUMN IF NOT EXISTS importance REAL;
            ALTER TABLE {qualified_table} ADD COLUMN IF NOT EXISTS embedding vector({dimensions});
        EXCEPTION WHEN OTHERS THEN
            RAISE NOTICE 'pgvector columns could not be added: %', SQLERRM;
        END $$;
        CREATE INDEX IF NOT EXISTS idx_memories_namespace ON {qualified_table}(namespace);
    "#))?;
    Ok(())
}
```

**特点**:
- `DO $$ ... $$` PL/pgSQL 匿名块包住 ALTER, 失败 RAISE NOTICE 而非 abort
- 类型 `vector(N)` 是 pgvector 扩展提供, 需 `CREATE EXTENSION IF NOT EXISTS vector`
- 可选启用: `vector_enabled = false` 时退回纯关键词检索

### 5. Recall 实际 SQL

```rust
let stmt = format!("
    SELECT m.id, m.key, m.content, m.category, m.created_at, m.session_id,
           a.alias AS agent_alias, m.agent_id,
           (CASE WHEN to_tsvector('simple', m.key) @@ plainto_tsquery('simple', $1)
                 THEN ts_rank_cd(to_tsvector('simple', m.key), plainto_tsquery('simple', $1)) * 2.0
                 ELSE 0.0 END +
                 CASE WHEN to_tsvector('simple', m.content) @@ plainto_tsquery('simple', $1)
                 THEN ts_rank_cd(to_tsvector('simple', m.content), plainto_tsquery('simple', $1))
                 ELSE 0.0 END) AS score
    FROM {qualified_table} m
    LEFT JOIN {qualified_agents} a ON a.id = m.agent_id
    WHERE ($2::TEXT IS NULL OR m.session_id = $2)
      AND ($1 = '' OR to_tsvector('simple', m.key || ' ' || m.content) @@ plainto_tsquery('simple', $1))
      {time_filter}
    ORDER BY score DESC, m.updated_at DESC
    LIMIT $3
",);
```

**手动拼接 SQL + 参数化**:
- 表名/列名用 format! 拼接 (因为是标识符不是值)
- 值用 `&[&query, &sid, &limit_i64, &s]` 参数化 (防注入)
- 时段过滤 `time_filter` 由 match 拼接, **不是值不参数化, 而是字符串模板拼接**

**注意**: 这里 `$2::TEXT IS NULL OR` 是 PostgreSQL 特有的语法 (类型注解 + 类型转换 + 谓词)。

### 6. row_to_entry 命名访问

```rust
fn row_to_entry(row: &Row) -> Result<MemoryEntry> {
    let timestamp: DateTime<Utc> = row.get("created_at");  // 命名而非按列号
    
    Ok(MemoryEntry {
        id: row.get("id"),
        key: row.get("key"),
        content: row.get("content"),
        category: Self::parse_category(&row.get::<_, String>("category")),
        timestamp: timestamp.to_rfc3339(),
        session_id: row.get("session_id"),
        score: row.try_get("score").ok(),
        namespace: row.try_get::<_, String>("namespace").unwrap_or_else(|_| "default".into()),
        importance: row.try_get("importance").ok(),
        superseded_by: None,
        agent_alias: row.try_get("agent_alias").ok(),
        agent_id: row.try_get("agent_id").ok(),
    })
}
```

**`try_get` 返回 `Result<T>`** —— 当列不存在或为 NULL 时返回 Err 而非 panic, 让 schema migration 时新增列不会破坏老 row。

### 7. 7 个测试用例

```rust
#[test] valid_identifiers_pass_validation() ...        // 验证合法标识符
#[test] invalid_identifiers_are_rejected() ...          // 拒绝非法 (含特殊字符)
#[test] parse_category_maps_known_and_custom_values()  // 枚举解析
#[test] drop_on_thread_drops_value_on_plain_os_thread  // 回归测试 Drop 不会在 Tokio 上 panic
#[test] new_does_not_panic_inside_tokio_runtime         // 即使连接到错误地址也不 panic
... 7 个测试
```

### 8. init_schema 流程

```rust
fn initialize_client(...) -> Result<Client> {
    std::thread::Builder::new().spawn(move || {
        let mut client = config.connect(NoTls)?;
        Self::init_schema(&mut client, &schema_ident, &qualified_table)?;
        zeroclaw_config::schema::v2::migrate_postgres_memory_to_v3(...)?;
        Ok(client)
    })?.join()?
}
```

**两个关键点**:
1. **整块在 OS 线程上做** —— 避免嵌套 Tokio runtime
2. **V3 migration 自动跑** —— `migrate_postgres_memory_to_v3` 升级 schema + backfill agent_id

## Shadow 复刻方案

**结论**: **不需要 ORM**, **不需要 sqlx**, 直接用 `postgres = "0.19"` (同步原生驱动), 拷 ZeroClaw 的设计模式。

### 改动计划 (3 个 PR, 逐步推进)

#### PR 1: 抽 `crates/shadow-storage` 抽象层 (P0, 3 天)

```toml
# crates/shadow-storage/Cargo.toml
[dependencies]
rusqlite = { version = "0.37", features = ["bundled"] }
shadow-core.workspace = true
```

```rust
// crates/shadow-storage/src/lib.rs
pub mod sql {
    include!(concat!(env!("OUT_DIR"), "/embedded_sql.rs")); // 静态 SQL
}

pub trait FromRow: Sized {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self>;
}

pub struct SqliteBackend { /* 当前 SqliteMemory 提到这里 */ }
impl SqliteBackend {
    pub async fn store(&self, key: &str, content: &str, category: MemoryCategory, session_id: Option<&str>) -> Result<()>;
    // 30+ 方法
}
```

shadow-memory 改成调用 shadow-storage::SqliteBackend, 然后 wrap AgentScoped。

#### PR 2: 接 Postgres (P1, 1 周)

```toml
# crates/shadow-postgres/Cargo.toml
[dependencies]
shadow-storage.workspace = true
shadow-core.workspace = true
postgres = { version = "0.19", features = ["with-chrono-0_4"] }
tokio = { version = "1", features = ["sync"] }
uuid.workspace = true
sha2 = "0.10"
chrono.workspace = true
parking_lot = "0.12"

[features]
memory-postgres = []
```

```rust
// crates/shadow-postgres/src/lib.rs
pub struct PostgresMemory {
    alias: String,
    client: DropOnThread<Arc<Mutex<Client>>>,
    qualified_table: String,
    qualified_agents: String,
}

impl PostgresMemory {
    pub fn new(alias: &str, db_url: &str, schema: &str, table: &str,
              pgvector_enabled: Option<bool>, pgvector_dimensions: Option<usize>) -> Result<Self> {
        // 抄 ZeroClaw: OS thread + init_schema + V3 migration
    }
}

struct DropOnThread<T: Send + 'static>(Option<T>);
impl<T: Send + 'static> Drop for DropOnThread<T> { /* 抄 ZeroClaw */ }

async fn run_on_os_thread<F, T>(f: F) -> Result<T> where F: FnOnce() -> Result<T> + Send + 'static { /* 抄 */ }

fn validate_identifier(value: &str, field_name: &str) -> Result<()> { /* 抄 */ }
fn quote_identifier(value: &str) -> String { format!("\"{value}\"") }

#[async_trait]
impl Memory for PostgresMemory {
    async fn store(&self, ...) -> Result<()> { /* OS thread 包装 + INSERT */ }
    async fn recall(&self, ...) -> Result<Vec<MemoryEntry>> { /* OS thread + tsvector 查询 */ }
    // ...
}
```

#### PR 3: shadow-memory trait 抽象统一 (P2, 3 天)

```rust
// crates/shadow-storage/src/lib.rs 新增
#[async_trait]
pub trait AsyncBackend: Send + Sync {
    async fn store(&self, ...);
    async fn recall(&self, ...) -> Result<Vec<MemoryEntry>>;
    // 30+ 方法
}

// SqliteBackend 和 PostgresMemory 都实现 AsyncBackend
// shadow-memory 的 AgentScopedMemory 改用 Arc<dyn AsyncBackend>
```

## 关键设计决策

### 为什么不用 sqlx

| 痛点 | 实际影响 |
|------|----------|
| sqlx::query! 宏要 DATABASE_URL 编译时 | Shadow 当前没 setup, 引入复杂度 |
| sqlx 真正 async, 但 Shadow trait 是 async fn (用 spawn_blocking 包装 sync 操作) | 大改写 |
| sqlx 不简化 backfill/hygiene 逻辑 | 仍要手写 |
| sqlx PG 类型和 SQLite 类型不同 | 写两套 FromRow |
| sqlx 编译慢 | +15-20% |
| **postgres crate 1054 行已经能解决问题** | **ZeroClaw 验证可行** |

**结论**: postgres crate 已经够用。零额外抽象, 零 ORM 学习曲线, 零编译时数据库依赖。

### 为什么不用 sqlx 的同时, 又要 `shadow-storage` 抽象

| 收益 | 说明 |
|------|------|
| **统一 FromRow** | sqlite/PG 各自的 row_to_entry 收敛到一处 |
| **统一 SQL 文本** | 50+ SQL 字符串集中到 `sql/` 目录, 可以做 diff/review |
| **统一 migration 表** | 当前零, 要加 |
| **统一错误** | anyhow::Result 在两层一致 |
| **未来支持 Qdrant/Lucid** | 同样的 trait 抽象 |

**绝对收益**: ~60% 的重复代码消失。开发新后端 (PG) 时, 复用 trait 和 SQL 模板。

## 与 Shadow 现状的具体对应

| 零Claw 文件 | Shadow 等价 | 备注 |
|------------|-----------|------|
| `crates/zeroclaw-memory/` | `crates/shadow-memory/` | 同名, 同 responsibility |
| `src/postgres.rs` (1054行) | ❌ 缺失 | 需要新建 |
| `src/sqlite.rs` (4015行) | `src/sqlite.rs` (1752行) | 56%, 缺失 vacuum/reindex/embedding_cache 优化 |
| `src/traits.rs` (Memory trait) | `shadow-core/kennel/memory.rs` | Shadow 已有 |
| `src/agent_scoped.rs` (862行) | `agent_scoped.rs` (418行) | 已抄 |
| `src/embeddings.rs` (474行) | `embedding.rs` (229行) | 49%, 简化版 |
| `src/vector.rs` (~150行) | `vector.rs` (147行) | 接近 |
| `src/conflict.rs` (251行) | `conflict.rs` (18行) | 仅 mark_superseded |
| `src/hygiene.rs` (409行) | ❌ 缺失 | 重要功能 |
| `src/decay.rs` | ❌ 缺失 | 时间衰减 |
| `src/importance.rs` | ❌ 缺失 | 重要性评分 |
| `src/consolidation.rs` | ❌ 缺失 | LLM consolidation |
| `src/lucid.rs` | ❌ 缺失 | 后端包装器 |
| `src/audit.rs` | ❌ 缺失 | 审计日志 |
| `src/knowledge_graph.rs` | ❌ 缺失 | PG 知识图谱 |

## 不接 ORM 的具体理由 (用户问的)

**Q: 远程数据库必须 ORM?**

**A: 不必须**。`postgres = "0.19"` 是 Rust 官方维护的同步驱动, 用于生产的案例:
- Hermes (你的项目): 现有 0 个 ORM
- ZeroClaw: postgres 0.19 同步, 25 个 crate
- SQLx 文档: "我们支持运行时查询, 不强制 ORM"

**真正需要的不是 ORM, 是这些**:

| 需要 | 解决方案 | 非 ORM |
|------|---------|--------|
| 远程连接 | `postgres = "0.19"` + 连接池 (PgBouncer) | ✅ |
| async 兼容 | `run_on_os_thread` + oneshot 桥接 | ✅ |
| SQL 注入防护 | 参数化 `&[&val]` + 标识符 quote | ✅ |
| Schema 迁移 | ZeroClaw V3 migration 手写 | ✅ |
| 类型化 row | 手写 `row_to_entry`, 不要重写 60+ 列 | ✅ |
| 多 backend 共用 trait | 抽 `shadow-storage::AsyncBackend` trait | ✅ (无 ORM) |

**ORM 解决的问题, 我们不需要**:
- ✅ 编译时 SQL 校验 (sqlx) → 用 `row.get::<String>("col_name")` + `?` + 手工测试已足够
- ✅ 自动 join/group_by DSL (diesel) → postgres.rs 用 1054 行手写, 70% 是 SQL, 30% 是 Rust mapping
- ✅ 自动 migration 工具 (diesel_migrations) → 我们有自己的 V3 schema migration
- ✅ Active Record 模式 (sea-orm) → 不适合底层 storage 层, 抽象总会存在边界

## 接入步骤

### Step 1: 现在 (1 天)

抽 `crates/shadow-storage` 抽象:

```bash
mkdir -p crates/shadow-storage/src/sql
```

把 `crate::sqlite.rs` 的 50+ SQL 字符串拆出来:

```rust
// crates/shadow-storage/src/sql/recall_fts.sql
"SELECT m.id, m.key, m.content, m.category, m.created_at, m.session_id,
        to_tsvector('simple', m.key || ' ' || m.content) @@ plainto_tsquery('simple', $1) AS hit
 FROM {table} m
 WHERE ($2::TEXT IS NULL OR m.session_id = $2)
   AND ($1 = '' OR to_tsvector('simple', m.key || ' ' || m.content) @@ plainto_tsquery('simple', $1))
 ORDER BY rank DESC, m.updated_at DESC
 LIMIT $3"
```

### Step 2: 接 Postgres (1 周)

```bash
mkdir -p crates/shadow-postgres
```

按 ZeroClaw 的 1054 行 postgres.rs 写一份适配 Shadow 的版本。**复用**:
- ✅ trait 签名 (shadow-core::Memory)
- ✅ DropOnThread / run_on_os_thread (ZeroClaw 实现, ZeroClaw 测试覆盖)
- ✅ validate_identifier / quote_identifier
- ✅ row_to_entry 命名访问模式

**新增**:
- ✅ pgvector (可选 feature)
- ✅ tsvector + ts_rank_cd 评分 (替代 SQLite bm25)
- ✅ UUID TIMESTAMPTZ 转换

### Step 3: 统一 trait (3 天)

把 `crates/shadow-storage/src/lib.rs` 升级:

```rust
#[async_trait]
pub trait AsyncBackend: Send + Sync {
    async fn store(&self, ...) -> Result<()>;
    // 30+ 方法
}
```

shadow-memory 改用 `Arc<dyn AsyncBackend>` 替代 `Arc<dyn Memory>` 实现 AgentScoped。

## 总结

| 问题 | 答案 |
|------|------|
| 接 Postgres 要用 ORM 吗? | **不需要** |
| ZeroClaw 用什么? | `postgres = "0.19"` 同步原生驱动 |
| 我们用什么? | 同样 `postgres = "0.19"` (以 feature gate `memory-postgres`) |
| 抽象层要做多少? | 加 `shadow-storage` trait + FromRow + SQL 集中, 不到 500 行 |
| ORM 的真实替代品 | `postgres` crate + 手写 trait 抽象 |
| 是否值得抽象 | 是 (统一 trait 后, 新后端复用所有 SQL 模板) |

**最关键的一点: ZeroClaw 已经验证这条路在生产环境能跑起来, 不要重新设计轮子**。
