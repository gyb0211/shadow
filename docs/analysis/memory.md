# Shadow 能力分析: Memory 记忆系统

> 对比 ZeroClaw 与 Shadow 的 memory 实现

## 1. Memory Trait 对比

| 项目 | ZeroClaw | Shadow | 差距 |
|------|----------|--------|------|
| trait bound | Send + Sync + Attributable | Attributable | 隐式含 |
| 方法数 | 25 (10必填+15默认) | 30 (更多) | Shadow 更多 |
| store | async store(key, content, category, session_id) | 一致 | 无 |
| recall | async recall(query, limit, session_id, since, until) | 一致 | 无 |
| get / get_for_agent | 有 | 有 | 无 |
| list | 有 | 有 | 无 |
| forget / forget_for_agent | forget_for_agent(key, agent_id) | forget_for_agent(key) 缺 agent_id | 需修复 |
| rename_agent | rename_agent(from, to) | rename_agent(_alias) 缺 _to | 需修复 |
| store_procedural | session_id: Option<&str> | session_id: &str | 类型不同 |
| supersede | 无 | 有 | Shadow 新增 |
| store_with_options | 无 | 有 (StoreOptions) | Shadow 新增 |
| stats / count_in_scope | 无 | 有 | Shadow 新增 |
| refresh_embedder | 无 | 有 | Shadow 新增 |
| MemoryKind | 无 | Episodic/Semantic(Preference/Fact/Decision/Entity)/Procedural | Shadow 新增 |

结论: Shadow trait 设计比 ZeroClaw 更丰富, 但实现严重不足。

## 2. MemoryStrategy Trait 对比

| 方法 | ZeroClaw | Shadow | 差距 |
|------|----------|--------|------|
| load_context | -> Result<String> (格式化文本) | -> Vec<MemoryEntry> (需调用方格式化) | 类型不同 |
| consolidate_turn | (user, assistant, provider, model, temp) | (user, assistant, session_id) | Shadow 缺 LLM 调用 |
| run_governance | 有 | 有 | 无 |

关键差距: ZeroClaw 的 consolidate_turn 接收 ModelProvider, 直接调用 LLM 做语义抽取。
Shadow 版本只存原始对话, 无语义提取能力。

## 3. ZeroClaw 后端实现

| 后端 | 功能 | Shadow 状态 |
|------|------|------------|
| SqliteMemory | FTS5 + 向量BLOB + 混合融合 + 嵌入缓存 (4015行) | 注释 (代码存在) |
| MarkdownMemory | 每条记忆 .md 文件, frontmatter+body | 注释 (代码存在) |
| NoneMemory | 空实现 | 唯一启用 |
| QdrantMemory | Qdrant 向量数据库 | 无 |
| PostgresMemory | PostgreSQL + pgvector | 无 |
| LucidMemory | 包装 SqliteMemory | 无 |
| AgentScopedMemory | Agent 范围限制 | 无 |

## 4. ZeroClaw 记忆分层

| 层级 | 用途 | 存储时机 | 衰减 | 重要性基准 |
|------|------|----------|------|-----------|
| Core | 长期事实/偏好/决策 | consolidate_turn 提取 | 永不衰减 | 0.7 |
| Daily | 对话历史摘要 | consolidate_turn Phase 1 | 7天半衰期 | 0.3 |
| Conversation | 对话上下文 | 会话期间 | 7天半衰期 | 0.2 |
| Custom | 自定义 | 按需 | 7天半衰期 | 0.4 |

最终检索分: hybrid_score*0.7 + importance*0.2 + recency_decay*0.1

## 5. ZeroClaw 检索流程

```
recall(query):
  Stage 1: 热缓存 LRU (TTL=300s, max=256) -> 命中直接返回
  Stage 2: FTS5 关键词检索 (BM25) -> top_score >= 0.85 提前返回
  Stage 3: 向量检索 + hybrid_merge(vector 0.7, keyword 0.3)
  -> apply_time_decay (Core 免疫, 其他 7天半衰期)
  -> 格式化注入 system prompt
```

## 6. ZeroClaw 存储流程

```
consolidate_turn(user, assistant):
  Phase 1: LLM 提取 history_entry (JSON)
    -> store(key="daily_{date}_{uuid}", category=Daily)
  Phase 2: 若 memory_update 非 null
    -> importance::compute_importance
    -> conflict::check_and_resolve_conflicts
    -> store(key="core_{uuid}", category=Core)
  -> run_governance() (后台清理/归档/衰减)
```

## 7. Shadow 差距表

| # | 能力 | 需要做什么 | 优先级 | 为什么 |
|---|------|-----------|--------|--------|
| 1 | 取消注释 sqlite/markdown | 修复 trait 签名后取消注释 | P0 | 当前只有 NoneMemory, 记忆空转 |
| 2 | 修复 forget_for_agent 签名 | 添加 agent_id 参数 | P0 | 无法按 agent 删除 |
| 3 | 修复 rename_agent 签名 | 添加 _to 参数 | P0 | 无法完成重命名 |
| 4 | 修复 store_procedural 类型 | &str -> Option<&str> | P0 | 类型不匹配 |
| 5 | 实现 MemoryStrategy | 为 DefaultMemoryStrategy 实现 trait | P0 | trait 形同虚设 |
| 6 | consolidate_turn 接入 LLM | 添加 provider/model/temp 参数 | P1 | 无语义提取 = 只存原始对话 |
| 7 | load_context 返回 String | 改为 Result<String> | P1 | 统一接口 |
| 8 | RetrievalPipeline | 串联 cache->fts->vector | P1 | 基础设施已就绪但未串联 |
| 9 | importance + decay | 实现评分和衰减模块 | P1 | 字段已定义但无计算逻辑 |
| 10 | 冲突检测 | conflict 模块 | P2 | trait 有 supersede 但无实现 |
| 11 | governance | 清理/归档/衰减 | P2 | trait 有 run_governance 但无实现 |

## Shadow 已有优势 (不需要改)
- Memory trait 比 ZeroClaw 更丰富 (StoreOptions/MemoryKind/MemoryStats)
- embedding/vector 基础设施已移植
- MemoryCategory 枚举已对齐
- ExportFilter 已实现
