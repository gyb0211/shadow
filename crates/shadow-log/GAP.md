# shadow-log 差距分析 -- 对照 ZeroClaw

## 当前状态 (Shadow)
- record! 宏 (唯一日志入口)
- LogEvent schema (简化 OTel)
- LogCaptureLayer (tracing Layer)
- JSONL 持久化 + 滚动裁剪
- broadcast 广播 (SSE 用)
- 12 Action (封闭枚举)
- 共 ~650 行

## ZeroClaw 对应 (zeroclaw-log: 5079行, 13文件)
- OTel/ECS 混合 schema (LogEvent)
- 37 Action (封闭枚举, 无 Other 逃逸)
- attribution_span! 宏: 从 Attributable 自动填充归因
- scope! 宏: 自由格式上下文 span
- LogCaptureLayer (762行): span 遍历 leaf->root 归因合并
- observer_bridge (411行): LogEvent -> ObserverEvent 投影
- tool_io (208行): 工具 I/O 捕获 + 泄漏扫描 + 截断
- migrate (332行): 旧 JSONL 格式迁移
- reader (906行): 分页读取 + 过滤
- ATTRIBUTION_FIELDS (15) + COMPOSITE_PREFIXES (5) 常量表
- __private 模块隔离 tracing

## 缺失项
| 功能 | 严重度 | ZeroClaw 实现 | Shadow 状态 |
|------|--------|--------------|-------------|
| attribution_span! | P0 | Attributable -> span 自动归因 | 缺失 |
| scope! 宏 | P1 | 自由格式上下文 span | 缺失 |
| observer_bridge | P1 | LogEvent -> Observer 投影 | 缺失 |
| tool_io 捕获 | P1 | 工具 I/O + 脱敏 | 缺失 |
| reader 分页 | P2 | 过滤+分页读取 | 缺失 |
| migrate | P2 | 旧格式迁移 | 缺失 |
| 37 Action | P2 | 完整动作词汇 | 12 Action |
| 常量表归因 | P1 | ATTRIBUTION_FIELDS + COMPOSITE | 缺失 |

## 开发建议
1. P0: attribution_span! 宏 (自动归因, 零成本)
2. P1: observer_bridge (复用 log 事件给 Observer)
3. P1: tool_io 捕获 + 脱敏
4. P1: scope! 宏
5. P2: reader 分页
6. P2: 扩展 Action 枚举
