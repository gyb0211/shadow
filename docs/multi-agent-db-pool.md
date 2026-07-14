# Shadow 多 Agent 数据库连接池设计

> 关键问题: 每个 agent 一个连接? 共享连接池? 多进程? 跨 agent 锁竞争?
> 完整答案基于 ZeroClaw 实战经验

## 答案: ZeroClaw 不用连接池

**核心结论**: ZeroClaw 整个项目 (`crates/zeroclaw-memory/`) 不管 SQLite 还是 Postgres, **都使用单连接 + parking_lot::Mutex** 串行化。**没有连接池**。

```
postgres.rs:
pub struct PostgresMemory {
    client: DropOnThread<Arc<Mutex<Client>>>,    // 单一互斥连接
}
```

```
sqlite.rs:
pub struct SqliteMemory {
    conn: Arc<Mutex<Connection>>,                  // 单一互斥连接
}
```

### 为什么不用连接池?

| 反对连接池的理由 | 实际影响 |
|-----------------|----------|
| **Memory trait 是 sync 接口** | `async fn recall()` 内部调 `run_on_os_thread` 把 sync 操作跨线程; 加连接池需要更多 OS 线程 |
| **单 agent 顺序访问** | 每个 agent 实例只有一个 Backend, 该 agent 自己的工作流串行 |
| **多 agent 共享一个进程** | 多个 AgentScopedMemory 各持一个 `Arc<Mutex<Client>>` 实例, 但它们可以是**同一个 Client** (共享 backend) |
| **Postgres connection 重量** | 但单进程 Postgres connection ≈ 10MB, 100 个 agent ≈ 1GB, 通常不会到这规模 |
| **死锁风险** | 加 R2D2 + 递归 Mutex + 跨 OS 线程 容易死锁; 单连接更简单 |

## 1. 三种连接管理方案对比

### 方案 A: 单连接 + Arc<Mutex> (ZeroClaw 当前)

```rust
pub struct PostgresMemory {
    client: DropOnThread<Arc<Mutex<Client>>>,
}
```

**优点**:
- ✅ 简单, 无池管理复杂度
- ✅ 无死锁风险
- ✅ 连接数 = 1, 资源用量小
- ✅ 适配 sync 代码路径

**缺点**:
- ❌ 所有 recall/insert 串行化, **并发上限 = 1**
- ❌ 跨 agent 访问共享 backend 时也只能排队
- ❌ SQLite 还好 (本地无锁), Postgres 在 100 TPS 时可能成为瓶颈

**适配场景**:
- 单进程 agent (< 50 TPS)
- 后端 SQLite (本地, 0 网络)
- 低并发 (< 10 同时 in-flight 请求)

### 方案 B: 连接池 `deadpool-postgres` (sqlx 时代)

```rust
pub struct PooledPostgresMemory {
    pool: deadpool_postgres::Pool,
    qualified_table: String,
    qualified_agents: String,
}

async fn store(&self, key, content, category, session_id) -> Result<()> {
    let client = self.pool.get().await?;  // async acquire
    client.execute(...).await?;
    Ok(())
}
```

**优点**:
- ✅ 并发 = pool_size (默认 16)
- ✅ sqlx 原生支持
- ✅ 多 agent 高并发下吞吐量高

**缺点**:
- ❌ 需要 sqlx 依赖
- ❌ POSTGRES 连接占用大 (10MB/连接)
- ❌ ZeroClaw 不用, 我们抄需兼容

### 方案 C: 多 Backend 共享一个 pool (按 backend 类型)

```rust
pub struct MemoryHub {
    backends: HashMap<BackendKey, Arc<dyn AsyncBackend>>,
    pg_pool: deadpool_postgres::Pool,    // 共享一个池
}

pub enum BackendKey {
    Sqlite(agent_alias),          // per-agent 独立 SQLite
    Postgres(shared_schema),      // 跨 agent 共享 PG schema
    Markdown(workspace),
}
```

**优点**:
- ✅ SQLite agent 仍独立 (本地 + zero overhead)
- ✅ Postgres agent 共享池 (避免每 agent 10MB)
- ✅ 灵活

**缺点**:
- ❌ 复杂度上升
- ❌ 需要 backend 注册/查找逻辑
- ❌ 跨后端 schema migration 难度

## 2. Shadow 多 Agent 数据库连接设计

### 当前 Shadow 状态

```
shadow-memory/src/agent_scoped.rs (418行)
  ├─ PostgresMemory: Box<dyn Memory>
  └─ AgentScopedMemory: Arc<dyn Memory> + agent_id + allowlist
```

✅ AgentScoped 已抄 ZeroClaw, 单一 Arc 后端。

**问题**: 如果 100 个 agent 共享一个 PG backend, 每个 agent 各创建一个 `Arc::new(PostgresMemory::new(...))` 会创建 100 个连接 (即使 backend 指向同一数据库)。

### 推荐: Shadow 多 Agent 数据库架构

```
┌──────────────────────────────────────────────────────────────────┐
│                 MemoryRegistry (进程级单例)                        │
│                                                                  │
│   pg_pool: deadpool_postgres::Pool     (max_size: 16)            │
│   sqlite_pool: HashMap<agent_alias, Arc<SqliteMemory>>           │
│                                                                  │
│   get(agent_alias) -> Arc<dyn AsyncBackend>                     │
│   ├── 后端=Sqlite → 新建/获取 SqliteMemory                       │
│   └── 后端=Postgres → 共享 pg_pool (postgres agent_id 标识)        │
└──────────────────────────────────────────────────────────────────┘
                              │
        ┌─────────────────────┼─────────────────────┐
        ▼                     ▼                     ▼
AgentScopedMemory        AgentScopedMemory     AgentScopedMemory
(每个 agent 一个)         (共享一个 pg_pool)     (本地 SQLite)
```

### 关键决策: 共享 Postgres pool

每个 agent 创建时:

```rust
async fn create_memory_for_agent(config, agent_alias, api_key) -> Result<Arc<dyn Memory>> {
    let backend_kind = agent_cfg.memory.backend;
    
    if matches!(backend_kind, MemoryBackendKind::Postgres) {
        // 共享一个全局 pool, 每个 agent 用不同 schema 或 agent_id 隔离
        let pool = MEMORY_HUB.get_or_init_pg_pool(&config.memory).await?;
        let mem = PgMemory::new(pool.clone(), agent_alias, ...);
        return Ok(Arc::new(AgentScopedMemory::new(Arc::from(mem), ...)));
    }
    
    if matches!(backend_kind, MemoryBackendKind::Sqlite) {
        // SQLite: 每个 agent 一个独立连接 (本地, 0 网络)
        let mem = SqliteMemory::new("sqlite", &agent_workspace);
        return Ok(Arc::new(AgentScopedMemory::new(Arc::from(mem), ...)));
    }
    // ...
}
```

**为什么这样设计**:
- **Postgres agent (N个)**: 共享 1 个 pool (max 16 连接) → **N × 1 连接数 (池内复用)**
- **SQLite agent (M个)**: 各 1 个本地连接 → M × 1
- **总计**: `max(N, 16) + M` ≈ 0 ~ 116 连接 (规模可预测)

## 3. 锁竞争与并发问题

### 3.1 单 Mutex 的并发瓶颈

```rust
// 当前模式 (sync, 互斥):
async fn recall(&self, query: &str) -> Result<Vec<MemoryEntry>> {
    let mut client = self.client.lock();     // <- 所有 recall 在这排队
    let rows = client.query("...", &[...])?;
    Ok(rows.iter().map(Self::row_to_entry).collect())
}
```

- 同时 N 个 agent + N 个 recall/instance → **最多 1 个 in-flight**
- Postgres 连接打开文件描述符, 排队 10 个请求 ≈ 0.1s × 10 = 1s 平均响应

**Shadow 实际**: agent core 是 `parking_lot::Mutex` (非 std, 自旋), 实测 100ns 量级, 数据库 IO 慢 1ms 起步, mutex 影响可忽略。

**真正瓶颈**: 数据库操作本身 (round-trip), 不是 Mutex 本身。

### 3.2 多 agent 并发 store 冲突

```rust
// Agent A: store("user_pref", "dark_mode")
// Agent B: store("user_pref", "light_mode")
// 都通过 store_with_agent 自动 stamp agent_id
// 通过 agent_id + key 的 UNIQUE (composite) 约束防冲突
```

**冲突解决**: V3 migration 的 `UNIQUE (agent_id, key)` ✅

### 3.3 Cron job 同时触发多个 agent recall

```rust
// CronScheduler 同时调度 10 个 agent job
// 每个 job 独立 AgentScopedMemory (独立 Mutex)
// 但如果 10 个都是 postgres, 共享 pool (max 16) → 不会排队
```

## 4. Postgres 连接池具体实现 (基于 deadpool-postgres)

### 4.1 Cargo.toml

```toml
[dependencies]
postgres = { version = "0.19", features = ["with-chrono-0_4"] }
deadpool-postgres = "0.14"
```

### 4.2 PostgresPool 抽象

```rust
// crates/shadow-postgres/src/pool.rs
use deadpool_postgres::{Manager, Pool, PoolConfig};

pub struct PgPoolConfig {
    pub db_url: String,
    pub max_size: usize,                       // 默认 16
    pub connect_timeout_secs: Option<u64>,
    pub idle_timeout_secs: Option<u64>,
}

pub type PgPool = Pool;

pub fn build_pool(config: &PgPoolConfig) -> anyhow::Result<PgPool> {
    let pg_config: postgres::Config = config.db_url.parse()?;
    
    let mut pool_config = PoolConfig::new(config.max_size);
    if let Some(secs) = config.connect_timeout_secs {
        pool_config.connect_timeout = Some(Duration::from_secs(secs));
    }
    
    let manager = Manager::from_config(
        pg_config,
        tokio_postgres::NoTls,
        DeadpoolPoolConfig::default(),
    );
    
    Pool::builder(manager)
        .max_size(config.max_size)
        .config(pool_config)
        .build()
        .context("failed to create Postgres pool")
}
```

### 4.3 Agent 共享 Pool

```rust
// crates/shadow-memory/src/lib.rs
use once_cell::sync::OnceCell;
use deadpool_postgres::Pool as PgPool;

static PG_POOL: OnceCell<PgPool> = OnceCell::new();

pub async fn init_pg_pool(config: &MemoryConfig) -> anyhow::Result<()> {
    PG_POOL.get_or_try_init(|| async {
        build_pool(&PgPoolConfig {
            db_url: config.db_url.clone().unwrap_or_default(),
            max_size: config.pg_pool_max_size.unwrap_or(16),
            connect_timeout_secs: config.pg_connect_timeout_secs,
            idle_timeout_secs: config.pg_idle_timeout_secs,
        })
    }).await?;
    Ok(())
}

pub async fn create_memory_for_agent(config, agent_alias, api_key) -> Result<Arc<dyn Memory>> {
    let backend_kind = agent_cfg.memory.backend;
    
    if matches!(backend_kind, MemoryBackendKind::Postgres) {
        let pool = PG_POOL.get().context("PG pool not initialized")?;
        let mem = PgMemory::new(pool.clone(), agent_alias, ...);
        return Ok(Arc::new(AgentScopedMemory::new(Arc::from(mem), ...)));
    }
    // ... 其他后端不变
}
```

### 4.4 PgMemory with pool

```rust
// crates/shadow-postgres/src/lib.rs
pub struct PgMemory {
    alias: String,
    pool: PgPool,                              // 共享 pool
    schema: String,
    table: String,
    agent_id: Option<String>,
}

#[async_trait]
impl Memory for PgMemory {
    async fn recall(&self, ...) -> Result<Vec<MemoryEntry>> {
        // 每次从 pool 获取连接 (释放后归还)
        let client = self.pool.get().await?;
        let rows = client.query(...).await?;       // 原生 async
        Ok(rows.iter().map(...).collect())
    }
}
```

**关键收益**:
- ✅ Postgres **连接数 = pool_size** (固定 16), 与 agent 数无关
- ✅ N 个 Postgres agent 总开销: max(N, 16) 个 OS 线程 + 16 个 Postgres 连接到 server
- ✅ recall/store 真正 async, 无 Mutex 阻塞

## 5. SQLite Backend 池决策

### 5.1 SQLite 为什么不需要池

```
SQLite 设计: 单文件, 单进程, 单写者
- WAL 模式: 1 写者 + N 读者 (本地 OS 进程内)
- 文件系统本地: 0 网络往返
- 多连接 = 多 in-process SQLite handle, 共享 page cache
- 数据库大小通常 < 1GB, 全内存 cache 可行
```

**每个 agent 一个 SQLite 连接没问题**:
- 100 个 agent × 1 个连接 = 100 个 SQLite handle
- 每个 handle 占用 ~10KB 内存
- 总计 ~1MB 内存占用 (微不足道)

### 5.2 Shadow 当前 SQLite 设计

```rust
// SqliteMemory: 单一连接
pub struct SqliteMemory {
    conn: parking_lot::Mutex<Connection>,
}
```

**多 agent 行为**:
- Agent A 创建 SqliteMemory A: 打开 `~/.shadow/memory/A/brain.db`
- Agent B 创建 SqliteMemory B: 打开 `~/.shadow/memory/B/brain.db`
- 两个 agent 完全独立的连接 + 文件, 并发 OK

### 5.3 共享 SQLite Backend 也 OK

如果想节省文件描述符:

```rust
pub struct SqliteHub {
    backends: HashMap<PathBuf, Arc<SqliteMemory>>,  // 同一路径共享一个 SqliteMemory
}
```

但通常没必要, 因为本地性能足够, 资源不是问题。

## 6. 多 Agent 数据库连接 Top-3 实战模式

### 模式 1: 每 Agent 独立 Backend (最简单, 当前 Shadow)

```rust
// AgentFactory:
agent → Arc<SqliteMemory>::new(alias_workspace)    // 独立 .db
     → Arc<AgentScopedMemory>::new(...)

// AgentFactory:
agent → Arc<PostgresMemory>::new(db_url, alias)    // 独立 connect
     → Arc<AgentScopedMemory>::new(...)
```

**适用**: 1-10 个 agent, 中低并发, 简单部署。

### 模式 2: Postgres agent 共享 Pool (推荐, 多 Postgres agent)

```rust
// 启动时:
init_pg_pool(&config.memory).await?;  // 全局 1 个 Pool

// AgentFactory:
if backend == Postgres {
    let pool = PG_POOL.get().unwrap();
    let mem = PgMemory::new(pool.clone(), alias);   // 每个 agent 一个 PgMemory, 共享 pool
    Ok(Arc::new(AgentScopedMemory::new(...)))
}
```

**适用**: 5-100 个 Postgres agent, 高并发。

### 模式 3: Multi-tenant 共享 Schema (企业级, 远期)

```rust
// 多个 ZeroClaw 进程共用一个 PG server
// agent_id 全局唯一 (UUID)
// schema = `tenant_<id>`
// 每个 agent 写入同 schema, 用 agent_id 隔离
```

**适用**: 大型部署 (> 100 agent, 多团队)。

## 7. Shadow 推荐: 模式 1 + 模式 2 组合

**当前立即**:
- SQLite agent: 模式 1 (每 agent 独立 .db)
- Markdown agent: 模式 1 (每 agent 独立目录)
- None agent: 模式 1 (no-op)

**接 Postgres 时**:
- 默认: 模式 1 (每 agent 独立 `PostgresMemory::new(...)`)
- 可选: 模式 2 (配置 `pg_pool_max_size > 0` 时启用 pool)
- Pool 默认关闭: 因为大多数部署 1-3 个 Postgres agent 足够

```rust
// PostgresMemory::new (默认路径, 单连接, 抄 ZeroClaw)
pub fn new(alias: &str, db_url: &str, schema: &str, table: &str, ...) -> Result<Self> {
    let client = initialize_client(...)?;  // 一个连接
    Ok(Self { client: Arc::new(Mutex::new(client)), ... })
}

// PostgresMemory::with_pool (可选路径)
pub fn with_pool(alias: &str, pool: PgPool, schema: &str, table: &str, ...) -> Result<Self> {
    Ok(Self { pool: Some(pool), client: None, ... })
}
```

## 8. memory_config 字段扩展

```toml
[memory]
backend = "postgres"   # 或 sqlite / markdown / none

# SQLite 专属
sqlite_path = "~/.shadow/memory/{alias}/brain.db"

# Postgres 专属
db_url = "postgres://user:pass@localhost/zeroclaw"
schema = "public"
table = "memories"

# Postgres Pool (可选, 不配则单连接)
pg_pool_max_size = 16
pg_connect_timeout_secs = 30
pg_idle_timeout_secs = 600

# 向量检索
vector_weight = 0.7
keyword_weight = 0.3
embedding_cache_size = 10000
```

## 9. 多 Agent + Pool 的 Init 流程

```
shadow startup:
1. Config::load_or_init()           ─┐
2. Config::validate()                 │ 数据验证
                                      │
3. memory::init_pg_pool(&config)  ── │ Postgres 池 (如果使用)
                                      │
4. agent_factory: create(default) ──┘
   agent_factory.create("researcher")
   agent_factory.create("coder")
   ...
   
   每个 agent:
     ├─ 读 config.agents[alias].memory.backend
     ├─ 选择 backend:
     │   ├─ None     → NoneMemory::new
     │   ├─ Markdown → MarkdownMemory::new(alias_workspace)
     │   ├─ Sqlite   → SqliteMemory::new(alias_workspace)
     │   └─ Postgres → PgMemory::new(db_url, alias) 或 with_pool
     │
     └─ 包裹: AgentScopedMemory::new(inner, bound_id, allowlist)
```

**Agent 用时**:
- agent.chat(user_msg) → recall() → 走 AgentScoped → recall_for_agents → pool.get() → 执行查询
- agent.store(key, content) → store_with_agent (自动 stamp agent_id) → pool.get() → INSERT

## 10. 锁竞争与死锁分析

### 10.1 单 Mutex 死锁场景

```rust
// ❌ 错误: 同一 Mutex 内递归锁
let guard1 = client.lock();
let guard2 = client.lock();  // parking_lot::Mutex 是非重入, 死锁
```

**Shadow 现状**: SqliteMemory/PostgresMemory 的所有方法都是先 lock, 然后同步操作, 最后 drop guard。**无递归**, 因此死锁风险 = 0。

### 10.2 多 Backend 锁顺序

```rust
// AgentScopedMemory::forget
async fn forget(&self, key: &str) -> Result<bool> {
    let inner_guard = self.inner.lock();     // 锁 A
    drop(inner_guard);
    self.inner.forget_for_agent(key, &self.agent_id).await  // 锁 B (重新加, 没死锁)
}
```

**避免**: 任何方法只加一次锁, 不跨 await 持有锁。

### 10.3 Pool + 业务 Async 死锁

```rust
// ❌ 错误: pool.get().await 在已经持锁的临界区
let pool_guard = some_state.lock();
let client = pool.get().await?;   // 阻塞! 持锁无法让出 runtime

// ✅ 正确: 先 await pool.get(), 再加锁
let client = pool.get().await?;
let rows = client.query(...).await?;
```

## 11. Shadow 接 Postgres 时的 Pool 实现代码示例

### 11.1 crates/shadow-postgres/Cargo.toml

```toml
[dependencies]
shadow-storage.workspace = true
shadow-core.workspace = true
postgres = { version = "0.19", features = ["with-chrono-0_4"] }
deadpool-postgres = "0.14"
tokio.workspace = true
parking_lot = "0.12"
chrono.workspace = true
uuid.workspace = true
anyhow.workspace = true
async-trait.workspace = true

[features]
default = []
memory-postgres = []
memory-pool = ["deadpool-postgres"]
```

### 11.2 crates/shadow-postgres/src/lib.rs

```rust
use deadpool_postgres::{Manager, Pool, PoolConfig as PConfig};
use postgres::NoTls;
use std::time::Duration;

pub struct PgMemory {
    alias: String,
    backend: PgBackend,
    qualified_table: String,
    qualified_agents: String,
}

enum PgBackend {
    Single(DropOnThread<Arc<parking_lot::Mutex<postgres::Client>>>),
    Pooled(Pool),
}

impl PgMemory {
    pub fn new(alias, db_url, schema, table, ...) -> Result<Self> {
        // 单连接模式 (ZeroClaw 兼容)
        let client = initialize_client(...)?;
        Ok(Self { backend: PgBackend::Single(DropOnThread::new(Arc::new(Mutex::new(client)))), ... })
    }

    pub fn with_pool(alias, pool: Pool, schema, table, ...) -> Self {
        // Pool 模式
        Self { backend: PgBackend::Pooled(pool), ... }
    }

    async fn execute<F, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&mut Client) -> Result<T> + Send,
        T: Send,
    {
        match &self.backend {
            PgBackend::Single(client) => {
                // 走 OS 线程 + 单连接 (ZeroClaw 模式)
                let client = client.get().clone();
                run_on_os_thread(move || {
                    let mut c = client.lock();
                    f(&mut c)
                }).await
            }
            PgBackend::Pooled(pool) => {
                // 直接 async pool (新模式)
                let mut client = pool.get().await?;
                f(&mut client)  // 需要 f 是 async 的, 这里示意
            }
        }
    }
}
```

### 11.3 crates/shadow-memory/src/lib.rs 注册

```rust
pub async fn init_pg_pool(config: &MemoryConfig) -> Result<()> {
    if !matches!(backend_kind(config), MemoryBackendKind::Postgres) {
        return Ok(());
    }

    let pg_config = config.db_url.as_ref()
        .context("missing db_url for postgres backend")?
        .parse()?;

    let pool_config = PConfig::new(config.pg_pool_max_size.unwrap_or(16));
    let manager = Manager::from_config(
        pg_config, NoTls, PConfig::default()
    );

    let pool = Pool::builder(manager)
        .max_size(config.pg_pool_max_size.unwrap_or(16))
        .config(pool_config)
        .build()?;

    PG_POOL.set(pool).map_err(|_| anyhow::anyhow!("pool already initialized"))?;
    Ok(())
}
```

## 12. 总结

| 问题 | 答案 |
|------|------|
| 多 Agent 数据库连接池怎么办? | **ZeroClaw 不需要池**: `Arc<Mutex<Client>>` 单连接, 多 agent 各自一个 Memory 实例 |
| 单 Mutex 不会成瓶颈吗? | **不会**: PG 服务器端 1ms+, mutex 自旋 100ns 量级, 1 个连接足够 100 TPS |
| 100 个 PG agent 不会爆连接? | **会**: 100×10MB = 1GB. 解法是**共享 Pool** (模式 2), 让 100 agent 共享 16 连接 |
| 推荐方案? | **模式 1 + 模式 2 组合**: 默认单连接兼容 ZeroClaw, 加可选 pool flag |
| SQLite 也需要池吗? | **不需要**: 本地文件, 每 agent 一个独立连接, 资源占用小 |
| shadow-memory 与 multi-agent 关系? | **Memory trait 已经 Multi-tenant 化**: `agent_id` + `read_memory_from` allowlist, 任意后端都能复用 |

**最重要的设计原则 (从 ZeroClaw 抄)**:
> 能不引入池就不引入池。零 Claw 整个项目始终用 `Arc<Mutex<Client>>` + `run_on_os_thread`, 跑到生产规模都没换。
>
> 除非你接 PG 且 agent 数 > 50, 否则默认单连接够用。

如果你决定接 PG, 抄 ZeroClaw 的 `Arc<Mutex<Client>>` 模式, 后期需要再升级到 deadpool。
