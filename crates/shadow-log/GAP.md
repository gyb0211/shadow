# shadow-log 差距分析 -- 对照 ZeroClaw

## 当前状态 (Shadow)
- record! 宏 (唯一日志入口) + attribution_span! + scope! 宏
- LogEvent schema (简化 OTel/ECS)
- LogCaptureLayer (tracing Layer) -- span 链 leaf->root 归因合并
- JSONL 持久化 + 滚动裁剪 + 保留策略 (max_files / max_age_days)
- broadcast 广播 (SSE 用)
- observer_bridge (LogEvent -> ObserverEvent 投影, 11 个 action 分支)
- tool_io 捕获 (Off/Redacted/Full 三态 + denylist + 截断)
- reader 分页 (cursor_line_offset + 过滤: action/category/outcome/severity/trace_id/q/field_eq)
- 37 Action (封闭枚举, 无 Other 逃逸)
- ATTRIBUTION_FIELDS (15) + COMPOSITE_PREFIXES (5) 常量表
- __private 模块隔离 tracing
- 11 文件, 2812 行

## ZeroClaw 对应 (zeroclaw-log: 5079 行, 13 文件)
- OTel/ECS 混合 schema (LogEvent)
- 37 Action (封闭枚举, 无 Other 逃逸)
- attribution_span! 宏: 从 Attributable 自动填充归因
- scope! 宏: 自由格式上下文 span
- LogCaptureLayer (762 行): span 遍历 leaf->root 归因合并
- observer_bridge (411 行): LogEvent -> ObserverEvent 投影
- tool_io (208 行): 工具 I/O 捕获 + 泄漏扫描 + 截断
- migrate (332 行): 旧 JSONL 格式迁移
- reader (906 行): 分页读取 + 过滤
- ATTRIBUTION_FIELDS (15) + COMPOSITE_PREFIXES (5) 常量表
- __private 模块隔离 tracing

## 原缺失项进度 (对照初版 GAP.md)

| 功能 | 严重度 | ZeroClaw 实现 | Shadow 状态 |
|------|--------|--------------|-------------|
| attribution_span! | P0 | Attributable -> span 自动归因 | ✅ 已实现 (macro.rs:60 + layer.rs AttributionSpanCollector) |
| scope! 宏 | P1 | 自由格式上下文 span | ⚠️ 已实现但 target 不匹配 (见下方"已知问题") |
| observer_bridge | P1 | LogEvent -> Observer 投影 | ✅ 已实现 (279 行, 11 个 action 分支) |
| tool_io 捕获 | P1 | 工具 I/O + 脱敏 | ✅ 已实现 (86 行 + config.rs ToolIoPolicy) |
| reader 分页 | P2 | 过滤+分页读取 | ✅ 已实现 (198 行, cursor_line_offset) |
| migrate | P2 | 旧格式迁移 | ❌ 仍缺失 |
| 37 Action | P2 | 完整动作词汇 | ✅ 已对齐 (37 个变体) |
| 常量表归因 | P1 | ATTRIBUTION_FIELDS + COMPOSITE | ✅ 已实现 (15 + 5) |

## 仍缺失项

| 功能 | 严重度 | ZeroClaw 实现 | 说明 |
|------|--------|--------------|------|
| migrate | P2 | 332 行, 旧 JSONL 格式迁移 | Shadow 无历史格式需要迁移, 优先级低; 若未来 schema 变更可补 |

## 已知问题 (编译期)

`cargo check -p shadow-log` 报 10 个错误:

### 1. subscriber.rs:39 -- UFCS 调用 trait 方法漏 receiver (E0061)
```rust
// 错误: :: 路径形式调用 trait 方法, 编译器认为需要 (self, filter) 两个参数
.with(LogCaptureLayer::with_filter(recording_filter))
// 修复: 改成方法调用形式
.with(LogCaptureLayer.with_filter(recording_filter))
```

### 2. layer.rs:94,122 -- ScopeSpanCollector 未实现 Visit (E0277)
`ScopeSpanCollector` 在 `on_new_span` / `on_record` 里调 `attrs.record(&mut v)` / `values.record(&mut v)`,
但该结构体没有 `impl Visit for ScopeSpanCollector`。需补 impl 块。

### 3. layer.rs:357,594,599,612 -- write! 无法写入 &mut String (E0599)
`AttributionSpanCollector::record_debug` 用 `write!(&mut buf, ...)` 但未 `use std::fmt::Write`。
String 的 Write impl 在 `std::fmt::Write` trait 下, 需要显式导入。

### 4. layer.rs:401,408,412 -- 不能 move 出 shared ref (E0507)
`ScopeSpanCollector::install` 里 `self.attribution` / `self.extra` 是 `&self` 字段, 不能 move。
需改成 clone 或 borrow (`&self.attribution` 传引用, `self.extra.iter()` 遍历)。

## 已知问题 (逻辑)

### 5. scope! 宏 target 与 layer 不匹配
- macro.rs:51: `target: "log_internal_scope"`
- layer.rs:56: `const SHADOW_SCOPE_SPAN: &str = "log_scope";`
- layer.rs:92 分支 `if target == SHADOW_SCOPE_SPAN` 永远不会命中 scope! 宏发出的 span
- 后果: scope! 宏发出的字段不会被 ScopeSpanCollector 捕获, 等于无效
- 修复: 二选一对齐 (改宏为 "log_scope" 或改常量为 "log_internal_scope")

## 开发建议
1. 修复编译错误 (问题 1-4) -- 阻断编译, 最高优先级
2. 对齐 scope! 宏 target (问题 5) -- 宏当前形同虚设
3. P2: migrate (Shadow 无历史格式, 可延后)
