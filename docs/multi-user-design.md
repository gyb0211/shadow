# Shadow 多用户 / Profile 设计

> 状态: **设计文档** (MVP 不实现, 仅锁定路径与 trait 扩展点)
> 创建: 2026-07-03

## 1. 目标

- **本地运行**: 支持多 profile 切换, 每个 profile 对应一个 git 仓库的一个固定分支 (例 `zhuwenquan`)
- **Pod 运行**: 单 profile 锁定, 一个 Pod 副本 = `{user}-{jiraId}` 临时实例, 跑完即销毁
- **反哺流程**: Pod / 本地 profile 的工作产物经流程 (PR / cherry-pick / 流水线, 细节待定) 回流到对应用户分支
- **形态兼容**: Shadow 可能是 agent (自己跑), 也可能是壳 (转发 Claude Code / Codex); profile 模型对两种形态都成立

## 2. 核心决策

| 决策点 | 选择 | 理由 |
|---|---|---|
| 隔离模型 | per-profile 一个目录 (Hermes 风格) | 简单、向后兼容、形态无关; 不引入 ZeroClaw 的 DB 列 + agent alias 多层 |
| Profile 绑定粒度 | `(git_remote, git_branch)` 二元组固定 | 启动时自动 checkout 固定分支, 反哺流程简单可预测 |
| 锁定机制 | **进程级运行时约束**: 启动时解析 profile, 之后不可切 | 形态无关 (agent 和壳都成立); 不需要 env 强制或 feature flag |
| 注册表存储 | per-profile 分散 (`<profile>/config.yaml`) | 顶层只放 `active_profile` sticky file; 不用集中注册表 |
| Profile Schema | 扁平 (MVP) → 未来分层 (`shadow.d/` + `delegates/`) | YAGNI; 壳模式落地时一次性迁移, 不提前预留空目录 (规避 ZeroClaw `workspace_id` 教训) |
| default profile | `~/.shadow/` 根目录本身 | 零迁移, 现有代码不动 |

## 3. 路径布局

### 3.1 MVP (扁平, agent 模式独占)

```
~/.shadow/                          # default profile = 根目录本身
├── SOUL.md                         # agent 人格 (Hermes 风格)
├── config.yaml                     # git_remote + git_branch + Shadow 配置
├── active_profile                  # sticky file (写当前 profile 名; MVP 只有 "default")
├── memory/brain.db                 # per-profile 独立
├── sessions/                       # per-profile 独立
├── logs/                           # per-profile 独立
├── workspace/                      # git 仓库工作树 (default 在 master/main)
└── profiles/                       # 命名 profile 父目录 (MVP 不创建)
    └── zhuwenquan/                 # 结构与 ~/.shadow/ 完全一致
        ├── SOUL.md
        ├── config.yaml             # git_branch: zhuwenquan
        ├── memory/brain.db
        ├── sessions/
        ├── workspace/              # git worktree on branch zhuwenquan
        └── ...
```

### 3.2 未来: 壳模式落地时迁移到分层

引入 Claude/Codex 壳模式时, profile schema 因形态差距 (Hermes `SOUL.md` ≠ Claude `CLAUDE.md`, memory 归属也不同) 需要分层:

```
~/.shadow/profiles/zhuwenquan/
├── config.yaml          # 公共: git + identity + mode 字段
├── workspace/           # 公共: git worktree (唯一形态无关项)
├── shadow.d/            # ← 原根级文件整体搬这里 (agent 模式数据)
│   ├── SOUL.md
│   ├── memory/
│   └── sessions/
└── delegates/           # 壳模式按工具分
    ├── claude/
    │   ├── CLAUDE.md
    │   └── .claude/
    └── codex/
```

迁移成本可控: 数据量小, 改路径常量 + 一次性迁移脚本. 这是一次性动作, 不反复.

## 4. Profile 锁定语义

```
启动阶段:
  profile_name = CLI(--profile) > env(SHADOW_PROFILE) > active_profile file > "default"
  ProfileHandle::open(name) -> Result<ProfileHandle>

运行时:
  ProfileHandle 持有 &self 不可变引用
  任何切换 API (switch_to / set_active) -> Err(ProfileLocked)
```

**对两种形态一致**:
- agent 模式: `Agent::new(handle)` 时绑定, 后续 `agent.switch_profile()` 报错
- 壳模式: 转发底层工具前, Shadow 已绑定 profile, 底层工具不感知 profile 概念

**Pod 场景**: 启动时 K8s 注入 `SHADOW_PROFILE=zhuwenquan-PROJ-1234` env, 之后进程绑定. 与本地行为完全一致, 不需要二进制差异或额外锁字段.

## 5. 形态差距: agent vs 壳

| 字段 | agent 模式 (MVP) | Claude 壳 (未来) | Codex 壳 (未来) |
|---|---|---|---|
| 人格文件 | `SOUL.md` | `CLAUDE.md` | codex 系统提示 |
| 短期记忆 | `sessions/` | `.claude/` | codex 自管 |
| 长期记忆 | `memory/brain.db` | (Shadow 通过 MCP 提供) | (类似) |
| workspace | `workspace/` | `workspace/` | `workspace/` |
| git 配置 | `config.yaml` | `config.yaml` | `config.yaml` |

**唯一形态无关项**: `workspace/` + `config.yaml` (git + identity). 这也是分层后留在 profile 根级的内容.

## 6. 反哺流程 (待定, 仅留接口)

```
trait SyncBack {
    async fn sync_back(&self, source: &ProfileHandle, target_branch: &str) -> Result<()>;
}

// 实现可插拔: GitPush / GitHubPR / CherryPick / CustomHook
```

具体走 PR / cherry-pick / 外部 CI 由流程定, Shadow 只负责提供:
- source profile 的 workspace 当前 commit 信息
- target profile 的 git_remote / git_branch
- 调用钩子

MVP 不实现, 不创建 trait 占位 (避免 ZeroClaw `workspace_id` 式预留).

## 7. MVP 不做清单

| 不做 | 原因 |
|---|---|
| `profiles/` 子目录创建 | default 够用 |
| CLI `--profile` 解析 | 函数签名预留, 不实现 |
| `ProfileHandle` 切换拒绝 | trait 签名预留, MVP 跑不到 |
| `SyncBack` trait | 留接口文档, 不写代码 |
| per-profile `config.yaml` 字段定义 | 等 profiles/ 落地再定 |
| 跨 profile 软守卫 (Hermes `cross_profile` flag) | MVP 单 profile 不需要 |
| 形态分层 (`shadow.d/` + `delegates/`) | 等壳模式落地 |

## 8. 现有代码的扩展点评估

| 项 | 现状 | 是否堵死多用户路径 |
|---|---|---|
| `shadow_config::config_dir()` | 已集中, 支持 `SHADOW_CONFIG_DIR` env override | ✅ 不堵死 |
| `Memory::new(workspace_dir)` | 接受路径参数 | ✅ 不堵死 |
| `SessionStore` | 基于 workspace_dir | ✅ 不堵死 |
| `shadow_log::init_from_config(workspace_dir)` | 接受路径参数 | ✅ 不堵死 |
| `Skills::load(workspace_dir)` | 接受路径参数, **但 `skills/mod.rs:571` 自定义了 home_dir() 重复扫描 `~/.shadow`** | ⚠️ 需统一到 `config_dir()` |
| Cron / Agent | 基于 workspace_dir | ✅ 不堵死 |

**唯一需修**: `crates/shadow-runtime/src/skills/mod.rs:571` 的 `home_dir()` 替换为 `shadow_config::config_dir()`. 这是当前已有小瑕疵, 不属于本次多用户设计, 但顺手修可避免未来双份路径解析.

## 9. 引用参考

- **Hermes** (`/Users/wan/.hermes/hermes-agent/`): 纯路径隔离模型, `HERMES_HOME` env 注入, 零迁移 default. 本设计主要参考.
- **ZeroClaw** (`/Users/wan/.zeroclaw/src/`): 三层隔离 (auth profile + agent alias + memory agent_id), 复杂度高, 适合多 agent 协作但 Shadow MVP 不需要. 反面教材: `AuthProfile.workspace_id` 字段声明未实现 = 包袱.
