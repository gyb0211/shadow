# shadow-memory 差距分析 -- 对照 ZeroClaw

## 当前状态 (Shadow)
- Markdown 后端: 关键词匹配 recall
- None 后端: 空实现
- 共 ~180 行

## ZeroClaw 对应 (zeroclaw-memory: 17442行, 26文件)
- SQLite (4015行, FTS5 全文搜索)
- PostgreSQL (1054行, pgvector)
- Qdrant (1161行, 向量库)
- Lucid (SQLite + 本地嵌入混合)
- Markdown
- None
- AgentScopedMemory: 按 agent_id 隔离
- AuditedMemory: 审计包装器
- Embedding 向量检索: 三级优先级 key 解析
- 记忆合并/去重/衰减: 时间衰减 + 相关性过滤
- 知识图谱 (含 pgvector 变体)
- hygiene 治理: 定期清理/保留/快照
- 自动保存过滤: 防上下文指数膨胀
- MemoryBackendKind 工厂: V3 dotted reference

## 缺失项
| 功能 | 严重度 | ZeroClaw 实现 | Shadow 状态 |
|------|--------|--------------|-------------|
| SQLite 后端 | P0 | FTS5 全文搜索 | 缺失 |
| PostgreSQL | P2 | pgvector | 缺失 (先不实现) |
| 向量检索 | P1 | embedding + Qdrant | 缺失 |
| 记忆衰减 | P1 | 时间衰减 + 相关性 | 缺失 |
| AgentScoped | P1 | 按 agent 隔离 | 缺失 |
| 记忆合并 | P2 | 去重/合并 | 缺失 |
| 知识图谱 | P2 | pgvector 变体 | 缺失 |
| hygiene 治理 | P2 | 定期清理 | 缺失 |
| 自动保存过滤 | P1 | 防膨胀 | 缺失 |

## 开发建议
1. P0: SQLite 后端 (FTS5 全文搜索) -- 本次实现
2. P1: AgentScoped wrapper
3. P1: 记忆衰减 (时间衰减 + score 过滤)
4. P1: 自动保存过滤
5. P1: 向量检索 (embedding)
6. P2: PostgreSQL (先不实现)
7. P2: 知识图谱
