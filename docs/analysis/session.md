# Shadow 能力分析: Session 会话持久化

> ZeroClaw 24方法/双后端 vs Shadow 6方法/JSONL单后端

## 1. SessionStore Trait 对比

| 方法 | ZeroClaw (SessionBackend) | Shadow (SessionStore) | 差距 |
|------|--------------------------|----------------------|------|
| load | sync fn load(key) -> Vec<ChatMessage> | async fn load(id) -> Option<Session> | 签名不同 |
| append | sync fn append(key, msg) -> Result | async fn append_message(id, msg) -> Result | OK |
| save | 无 (用append) | async fn save(session) -> Result | Shadow独有 |
| delete | sync fn delete_session(key) -> Result<bool> | async fn delete(id) -> Result | OK |
| list | sync fn list_sessions() -> Vec<String> | async fn list() -> Vec<String> | OK |
| list_with_metadata | sync fn -> Vec<SessionMetadata> | async fn -> Vec<SessionMetadata> | OK |
| load_with_timestamps | sync fn -> Vec<TimestampedMessage> | 无 | 缺 |
| remove_last | sync fn -> Result<bool> | 无 | 缺 |
| update_last | sync fn -> Result<bool> | 无 | 缺 |
| search | sync fn(SessionQuery) -> Vec<SessionMetadata> | 无 | 缺 |
| get_session_metadata | sync fn -> Option<SessionMetadata> | 无 | 缺 |
| session_exists | sync fn -> bool | 无 | 缺 |
| set/get_session_name | 2个方法 | 无 | 缺 |
| set/get_session_agent_alias | 2个方法 | 无 | 缺 |
| clear_agent_attribution | sync fn -> usize | 无 | 缺 |
| rename_agent_attribution | sync fn -> usize | 无 | 缺 |
| count_agent_attribution | sync fn -> usize | 无 | 缺 |
| set_session_context | sync fn(SessionContext) | 无 | 缺 |
| set/get_session_state | 2个方法 | 无 | 缺 |
| list_running_sessions | sync fn -> Vec<SessionMetadata> | 无 | 缺 |
| list_stuck_sessions | sync fn(threshold) -> Vec<SessionMetadata> | 无 | 缺 |
| compact | sync fn -> Result | 无 | 缺 |
| cleanup_stale | sync fn(ttl_hours) -> usize | 无 | 缺 |
| clear_messages | sync fn -> usize | 无 | 缺 |
| 总计 | 24方法 (10必填+14默认) | 6方法 (全必填) | 缺18方法 |

## 2. Session 元数据字段对比

| 字段 | ZeroClaw | Shadow | 差距 |
|------|----------|--------|------|
| id/key | key: String | id: String | 命名不同 |
| name | name: Option<String> | title: Option<String> | 命名不同 |
| created_at | DateTime<Utc> | Option<String> (RFC3339) | 类型不同 |
| last_activity | DateTime<Utc> | updated_at: Option<String> | 命名不同 |
| message_count | usize | usize | OK |
| agent_alias | Option<String> | Option<String> | OK |
| channel_id | Option<String> (<type>.<alias>) | 缺 | 缺(渠道路由) |
| room_id | Option<String> | 缺 | 缺(房间标识) |
| sender_id | Option<String> | 缺 | 缺(发送者标识) |
| state | String ("idle"/"running"/"error") | 缺 | 缺(运行状态) |
| turn_id | Option<String> | 缺 | 缺(turn追踪) |
| turn_started_at | Option<DateTime<Utc>> | 缺 | 缺(超时检测) |

## 3. 持久化后端对比

| 特性 | ZeroClaw | Shadow | 差距 |
|------|----------|--------|------|
| 后端1 | JSONL (兼容/回退) | JSONL (唯一) | - |
| 后端2 | SQLite/WAL/FTS5 (默认) | 无 | 缺 |
| trait抽象 | SessionBackend trait | SessionStore trait | Shadow已有trait |
| 迁移 | migrate_from_jsonl | 无 | 缺 |
| 文件布局 | sessions/{key}.jsonl | sessions/{id}.jsonl + {id}.meta.json | Shadow有sidecar |
| 元数据存储 | session_metadata表(SQLite) | .meta.json sidecar | 设计不同 |
| FTS5搜索 | sessions_fts虚拟表+触发器 | 无 | 缺 |

## 4. Session 与 Memory 关系对比

| 特性 | ZeroClaw | Shadow | 差距 |
|------|----------|--------|------|
| session_id参数 | Memory::store/recall都有session_id | Memory::store/recall有session_id | OK |
| MemoryEntry.session_id | 有 | 有 | OK |
| session_id生成 | sanitize_session_key(channel_scope+sender) | 无统一生成 | 缺 |
| CLI的session_id | sanitize("cli:{path}") | 无 | 缺 |
| 群聊memory隔离 | [sender_id, history_key]双scope | 无 | 缺 |
| purge_session | Memory::purge_session(session_id) | 有trait方法 | OK |

## 5. 多渠道隔离对比

| 特性 | ZeroClaw | Shadow | 差距 |
|------|----------|--------|------|
| session key生成 | conversation_history_key(msg): channel_scope+reply_target+thread_ts, sanitized | 无 | 缺 |
| 结构化路由列 | channel_id/room_id/sender_id | 无 | 缺 |
| owner映射 | owner_by_channel_key | 无 | 缺 |
| LRU+持久双层 | 内存LRU(1000)+磁盘 | 无LRU | 缺 |
| 先磁盘后内存 | 是 | 无 | 缺 |

## 6. 启动恢复对比

| 步骤 | ZeroClaw | Shadow | 差距 |
|------|----------|--------|------|
| 1.列出所有session | list_sessions_with_metadata | list (按mtime) | Shadow有简化版 |
| 2.按活跃度排序 | sort by last_activity | sort by mtime | 类似 |
| 3.路由到owning agent | channel_id->agent反查 | 无 | 缺 |
| 4.截断历史 | MAX_CHANNEL_HISTORY(50) | 无 | 缺 |
| 5.闭合中断session | 追加"[interrupted]" | 无 | 缺 |
| 6.清理orphan | remove_orphaned_tool_messages | 无 | 缺 |
| 7.LRU填充 | push到conversation_histories | 无 | 缺 |

## 7. 保存时机对比

| 时机 | ZeroClaw | Shadow (注释代码) | 差距 |
|------|----------|-----------------|------|
| 渠道: user消息 | store.append(key, user_msg) | 无渠道 | 缺 |
| 渠道: assistant消息 | store.append(key, assistant_msg) | 无渠道 | 缺 |
| 流式: 增量更新 | update_last (SQLite单次UPDATE) | 无 | 缺 |
| CLI: turn完成后 | save_interactive_session_history | store.append_message | 有注释代码 |
| /new /clear | delete_session + clear内存 | 无 | 缺 |
| 不保存 | tool中间结果不持久化 | 同 | OK |

## 8. Session 命令对比

| 命令 | ZeroClaw | Shadow | 差距 |
|------|----------|--------|------|
| /new | delete_session + clear内存 + pending_new | 无 | 缺 |
| /clear | 同/new | 无 | 缺 |
| /sessions | sessions_list工具 | 无 | 缺 |
| sessions_current | 工具: 当前session key+metadata | 无 | 缺 |
| sessions_history | 工具: 读指定session最近N条 | 无 | 缺 |
| sessions_send | 工具: 跨agent追加消息 | 无 | 缺 |
| sessions_reset | 工具: 清空消息(保留session) | 无 | 缺 |
| sessions_delete | 工具: 永久删除 | 无 | 缺 |
| 权限控制 | SessionOwnershipScope | 无 | 缺 |

## 9. 差距表

| # | 能力 | 需要做什么 | 优先级 | 为什么 |
|---|------|-----------|--------|--------|
| 1 | Session->Memory关联 | 确保session_id贯穿memory系统(已有trait参数,需实际使用) | P0 | per-conversation记忆隔离 |
| 2 | session_exists | 添加方法 | P1 | 避免重复创建 |
| 3 | search | 添加FTS5搜索方法 | P1 | TUI需要搜索历史 |
| 4 | channel_id/room_id/sender_id | 添加元数据字段 | P1 | 多渠道前提 |
| 5 | update_last | 添加方法(default: remove_last+append) | P1 | 流式响应增量持久化 |
| 6 | orphan修复 | 添加remove_orphaned_tool_messages | P1 | 崩溃恢复后修复半残session |
| 7 | 运行状态 | state/turn_id/turn_started_at + 4方法 | P2 | stuck检测+dashboard |
| 8 | agent归属级联 | clear/rename/count_agent_attribution | P2 | agent删除/重命名时自动跟随 |
| 9 | SQLite后端 | 实现SqliteSessionStore+FTS5+迁移 | P2 | 大量session时性能 |
| 10 | LRU+持久双层 | 内存LRU cache | P2 | 减少磁盘IO |
| 11 | cleanup_stale | TTL清理方法 | P2 | 防止无限增长 |
| 12 | session命令 | /new /clear /sessions + sessions_*工具 | P2 | 用户需要管理session |
| 13 | 启动hydration | 批量load+路由+orphan修复+LRU填充 | P3 | 重启后上下文恢复 |
| 14 | persist lock | per-sender Mutex | P3 | 并发写入序列化 |

## Shadow 已有优势
- append_message vs save 语义分离 (ZeroClaw只有append+update_last)
- .meta.json sidecar 设计干净, 向后兼容
- current_session_id() 已实现
- SessionStore trait 已继承 Attributable (Role::Session)
- Session/SessionMetadata 结构体已有 agent_alias 字段
