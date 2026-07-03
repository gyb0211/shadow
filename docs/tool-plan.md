# Shadow Tool 执行计划

> 基于 tool-design.md, 分 4 步执行

## Step 1: ToolRegistry + 工具注册表 (shadow-runtime)

文件: crates/shadow-runtime/src/tools/registry.rs (新建)

- [ ] ToolRegistry struct: Vec<Box<dyn Tool>> + find/specs/execute
- [ ] register() / unregister() 动态注册
- [ ] default_tools() 返回 ToolRegistry 而非 Vec
- [ ] Agent 从 Vec<Box<dyn Tool>> 改为 ToolRegistry
- [ ] 编译: cargo build --features tui

## Step 2: 补齐基础工具 (shadow-runtime)

文件: crates/shadow-runtime/src/tools/

- [ ] memory_recall.rs: MemoryRecallTool (调 memory.recall)
- [ ] memory_store.rs: MemoryStoreTool (调 memory.store)
- [ ] glob_search.rs: GlobSearchTool (文件名搜索)
- [ ] content_search.rs: ContentSearchTool (文件内容搜索)
- [ ] 注册到 default_tools()
- [ ] 每个 tool 有单元测试

## Step 3: 并行执行 + 凭证脱敏 (shadow-runtime)

文件: crates/shadow-runtime/src/agent.rs

- [ ] 工具循环: 多 tool_calls 用 join_all 并行
- [ ] requires_approval 的工具仍串行 (先检查)
- [ ] scrub_credentials() 函数: 脱敏 API key/token
- [ ] observer 输出前脱敏

## Step 4: 装饰器模式 (shadow-runtime)

文件: crates/shadow-runtime/src/tools/wrapper.rs (新建)

- [ ] ToolWrapper trait
- [ ] RateLimitedTool: 速率限制
- [ ] PathGuardedTool: 路径安全检查
- [ ] default_tools() 用装饰器包装
- [ ] 编译 + 测试: cargo test --workspace -- --skip cron::tests::update_next_run
