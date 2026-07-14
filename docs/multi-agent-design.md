content = """# Shadow 多 Agent 设计文档

> 现状分析 + ZeroClaw 多 agent 经验 + Shadow 落地路径

## 1. Shadow 现状 (基于 main 分支)

Shadow 的 multi_agent 框架已经**部分就绪**, 数据结构层齐全, 运行时层缺关键组件。

### 1.1 已实现 (在 `crates/shadow-config/src/multi/`)

| 类型 | 字段 | 状态 |
|------|------|------|
| `AliasedAgentConfig` | enabled, workspace, memory, model_provider, risk_profile, runtime_profile | ✅ 有 |
| `AgentWorkspaceConfig` | path, access, unrestricted_filesystem, read_memory_from | ✅ 有 |
| `AgentMemoryConfig` | backend | ✅ 有 |
| `MemoryBackendKind` | None / Sqlite / Postgres / Markdown / Lucid / Unknown | ✅ 有 (比 ZeroClaw 少 Qdrant + Lucid 占位) |
| `AccessMode` | Read / Write / ReadWrite | ✅ 有 |

### 1.2 关键缺失 (按 ZeroClaw 对照)

| 能力 | ZeroClaw | Shadow | 差距 |
|------|----------|--------|------|
| `[peer_groups.<name>]` | ✅ 联合成员 + mutual membership | ❌ 完全缺失 | 整块缺 |
| `[peer_groups.<name>].external_peers` | ✅ 外部用户名 | ❌ | 整块缺 |
| `[peer_groups.<name>].ignore` | ✅ 黑名单 | ❌ | 整块缺 |
| `[peer_groups.<name>].output_modality` | Mirror/Voice/Text | ❌ | 整块缺 |
| `ResolvedPeers` 解析器 | ✅ `resolve_peer_set(config, agent_alias)` | ❌ | 整块缺 |
| `SubAgentSpawn` (运行时子 agent) | ✅ 458 行, 11 种 EscalationViolation | ❌ | 整块缺 |
| `SubAgentOverrides` (policy 收敛) | ✅ `ensure_no_escalation_beyond` | ❌ | 整块缺 |
| `SubAgentContext` | ✅ parent_alias + policy + memory allowlist | ❌ | 整块缺 |
| `[a2a.server]` (跨 agent 发现) | ✅ A2aServerConfig (HTTPS well-known route) | ❌ | 整块缺 |
| `EscalationViolation` (11 种) | ✅ policy.rs 枚举 | ❌ | 整块缺 |
| `send_message_to_peer` tool | ✅ LLM 主动触发 | ❌ | 整块缺 |
| Memory 后端 validator | ✅ `Config::validate()` 检查 `read_memory_from` 同后端 | ❌ | 缺校验 |

## 2. ZeroClaw 多 Agent 设计理念

### 2.1 三个层次

```
┌───────────────────────────────────────────────────────────┐
│ Level 1: 静态多 Agent (config 定义)                         │
│ → [agents.<alias>]: 多个 agent 各有独立配置                   │
│ → [peer_groups.<name>]: 联合成员机制                        │
└───────────────────────────────────────────────────────────┘
                          ↓
┌───────────────────────────────────────────────────────────┐
│ Level 2: 运行时多 Agent (内存代理)                          │
│ → spawn_subagent 工具: LLM 派生子任务                       │
│ → SubAgentSpawn 验证: 权限必须 ≤ parent                    │
│ → 确保 audit trail 可追溯                                  │
└───────────────────────────────────────────────────────────┘
                          ↓
┌───────────────────────────────────────────────────────────┐
│ Level 3: 跨 Agent 通信 (A2A)                                 │
│ → [a2a.server] 暴露 A2A discovery endpoint                 │
│ → /a2a/agents/<alias> 卡描述                                 │
│ → external peers 寻址                                      │
└───────────────────────────────────────────────────────────┘
```

### 2.2 数据结构 (摘自 `multi_agent.rs`)

```rust
define_provider_ref!(AgentAlias, "agents");
define_provider_ref!(PeerGroupName, "peer_groups");
define_provider_ref!(PeerUsername, "channels.peers");

pub enum AccessMode { Read, Write, ReadWrite }   // 跨 agent filesystem 权限

pub enum MemoryBackendKind {                     // 关闭集合, schema is law
    None, Sqlite, Postgres, Qdrant, Markdown, Lucid,
}

pub struct AgentWorkspaceConfig {
    pub path: Option<PathBuf>,                   // None = derive 路径
    pub access: BTreeMap<AgentAlias, AccessMode>, // 跨 agent 文件系统
    pub unrestricted_filesystem: bool,            // escape hatch
    pub read_memory_from: Vec<AgentAlias>,         // 跨 agent 记忆读
}

pub struct AgentMemoryConfig {
    pub backend: MemoryBackendKind,                // agent 创建时锁定
}

pub struct PeerGroupConfig {
    pub channel: ChannelRef,                       // "telegram" 或 "telegram.work"
    pub agents: Vec<AgentAlias>,                   // 成员
    pub external_peers: Vec<PeerUsername>,         // 非 agent 用户
    pub ignore: Vec<PeerUsername>,                 // 黑名单
    pub output_modality: OutputModality,           // 输出偏好
}

pub struct A2aServerConfig {
    pub enabled: bool,
    pub bind: Option<String>, pub port: Option<u16>,
    pub public_base_url: String,                   // 公开代理后 URL
}
```

### 2.3 关键设计原则 (摘自代码注释)

**不变性 (immutability)**:
```
An agent's backend is locked at agent creation and immutable on subsequent loads.
Config::validate() enforces immutability against the persisted on-disk state.
```

**校验时机**:
- `Config::validate()` 启动时检查 cross-backend allowed list 一致性
- 启动时检查每个 `read_memory_from` entry 真实存在
- **跨后端不允许**: SQLite agent 不能 recall postgres agent 的 memory

**默认安全**:
```
A missing entry in `access` means no cross-agent access at all (jailed).
The enum only encodes the granted modes; absence is the safe default.
```

## 3. ZeroClaw 解析器 `peers.rs` (关键运行时逻辑)

```rust
pub struct ResolvedPeers {
    pub agent_peers: BTreeMap<String, BTreeSet<String>>,     // channel → agent aliases
    pub external_peers: BTreeMap<String, BTreeSet<String>>,   // channel → usernames
}

impl ResolvedPeers {
    pub fn is_known_peer(&self, channel_type: &str, target: &str) -> bool;
    pub fn allows_inbound(&self, channel_type: &str, origin: &str) -> bool;
}

pub fn resolve_peer_set(config: &Config, agent_alias: &str) -> ResolvedPeers {
    let mut resolved = ResolvedPeers::default();
    for group in config.peer_groups.values() {
        let on_group = group.agents.iter().any(|a| a.as_str() == agent_alias);
        if !on_group { continue; }

        let channel = group.channel.to_string();

        // 1. mutual membership (排除自己)
        let self_norm = agent_alias.trim_start_matches('@').to_ascii_lowercase();
        for member in &group.agents {
            let normalized = member.as_str().trim_start_matches('@').to_ascii_lowercase();
            if normalized != self_norm {
                resolved.agent_peers.entry(channel.clone()).or_default().insert(normalized);
            }
        }

        // 2. external peers
        for ext in &group.external_peers {
            resolved.external_peers.entry(channel.clone()).or_default()
                .insert(ext.as_str().trim_start_matches('@').to_ascii_lowercase());
        }

        // 3. ignore list (subtract)
        for ignored in &group.ignore {
            let needle = ignored.as_str().trim_start_matches('@').to_ascii_lowercase();
            resolved.external_peers.get_mut(&channel).map(|s| s.remove(&needle));
            resolved.agent_peers.get_mut(&channel).map(|s| s.remove(&needle));
        }
    }
    resolved
}
```

**关键设计**:
- **大小写不敏感 + 去 `@` 前缀** — 防止渠道 API 不同前缀
- **mutual membership**: 只 mutual peer 是可见的
- **self-exclusion**: 自己永远不在自己的 peer set
- **ignore 是减法**, 不是 peer set 的一部分

## 4. SubAgentSpawn 设计 (458 行, ZeroClaw 关键机制)

```rust
pub struct SubAgentOverrides {
    pub policy: Option<SecurityPolicy>,
    pub allowed_agent_aliases: Option<HashSet<String>>,
}

pub struct SubAgentContext {
    pub parent_alias: String,
    pub policy: Arc<SecurityPolicy>,
    pub allowed_agent_aliases: HashSet<String>,
    pub run_id: String,
    pub trace_id: String,
}

pub struct SubAgentSpawn {
    parent_alias: String,
    parent_policy: Arc<SecurityPolicy>,
    parent_allowed: HashSet<String>,    // alias set
    overrides: SubAgentOverrides,
}

impl SubAgentSpawn {
    pub fn for_agent(config: &Config, agent_alias: &str) -> Result<Self>;
    pub fn with_overrides(self, overrides: SubAgentOverrides) -> Result<Self>;
    pub fn build(self) -> Result<SubAgentContext>;
}
```

### 4.1 权限收敛验证 (11 种 EscalationViolation)

`SecurityPolicy::ensure_no_escalation_beyond(parent, child)` 检查 11 种升级:

```rust
pub enum EscalationViolation {
    AutonomyAboveParent,                    // autonomy 升级
    ReadWriteRootNotInParent,               // RW 路径不在父白名单
    ReadOnlyRootNotInParent,                // RO 路径不在父白名单
    WriteOnlyRootNotInParent,               // WO 路径不在父白名单
    CommandNotInParent,                     // 命令不在父白名单
    WorkspaceOnlyDisabledByChild,           // workspace_only 被关
    ForbiddenPathDroppedByChild,            // forbidden_paths 被去掉
    ShellEnvPassthroughExpanded,            // shell env passthrough 扩大
    MaxActionsExceeded,                     // max_actions 缩小 (但 child 反过来)
    MaxCostExceeded,                         // max_cost 缩小
    ShellTimeoutExceeded,                    // shell timeout 缩小
    BlockHighRiskCommandsDisabledByChild,    // block_high_risk 被关
    RequireApprovalDisabledByChild,          // require_approval 被关
}
```

**Ordinal 排序**:
```rust
pub enum AutonomyLevel {
    ReadOnly,    // = 0
    Supervised,  // = 1
    Full,        // = 2
}
// 派生: PartialOrd + Ord, 子 agent autonomy 必须 <= parent
```

### 4.2 SubAgent memory allowlist 收敛

```rust
// SubAgentOverrides 里的 allowed_agent_aliases 必须 ⊆ parent.allowed
fn validate_memory_allowlist(
    parent: &HashSet<String>,
    requested: Option<&HashSet<String>>,
) -> Result<HashSet<String>> {
    match requested {
        Some(req) => {
            let extra: Vec<_> = req.difference(parent).collect();
            if !extra.is_empty() {
                anyhow::bail!("SubAgent memory allowlist exceeds parent: {extra:?}");
            }
            Ok(req.clone())
        }
        None => Ok(parent.clone()),  // inherit
    }
}
```

**Alias vs UUID**: SubAgentSpawn 持有 alias (config 层), consumer (`create_memory_for_agent`) 负责把 alias 解析为后端 UUID (SQLite) 或保留 alias (markdown/qdrant)。

### 4.3 tracing 集成

```rust
SubAgentContext::trace_id format: 
"agent.<parent_alias>.subagent.<run_id>"
```

- run_id 是 UUID v4 (与 agent_id 不同)
- 提交后写入 `agent.<alias>.subagent.<run_id>` span
- 日志归因跨 run 可追溯

## 5. Shadow 复刻路线 (3 个 PR)

### PR 1: 数据结构层 (`crates/shadow-config/src/multi/peer_group.rs`)

直接抄 ZeroClaw, 加:

```rust
pub mod peer_group;  // 新增

// mirror multi_agent.rs 的 peer_group 子模块
define_provider_ref!(PeerGroupName, "peer_groups");
define_provider_ref!(PeerUsername, "channels.peers");

pub enum OutputModality { Mirror, Voice, Text }

pub struct PeerGroupConfig {
    pub channel: ChannelRef,
    pub agents: Vec<AgentAlias>,
    pub external_peers: Vec<PeerUsername>,
    pub ignore: Vec<PeerUsername>,
    pub output_modality: OutputModality,
}

// 加到 shadow-config/src/lib.rs
pub use peer_group::{PeerGroupConfig, PeerGroupName, PeerUsername, OutputModality};

// 加到 Config::agents: BTreeMap<String, PeerGroupConfig>
```

**改动量**: ~150 行新代码 + 已有 schema 集成

### PR 2: 解析器 (`crates/shadow-runtime/src/peers.rs`)

抄 ZeroClaw 的解析器:

```rust
pub struct ResolvedPeers {
    pub agent_peers: BTreeMap<String, BTreeSet<String>>,
    pub external_peers: BTreeMap<String, BTreeSet<String>>,
}

pub fn resolve_peer_set(config: &Config, agent_alias: &str) -> ResolvedPeers {
    // 抄 ZeroClaw 的全部 logic
}
```

集成到 channel orchestrator:
```rust
// orchestrator 收到 message 时:
let peers = shadow_runtime::peers::resolve_peer_set(&config, &self_handle);
if !peers.allows_inbound(&channel_type, &origin) { return; }
```

### PR 3: SubAgentSpawn (`crates/shadow-runtime/src/subagent/`)

这是最大块, 抄 ZeroClaw 458 行 subagent/mod.rs:

```rust
pub struct SubAgentOverrides { ... }
pub struct SubAgentContext { ... }
pub struct SubAgentSpawn { ... }

pub fn for_agent(config: &Config, agent_alias: &str) -> Result<Self>;
pub fn with_overrides(self, overrides: SubAgentOverrides) -> Result<Self>;
pub fn build(self) -> Result<SubAgentContext>;
```

接入 `spawn_subagent` tool 和 cron 的 `JobType::Agent`, 调用 builder。

同时补 EscalationViolation:

```rust
// crates/shadow-runtime/src/security/escalation.rs
pub enum EscalationViolation { ... 11 种 ... }

impl SecurityPolicy {
    pub fn ensure_no_escalation_beyond(&self, parent: &Self) -> Result<(), EscalationViolation>;
}
```

## 6. A2A (Agent-to-Agent) Discovery

### 6.1 ZeroClaw 设计

```toml
[a2a.server]
enabled = true
bind = "0.0.0.0"          # 可选, 默认 gateway host
port = 8443              # 可选, 默认 gateway port
public_base_url = ""     # 可选, 公开代理 URL

[agents.clamps.a2a]
published = true         # 是否出现在 discovery catalog
exposed_skills = ["github-pr-workflow"]  # 卡上显示哪些 skill
```

A2A 是一个标准协议 (Google), ZeroClaw 在 gateway 暴露 `/.well-known/agents.json`, 包含每个 `published=true` 的 agent 卡片 (含 name/skills/url)。

### 6.2 Shadow 缺什么

| 组件 | 状态 |
|------|------|
| `[a2a.server]` config | ❌ |
| `[agents.<alias>.a2a]` config | ❌ |
| A2aServerSection (默认关闭) | ❌ |
| AgentA2aConfig (per-alias 开关) | ❌ |
| shadow-gateway crate (HTTPS, well-known routes) | ❌ (空壳) |
| 卡片 JSON 序列化 | ❌ |

### 6.3 A2A 工作量评估

| 子任务 | 行数估 | 说明 |
|------|--------|------|
| A2aServerConfig + AgentA2aConfig | ~100 | 抄 struct |
| shadow-gateway 加 a2a 模块 | ~300 | axum routes + 卡片序列化 |
| exposure_skills 过滤 | ~50 | 从 SkillsService::resolve_effective_skills 提取 |
| 集成到 orchestrator | ~100 | 注册 a2a routes |

A2A 总体 ~600 行, 工作量大, 但都是 axum + serde 模板化代码。

## 7. 内存多 Agent (关键)

### 7.1 Shadow 当前 AgentScoped 能力

```rust
// Shadow agent_scoped.rs (抄 ZeroClaw):
pub struct AgentScopedMemory {
    inner: Arc<dyn Memory>,
    agent_id: String,
    allowed_agent_ids: HashSet<String>,
}
```

✅ 已完整。`read_memory_from` allowlist 通过后端 UUID 解析后传入:

```rust
// shadow-memory/src/lib.rs:
let inner = create_memory_with_storage_and_routes(...)?;   // Sqlite
let inner_arc: Arc<dyn Memory> = Arc::from(inner);

let bound_id = inner_arc.ensure_agent_uuid(agent_alias).await?;
let mut allowlist_ids = Vec::new();
for peer in &agent_cfg.workspace.read_memory_from {
    let uuid = inner_arc.ensure_agent_uuid(peer.as_str()).await?;
    allowlist_ids.push(uuid);
}
let scoped = AgentScopedMemory::new(inner_arc, bound_id, allowlist_ids);
```

✅ Shadow 已做。缺的是 **Config::validate() 校验**:
- 每个 `read_memory_from.alias` 在 `agents` 表存在
- **同后端**: 不能让 SQLite agent recall postgres agent (混合 UUID)

```rust
// shadow-config/src/validation.rs (新增)
impl Config {
    pub fn validate(&self) -> Result<(), ValidationError> {
        for (alias, agent) in &self.agents {
            // 1. own backend 合法
            if !valid_backends.contains(&agent.memory.backend) {
                bail!("agents.{alias}: unknown backend");
            }

            // 2. read_memory_from 同后端
            for peer_alias in &agent.workspace.read_memory_from {
                let peer = self.agents.get(peer_alias.as_str()).ok_or(...)?;
                if peer.memory.backend != agent.memory.backend {
                    bail!("agents.{alias}.read_memory_from.{peer_alias}: cross-backend disallowed");
                }
            }
        }
        Ok(())
    }
}
```

### 7.2 跨 Agent recall

```rust
// shadow-memory/src/agent_scoped.rs 已实现:
async fn recall(&self, ...) -> Result<Vec<MemoryEntry>> {
    let allowed = self.allowed_slice();
    self.inner
        .recall_for_agents(&allowed, query, limit, session_id, since, until)
        .await
}
```

`recall_for_agents` 限定 UUID, 防止越界。✅ 已实现。

## 8. 工作量估计

| PR | 内容 | 行数估 | 时间估 |
|----|------|--------|--------|
| PR 1 | peer_group config | ~150 + 200 集成 | 1 天 |
| PR 2 | peers.rs 解析器 | ~200 | 1 天 |
| PR 3 | SubAgentSpawn + EscalationViolation | ~700 | 3 天 |
| PR 4 | Config::validate 校验 | ~150 | 1 天 |
| PR 5 | spawn_subagent tool | ~150 | 1 天 |
| PR 6 | A2A discovery (可选) | ~600 | 3 天 |

总计 (不含 A2A): ~1350 行 + 7 天
总计 (含 A2A): ~1950 行 + 10 天

## 9. 优先级建议

| 优先级 | 任务 | 原因 |
|--------|------|------|
| **P0** | Config::validate 校验 read_memory_from 同后端 | 数据完整性, 缺会崩溃 |
| **P0** | peer_group config + 解析器 | Shadow 当前多 agent 配置不可用 |
| **P1** | SubAgentSpawn | 启用 spawn_subagent 工具, agent loop 闭环 |
| **P1** | EscalationViolation | SubAgent 安全收敛, 不做会越权 |
| **P2** | cron 集成 spawn_subagent | 时间触发子任务 |
| **P3** | A2A discovery | 跨进程 agent 发现, 远期 |

## 10. 总结

**Shadow 多 agent 现状**: 数据结构层✅ (AgentWorkspaceConfig/AgentMemoryConfig/AccessMode/read_memory_from), 运行时层❌ (解析器/SubAgent/A2A 缺)。

**ZeroClaw 经验**:
1. **immutability**: agent 创建后 backend 锁定
2. **deny-by-default**: access/allowlist 缺失=默认拒绝
3. **mutual membership**: peer group 自动 mutual peer
4. **escalation validation**: 11 种 Violation, SubAgent 必须 ≤ parent
5. **UUID vs alias 分层**: SubAgent 持有 alias, consumer 解析

**落地路径**: 数据结构 → 解析器 → SubAgent (4 PR, ~7 天)。A2A 是远期远景。
"""
</content>
