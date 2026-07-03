# Shadow Memory 执行计划

> 基于 memory-design.md, 分 5 步执行

## Step 1: 重构 Memory Trait + MemoryEntry (shadow-core)

文件: crates/shadow-core/src/memory.rs

- [ ] MemoryEntry: timestamp 改 String (RFC 3339), 加 score 字段
- [ ] MemoryCategory 枚举: Core / Daily / Conversation / Custom(String)
- [ ] Memory trait 新签名:
  - store(key, content, category, session_id) 分参数
  - recall(query, limit, session_id) 加 session 过滤
  - list(category) 加 category 过滤
  - forget(key) -> bool
  - count() -> usize
  - health_check() -> bool
  - name() -> &str
- [ ] NoneMemory 适配新 trait
- [ ] 导出 MemoryCategory

## Step 2: 重构 Markdown 后端 (shadow-memory)

文件: crates/shadow-memory/src/markdown.rs

- [ ] store(): 用新签名, 正确写 frontmatter (id, key, category, timestamp, session_id)
- [ ] get(): 正确解析 frontmatter, 还原 MemoryEntry 全部字段
- [ ] list(): 正确解析 frontmatter
- [ ] recall(): 加 session_id 过滤
- [ ] forget() -> bool
- [ ] count() / health_check() / name()
- [ ] 按 session_id 子目录隔离 (session/{id}/key.md)

## Step 3: 重构 SQLite 后端 (shadow-memory)

文件: crates/shadow-memory/src/sqlite.rs

- [ ] store(): 用新签名 (key, content, category, session_id 分参数)
- [ ] recall(): 加 session_id WHERE 条件
- [ ] list(): 加 category WHERE 条件
- [ ] forget() -> bool
- [ ] count() / health_check() / name()
- [ ] category 列存储枚举字符串 ("core" / "daily" / "conversation" / custom)
- [ ] timestamp 列改 TEXT (RFC 3339)

## Step 4: MemoryStrategy + Agent 集成 (shadow-runtime)

文件: crates/shadow-runtime/src/agent.rs + crates/shadow-memory/src/strategy.rs

- [ ] MemoryStrategy trait: before_chat() + after_chat()
- [ ] DefaultMemoryStrategy 实现:
  - before_chat: 从 user_message 提取关键词 → memory.recall() → 格式化为 context
  - after_chat: 简单存储 user+assistant 摘要 (不做 LLM 摘要, 太贵)
- [ ] Agent.chat_with_stream():
  - 调 before_chat() 获取记忆上下文, 拼接到 system prompt
  - 对话完成后调 after_chat() 存储记忆
- [ ] Agent 加 memory_strategy: Option<Arc<dyn MemoryStrategy>> 字段

## Step 5: 测试 + 工厂函数

- [ ] create_memory() 适配新 trait
- [ ] Markdown 单元测试: store → get → recall → forget 往返
- [ ] SQLite 单元测试: store → recall with session → count → forget
- [ ] MemoryStrategy 测试: before_chat 返回非空 (store 后 recall)
- [ ] 编译: cargo build --features tui
- [ ] 测试: cargo test --workspace -- --skip cron::tests::update_next_run
