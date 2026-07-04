# Shadow 远程 Agent 协议设计

## 三阶段实现计划

### 阶段1: 进程内 Delegate (~250行)
- 同进程, 不同配置的 agent 互调
- 参考 ZeroClaw delegate.rs sync 模式

### 阶段2: ACP 子进程 (~200行)
- spawn claude/codex/opencode → stdio JSON-RPC
- 参考 Hermes delegate_task acp_command 模式

### 阶段3: A2A 远程 (~300行)
- HTTP JSON-RPC → 远程 Shadow/ZeroClaw
- 参考 ZeroClaw a2a.rs, 实现 Client 端 + Server 端

## 共享: 注册发现层 (~100行)
- Well-known catalog + seed 节点
- 参考 A2A discovery

---
详细设计见各阶段文档
