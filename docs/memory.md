# Shadow Memory 系统设计文档

> 抄自 ZeroClaw，精简适配 Shadow 架构
> 源代码在 `crates/shadow-memory/src/`，共 3703 行，10 个文件

## 1. 整体架构

```
┌─────────────────────────────────────────────────────────────────────┐
│                        create_memory_for_agent                       │
│                         (factory 函数)                               │
└────────────────────────────────┬────────────────────────────────────┘
                                 │
                  ┌──────────────┴──────────────┐
                  │                             │
        Markdown backend            其他 backend
        (无 SQLite 时)            (经 create_memory_with_storage_and_routes)
                  │                             │
                  ▼                             ▼
      ┌──────────────────┐         ┌──────────────────────────┐
      │  MarkdownMemory  │         │ SqliteMemory / LucidMemory│
      │ (own + peers)   │         │ / Postgres / Qdrant ...   │
      │                  │         │ (Box<dyn Memory>)         │
      └────────┬─────────┘         └──────────────┬───────────┘
               │                                  │
               ▼                                  ▼
    ┌─────────────────────┐           ┌──────────────────────────┐
    │AgentScopedMarkdown │           │  AgentScopedMemory       │
    │ Memory (wrap)       │           │  (agent 范围限制 wrap)   │
    └─────────────────────┘           └──────────────────────────┘
```

## 2. 模块结构

| 文件 | 行数 | 职责 |
|------|------|------|
| `lib.rs` | 339 | 模块入口、工厂函数、backend 分类 |
| `agent_scoped.rs` | 416 | Agent 范围限制包装器 (Arc<dyn Memory> + allowed_sibling_agent_ids) |
| `agent_scoped_markdown.rs` | 207 | Markdown 多源包装器 (own + peers 合并) |
| `markdown.rs` | 366 | Markdown 后端 (frontmatter + body .md 文件) |
| `sqlite.rs` | 1752 | SQLite 后端 (FTS5 + 向量混合 + 嵌入缓存) |
| `none.rs` | 111 | 空后端 (no-op) |
| `strategy.rs` | 118 | DefaultMemoryStrategy (load_context / consolidate_turn / run_governance) |
| `embedding.rs` | 229 | EmbeddingProvider trait + Noop/OpenAI 实现 |
| `vector.rs` | 147 | 向量化工具 (cosine_similarity / vec_to_bytes / hybrid_merge) |
| `conflict.rs` | 18 | 冲突检测 (mark_superseded) |

## 3. 公开 API

### 3.1 工厂函数 (`lib.rs`)

| 函数 | 签名 | 说明 |
|------|------|------|
| `create_memory_for_agent` | `async fn(config: &Config, agent_alias: &str, api_key: Option<&str>) -> Result<Arc<dyn Memory>>` | 主入口，根据 agent 配置选 backend，包装 AgentScoped 限制 |
| `create_memory_with_storage_and_routes` | `fn(config: &MemoryConfig, embedding_routes: &[EmbeddingRouteConfig], active_storage: ActiveStorage<'_>, workspace_dir: &Path, api_key: Option<&str>, providers: Option<&ModelProviders>) -> anyhow::Result<Box<dyn Memory>>` | 通用工厂，支持 Sqlite/Lucid/Postgres 等 |
| `resolve_embedding_config` | `fn(config: &MemoryConfig, embedding_routes: &[EmbeddingRouteConfig], api_key: Option<&str>, providers: Option<&ModelProviders>) -> ResolvedEmbeddingConfig` | 解析 embedding 来源 (dotted provider ref 或 self config) |
| `backend_kind_from_dotted` | `fn(memory_backend: &String) -> String` | `"models.sqlite"` → `"sqlite"` |
| `classify_memory_backend` | `fn(backend: &str) -> MemoryBackendKind` | 字符串 → 枚举 (Sqlite/Markdown/None/Qdrant/Postgres/...) |
| `extract_queries` | `fn(message: &str) -> Vec<String>` (strategy) | 从 user message 提取搜索关键词 |
| `format_entries` | `fn(entries: &[MemoryEntry]) -> String` (strategy) | 格式化 memory 为 system prompt 注入 |

### 3.2 后端实现

| Backend | 结构 | `name()` 行为 |
|---------|------|---------------|
| `MarkdownMemory` | frontmatter + body 存 .md | `"markdown"` |
| `NoneMemory` | 全部 no-op | `"none"` |
| `SqliteMemory` | FTS5 + 向量 BLOB + 嵌入缓存 | `"sqlite"` |
| `AgentScopedMarkdownMemory` | 多 MarkdownMemory 读合并，写只 own | own 的 name |
| `AgentScopedMemory` | Arc<dyn Memory> + allowed_sibling_agent_ids | inner.name() |

### 3.3 辅助 trait

| Trait | 方法 | 说明 |
|-------|------|------|
| `EmbeddingProvider` | `name`, `dimensions`, `embed`, `embed_one` (default), `is_noop` (default) | 文本向量化 |
| `NoopEmbedding` | 空实现 | SqliteMemory 退化为纯 FTS5 |
| `OpenAiEmbedding` | 调用 OpenAI 兼容 /v1/embeddings | API 支持 custom base_url |

### 3.4 工具函数 (`vector.rs`)

| 函数 | 签名 | 说明 |
|------|------|------|
| `cosine_similarity` | `fn(a: &[f32], b: &[f32]) -> f32` | 余弦相似度，clamp [0,1] |
| `vec_to_bytes` / `bytes_to_vec` | `fn(&[f32]) -> Vec<u8>` / `fn(&[u8]) -> Vec<f32>` | f32 小端字节序序列化 |
| `hybrid_merge` | `fn(vector_results, keyword_results, vector_weight, keyword_weight) -> Vec<ScoredResult>` | 向量 + 关键词混合融合 |

### 3.5 Conflict (`conflict.rs`)

| 函数 | 签名 | 说明 |
|------|------|------|
| `mark_superseded` | `fn(conn: &Connection, superseded_ids: &[String], new_id: &str) -> anyhow::Result<()>` | UPDATE memories SET superseded_by = ? |

## 4. 工厂函数分发逻辑

`create_memory_for_agent` 根据 `agent_cfg.memory.backend` 分发：

```
Markdown  → AgentScopedMarkdownMemory::new(own=MarkdownMemory::new(alias, agent_workspace), peers=MarkdownMemory list)
None      → NoneMemory::new("none")
其他      → create_memory_with_storage_and_routes → Arc<dyn Memory> → AgentScopedMemory::new(inner, bound_id, allowlist)
```

`create_memory_with_storage_and_routes` 根据 `MemoryBackendKind` 分发：
- `Sqlite` → `SqliteMemory::with_embedder(...)` (带 EmbeddingProvider)
- `Lucid` → 注释中，未实现
- `Postgres` → 注释中，需独立 early-return (type alias config)
- `None` → `NoneMemory`

## 5. AgentScopedMemory 设计 (抄 ZeroClaw)

### 核心目的
每个 agent 持有自己的 per-agent backend 实例（在 agent 创建时通过 `[agents.<alias>.memory.backend]` 选定，之后不可变）。

包装器做 3 件事：
1. **每次 store 通过 `store_with_agent` 自动盖 agent_id 戳** — 后端原始数据可追溯
2. **每次 recall 走 `recall_for_agents(allowed)` 过滤** — 只看自己 + 允许的 sibling
3. **调用方 caller allowlist 与 bound allowlist 求交集** — 防止扩大范围

### 字段
```rust
pub struct AgentScopedMemory {
    inner: Arc<dyn Memory>,
    agent_id: String,                      // 绑定的 agent UUID
    allowed_agent_ids: HashSet<String>,    // 召回白名单 (own + siblings)
}
```

### 关键方法语义

| 方法 | 行为 |
|------|------|
| `name/health_check` | 透传 inner |
| `store` | 内部走 `store_with_agent`，自动 stamp `agent_id` |
| `store_with_metadata` | 透传 + stamp |
| `store_with_agent` | **拒绝外部 agent_id**：如果 caller 传的 agent_id != bound，则 bail "AgentScopedMemory refuses store_with_agent for foreign agent_id" |
| `recall` | 走 `recall_for_agents(&bound_allowlist, ...)` |
| `recall_for_agents(caller_allowlist, ...)` | 交集：caller.allowlist ∩ bound.allowlist，非空才查 inner；如果 caller_allowlist 是空，用 bound |
| `get(key)` | 先查 own，再 fallback 到允许 sibling（按 `get_for_agent` 查） |
| `get_for_agent(key, agent_id)` | 如果 agent_id 不在白名单，返回 None |
| `list` | inner.list + post-filter (按 agent_id ∈ allowlist) |
| `forget(key)` | **只删 own agent 的 row**；如果 inner 返回该 key 命中但 agent_id 不同，bail "AgentScopedMemory refuses to forget cross-agent row" |
| `forget_for_agent(key, agent_id)` | 只允许删 bound agent 的 row（allowlist 给了 recall 但不给 delete）|
| `purge_namespace` | **拒绝**（必须用 admin Memory handle）|
| `purge_session(session_id)` | 调 `inner.purge_session_for_agent(session_id, &self.agent_id)` |
| `purge_session_for_agent(session_id, agent_id)` | 校验 agent_id 在 allowlist 才透传 |
| `purge_agent` / `rename_agent` / `export_agent` / `count_agent` | 拒绝 (admin-only) |
| `count` | inner.list + post-filter (按 allowed) |
| `stats / reindex` | 默认实现或 inner 透传 |
| `recall_namespaced` | recall + 后过滤 namespace |
| `export(filter)` | list + 按 filter 二次过滤 (namespace/since/until) |
| `ensure_agent_uuid` | 透传 inner |

### 安全保证
- 跨 agent 写入：bail
- 跨 agent 删除：bail
- 跨 agent bulk 操作 (purge_namespace/purge_agent/rename_agent)：bail
- recall 通过 allowlist 白名单收敛：bounded visibility
- delete 必须绑定到 own agent：bounded destruction

## 6. AgentScopedMarkdownMemory 设计

不同于 AgentScopedMemory（用 UUID），markdown 包装器用 alias 字符串：

```rust
pub struct MarkdownPeer {
    pub alias: String,                // peer agent 的 alias
    pub memory: MarkdownMemory,       // peer 的 markdown 后端
}

pub struct AgentScopedMarkdownMemory {
    own_alias: String,
    own: MarkdownMemory,
    peers: Vec<MarkdownPeer>,
}
```

读取方法: own + 所有 peers 都查，merge 后 truncate 到 limit。

`store_with_agent(key, content, category, session_id, namespace, importance, agent_id)`：
- 校验 agent_id == own_alias (类似 AgentScopedMemory 的 foreign 拒绝)
- 透传给 own.store_with_agent

## 7. DefaultMemoryStrategy

| 方法 | 行为 |
|------|------|
| `load_context(observer, query, session_id)` | 拆分 query 为多个关键词，每个关键词调 recall，按 key 去重，按 score 排序，截断 5 条 |
| `consolidate_turn(user, assistant, session_id)` | 重要性过滤：assistant < 10 字符或 user < 3 字符跳过；否则生成 key = `turn_<ts>_<session>` 存为 Conversation 类别 |
| `run_governance()` | 占位 no-op (未来: 后台清理/归档/衰减) |

## 8. SQLite 后端设计

`SqliteMemory::with_embedder(name, workspace_dir, embedder, vector_weight, keyword_weight, embedding_cache_size, sqlite_open_timeout, search_mode)`

主要能力：
- **FTS5 全文检索**：trigram tokenize (空串模糊匹配)
- **向量混合**：vector BLOB 列 + cosine_similarity + hybrid_merge
- **嵌入缓存**：LRU cache size configurable
- **schema 迁移**：add_column_if_missing 支持运行时 ALTER TABLE
- **隔离级别**：vector 退化（NoopEmbedding → 纯 FTS5）

WAL 模式，单文件 `{workspace}/brain.db`。

## 9. EmbeddingProvider 抽象

```rust
#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    fn name(&self) -> &str;
    fn dimensions(&self) -> usize;
    async fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>>;
    async fn embed_one(&self, text: &str) -> Result<Vec<f32>> { /* default */ }
    fn is_noop(&self) -> bool { false /* default */ }
}
```

实现：
- `NoopEmbedding`: `dimensions=0`, `embed` 返回空 Vec，`is_noop=true`
- `OpenAiEmbedding`: 调用 `POST /v1/embeddings`，支持 base_url/api_key/model/dim 配置

工厂 `create_embedding_provider(name, api_key, model, dims)`:
- `none` → NoopEmbedding
- `openai/openrouter/custom:<url>` → OpenAiEmbedding

## 10. Memory trait 接口 (shadow_core::Memory)

完整 30+ 方法，关键分组：

**基础 CRUD (必填)**:
- `name() -> &str` — 后端名
- `store(key, content, category, session_id) -> Result<()>` — 存储
- `recall(query, limit, session_id, since, until) -> Result<Vec<MemoryEntry>>` — 检索
- `get(key) -> Result<Option<MemoryEntry>>` — 单条
- `list(category, session_id) -> Result<Vec<MemoryEntry>>` — 列表
- `forget(key) -> Result<bool>` — 删除
- `count() -> Result<usize>` — 总数
- `health_check() -> bool`

**Agent 归属**:
- `get_for_agent(key, agent_id)` (默认实现: get + filter)
- `forget_for_agent(key, agent_id)` (必填)
- `store_with_agent(key, content, category, session_id, namespace, importance, agent_id)` (必填)
- `store_with_metadata(key, content, category, session_id, namespace, importance)` (默认: store)
- `recall_for_agents(caller_allowed, query, limit, session_id, since, until)` (必填)
- `ensure_agent_uuid(alias) -> Result<String>` (默认: 返回 alias)

**会话隔离**:
- `purge_session(session_id)` (默认: bail)
- `purge_session_for_agent(session_id, agent_id)` (默认: bail)

**Agent 范围操作 (admin-only)**:
- `purge_namespace(namespace)` / `purge_agent(alias)` (默认: bail)
- `export_agent(alias)` / `rename_agent(from, to)` / `count_agent(alias)` (部分默认实现)

**元数据/recall**:
- `recall_namespaced(namespace, query, limit, session_id, since, until)` (默认: recall 后过滤 namespace)
- `export(filter)` (默认: list + 后过滤)
- `supersede(superseded_ids, new_id)` (默认: no-op)
- `store_procedural(messages, session_id)` (默认: no-op)

**存储/性能**:
- `reindex()` / `refresh_embedder(...)` / `stats()` / `count_in_scope(...)` (默认实现)

## 11. 使用流程 (Agent 主循环)

```
1. 创建 Agent 时:
   - 读 config.agents[alias].memory.backend
   - 调 create_memory_for_agent(config, alias, api_key)
   - 返回 Arc<dyn Memory> (包了 AgentScoped* 后端)

2. 对话前 (consolidate_turn 或 before_chat):
   - 构造 DefaultMemoryStrategy(memory.clone())
   - 调 strategy.load_context(observer, user_msg, session_id) -> Vec<MemoryEntry>
   - 格式化为 system prompt 注入

3. 对话后 (consolidate_turn):
   - strategy.consolidate_turn(user_msg, assistant_msg, session_id)

4. 用户主动工具调用 (memory_*):
   - tool 调 memory.store(key, content, category, session_id)
   - tool 调 memory.recall(query, limit, session_id, since, until)
   - tool 调 memory.forget(key)
   - tool 调 memory.list(category, session_id)

5. Agent 销毁:
   - Drop Agent
   - 内部 memory 引用计数 → 0 → drop
   - AgentScopedMemory / AgentScopedMarkdownMemory Drop
```

## 12. 配置示例

```toml
[agents.clamps]
memory.backend = "markdown"   # 或 "sqlite" / "none"

# 可选: embedding 配置
[memory]
backend = "sqlite"
vector_weight = 0.7
keyword_weight = 0.3
embedding_cache_size = 10000

# 可选: embedding route
[[embedding_routes]]
hint = "default"
model_provider = "custom:https://api.example.com/v1"
model = "text-embedding-3-small"
dimensions = 1536

# 可选: 跨 agent read allowlist
[agents.clamps.workspace]
read_memory_from = ["shared-facts"]   # 从其他 agent 的 markdown 读取
```

## 13. 与 ZeroClaw 对比

| 维度 | ZeroClaw | Shadow | 差距 |
|------|----------|--------|------|
| 后端 | 6 个 (Sqlite/Markdown/None/Qdrant/Postgres/Lucid/AgentScoped*) | 5 个 (Sqlite/Markdown/None + AgentScoped*) | 缺 Qdrant/Postgres/Lucid |
| Sqlite 行数 | 4015 | 1752 | 56% (省了部分高级特性) |
| MemoryStrategy | 3方法 (load_context→Result<String>, consolidate_turn 接收 ModelProvider) | 3方法 (load_context→Vec, consolidate_turn 仅 session_id) | 缺 LLM 语义抽取 |
| AgentScopedMemory | 完整实现 (15行 Trait + 大量方法) | 完整抄 (418行) | 0 |
| agent_scoped.rs 测试 | 12 个集成测试 | 0 个 | 需补充 |
| Hygiene / Governance | 完整 (过期清理 / 衰减 / 冲突检测) | 仅 mark_superseded | 需补充 |
| 检索缓存 LRU | 完整 (HotCache + FTS + Vector) | 简单 (无 HotCache) | 需补充 |
| StreamChunk 集成 | 是 | 无 | 无需 |

## 14. 已实现 vs 未实现

### ✅ 已实现

| 能力 | 状态 |
|------|------|
| Memory trait 完整 30+ 方法 | ✅ |
| NoneMemory / MarkdownMemory / SqliteMemory | ✅ |
| AgentScopedMemory (完整 Memory trait) | ✅ (抄 ZeroClaw) |
| AgentScopedMarkdownMemory (own + peers) | ✅ |
| DefaultMemoryStrategy (load_context + consolidate_turn) | ✅ |
| EmbeddingProvider trait + Noop/OpenAI | ✅ |
| 工厂 create_memory_for_agent | ✅ |
| Conflict: mark_superseded | ✅ |
| Vector: cosine_similarity / hybrid_merge | ✅ |
| ToolKind::Sqlite/SopList/...Shadow独有工具类型 | ✅ |
| shadow-core exports (MemoryKind/MemoryStats/ProceduralMessage/...) | ✅ |

### ⏳ WIP / 缺失

| 能力 | 优先级 | 说明 |
|------|--------|------|
| LucidMemory | P3 | 包装 SqliteMemory 加额外特性 |
| PostgresMemory | P3 | PostgreSQL + pgvector |
| QdrantMemory | P3 | Qdrant 向量数据库 |
| Hygiene | P2 | 记忆清理/归档机制 |
| Governance | P3 | run_governance 实际清理逻辑 |
| Importance 评分 | P2 | Compute by category + keywords |
| Decay 衰减 | P2 | 时间衰减 (Core 免疫 / 7 天半衰期) |
| LLM 集成 consolidate_turn | P3 | 接 ModelProvider 做语义抽取 |
| AgentScopedMemory 集成测试 | P2 | 12 个测试抄 ZeroClaw |
| KnowledgeGraph | P3 | PG 知识图谱 |
| ResponseCache | P3 | 响应缓存 |
| Snapshot | P3 | 快照 |
| Chunker | P3 | 文本分块 |
| 自动 hydration | P2 | 启动从 memory 召回到 system prompt |

## 15. 下一步建议

P1 (1-2 周):
1. AgentScopedMemory 补 12 个集成测试 (从 ZeroClaw 测试抄)
2. Importance + Decay 模块 (影响 recall 排序)
3. consolidate_turn 接 ModelProvider (LLM 抽取长期事实)

P2 (1-2 月):
1. Hygiene: 过期清理 / 归档
2. HotCache: recall LRU 层 (减少磁盘 IO)
3. Recovery: 启动从 SQLite hydrate 历史到 memory (类似 session 的 hydration)

P3 (远期):
1. Lucid / Postgres / Qdrant 后端
2. KnowledgeGraph (PG)
3. ResponseCache
4. Snapshot
5. MCP / 跨后端
