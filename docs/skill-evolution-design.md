# Skill 自进化反馈循环 -- 三系统对比研究

> 研究 ZeroClaw + Hermes + Claude 的 skill 自进化机制

## 一、Hermes Agent 的 Skill 自进化

### 1.1 核心理念

Hermes 的核心卖点是 "Self-improving through skills" -- 通过技能自进化。

> Hermes learns from experience by saving reusable procedures as skills.
> When it solves a complex problem, discovers a workflow, or gets corrected,
> it can persist that knowledge as a skill document that loads into future sessions.

### 1.2 Skill 生命周期

```
用户对话 → Agent 解决问题 / 被纠正 / 发现新技巧
  ↓
后台 review fork (对话结束后触发)
  ↓
LLM 分析对话: "有什么值得保存的?"
  ↓
决策: PATCH 现有技能 / CREATE 新技能 / DO NOTHING
  ↓
通过 skill_manage 工具写入 SKILL.md
  ↓
下次对话 → skill 自动加载 → 更好的表现
```

### 1.3 Hermes 的实现 (Python)

从 hermes-agent 源码分析:

**触发条件:**
- 对话积累 N 轮工具调用后 (nudge_interval_iterations)
- 不是每次对话都触发, 有冷却时间

**Review Fork:**
- 对话结束后, 启动一个受限的 Agent 循环
- 给 review Agent 有限的工具: skill_manage, skill_view, skills_list
- 递归保护: review 内部不触发新的 review (task_local)

**Review Prompt 核心逻辑:**
```
你是后台技能审查 Agent。看刚才的对话, 决定是否应该改变技能库。

信号:
- 用户纠正了你的风格/格式/步骤 → 更新技能
- 发现了新技巧/修复/绕过 → 捕获到技能
- 技能被调用但失败了/过时了 → 修复技能

不要保存:
- 环境相关错误 (缺包/路径不对)
- 对工具的负面断言 ("X 不能用")
- 一次性任务

优先级:
1. PATCH 当前调用的技能
2. PATCH 已有的类级技能 (加子节/加陷阱)
3. ADD 支持文件 (references/templates/scripts)
4. CREATE 新的类级技能 (最后手段)
```

### 1.4 Skill 格式 (agentskills.io 标准)

```markdown
---
name: debugging
description: 系统化调试方法
version: "0.2.0"
updated_at: "2026-07-01T12:00:00Z"
improvement_reason: "添加了 Cargo 缓存清理步骤"
---

# Debugging

## 步骤
1. 重现问题
2. ...

## 陷阱
- 不要在 Docker 容器内调试网络问题
- ...

<!-- Improvement: 2026-07-01 | Reason: 添加了 Cargo 缓存清理步骤 -->
<!-- Improvement: 2026-06-28 | Reason: 初始创建 -->
```

### 1.5 关键设计

- **冷却时间**: 同一技能不会频繁修改 (内存 + 磁盘双冷却)
- **原子写入**: 临时文件 → 验证 → rename (防写入中途崩溃)
- **审计跟踪**: 每次改进追加 HTML 注释 `<!-- Improvement: timestamp | Reason -->`
- **front-matter 元数据**: `updated_at` + `improvement_reason` 持久化在 SKILL.md
- **类级技能**: 技能名是 "debugging" 不是 "fix-bug-on-tuesday" (泛化)
- **不保存负面断言**: "X 不能用" 会变成自我约束, 环境变了就错了

## 二、ZeroClaw 的 Skill 自进化

### 2.1 架构 (12,065行, 21文件)

```
skills/
├── improver.rs     (730行) -- SkillImprover: 原子写入 + 冷却 + 审计跟踪
├── review.rs       (433行) -- 后台 review fork (抄 Hermes 设计)
├── review_prompt.md         -- review Agent 的系统提示
├── creator.rs      (911行) -- 从模板创建新技能
├── audit.rs        (901行) -- 安全审计 (命令/路径/权限)
├── cache.rs        (586行) -- 技能缓存 (修改时间检测)
├── suggestions.rs  (785行) -- 缺失技能建议
├── testing.rs      (527行) -- 技能测试框架
└── skillforge/              -- 自动技能发现
    ├── scout.rs    (344行) -- 扫描 GitHub 仓库
    ├── evaluate.rs (272行) -- 评估仓库质量
    └── integrate.rs(316行) -- 集成为 SKILL.md
```

### 2.2 与 Hermes 的关系

ZeroClaw review.rs 的注释明确写了:
```
// Inspired by hermes-agent's `_spawn_background_review` pattern
// (see nousresearch/hermes-agent at run_agent.py).
```

ZeroClaw 的改进:
- Rust async: review fork 内联 await (不需要后台线程)
- 直接操作 agentskills.io 格式 (SKILL.md)
- 通过 skill_manage 工具写入 (不是直接文件操作)
- 原子写入 + 验证 + rename

### 2.3 SkillImprover 详细流程

```
1. should_improve_skill(slug)
   ├─ config.enabled?
   ├─ 内存冷却: Instant 比较 cooldown_secs
   └─ 磁盘冷却: SKILL.md front-matter 的 updated_at 比较

2. improve_skill(slug, improved_content, reason)
   ├─ validate_skill_content(improved_content)  -- 必须有 front-matter + name
   ├─ 读取现有文件, 提取审计跟踪
   ├─ append_improvement_metadata() -- 更新 front-matter (updated_at + reason)
   ├─ 拼接: 新内容 + 旧审计跟踪 + 新审计条目
   ├─ 写临时文件 .SKILL.md.tmp
   ├─ 验证临时文件可读且合法
   └─ rename → SKILL.md (原子替换)
   └─ 更新内存冷却
```

### 2.4 审计跟踪格式

```markdown
<!-- Improvement: 2026-07-01T12:00:00Z | Reason: 添加了 Cargo 缓存清理步骤 -->
<!-- Improvement: 2026-06-28T10:00:00Z | Reason: 初始创建, 从 hermes 迁移 -->
```

每次改进追加一行, 永不删除, 形成完整的改进历史。

### 2.5 SkillForge (自动技能发现)

ZeroClaw 超越 Hermes 的地方 -- 不只是从对话中学习, 还能从 GitHub 仓库发现技能:

```
scout → 扫描 GitHub (按语言/星数过滤)
  ↓
evaluate → 评估: README 完整性 / 许可证 / 活跃度 / 星数
  ↓
integrate → 生成 SKILL.md + 注册到技能目录
```

## 三、Claude 的 Skill 机制

### 3.1 Claude Code 的 Skills

从 Hermes skill 列表中的 claude-code 技能描述:
- Claude Code 使用 SKILL.md 格式 (和 Hermes/ZeroClaw 一致)
- 技能存储在 `.claude/skills/` 目录
- 有 skill-creator 子代理 (grader/comparator/analyzer) 做技能质量评估

### 3.2 Claude 的 skill-creator

```
.claude/skills/skill-creator/
├── agents/
│   ├── grader.md       -- 评分技能质量
│   ├── comparator.md   -- 比较新旧版本
│   └── analyzer.md     -- 分析改进方向
├── references/
│   └── schemas.md      -- 技能 schema 定义
└── SKILL.md
```

这是一个元技能 -- 创建和改进其他技能的技能。

### 3.3 Claude 的改进循环

```
1. 用户使用技能 → 结果不理想
2. skill-creator 激活:
   a. analyzer 分析: 哪里出了问题?
   b. grader 评分: 当前技能质量如何?
   c. 生成改进版
   d. comparator 比较: 新版比旧版好吗?
3. 如果新版更好 → 替换
```

和 Hermes/ZeroClaw 不同: Claude 用多个专门 Agent (grader/comparator/analyzer) 而非单个 review Agent。

## 四、三系统对比

| 维度 | Hermes | ZeroClaw | Claude |
|------|--------|----------|--------|
| **触发** | N 轮工具调用后 | N 轮 + 冷却 | 用户手动 / 自动 |
| **执行者** | 单个 review Agent | 单个 review Agent | 多 Agent (grader/comparator/analyzer) |
| **写入方式** | skill_manage 工具 | skill_manage 工具 | 文件操作 |
| **原子性** | Python 文件写入 | 临时文件 → rename | 文件操作 |
| **冷却** | 有 | 内存 + 磁盘双冷却 | 不确定 |
| **审计跟踪** | 无 (Hermes) | HTML 注释历史 | 不确定 |
| **安全审计** | 无 | 901行, 5类检查 | grader 评分 |
| **自动发现** | 无 | SkillForge (GitHub) | 无 |
| **技能测试** | 无 | 527行测试框架 | comparator 比较 |
| **格式** | SKILL.md | SKILL.md + SKILL.toml | SKILL.md |
| **递归保护** | task_local | task_local | 不确定 |
| **类级 vs 实例** | 类级 | 类级 | 类级 |
| **负面断言过滤** | review prompt | review prompt | 不确定 |

## 五、Shadow 的设计方向

### 5.1 当前状态

Shadow 有:
- SkillsService (加载/列表/查找)
- SkillShellTool (shell 工具执行)
- SkillHttpTool (HTTP 工具执行)
- SKILL.md 解析 (自实现 YAML 解析器)
- 安全审计 (audit_skill, 基础命令检查)

Shadow 没有:
- 自进化 (无 review fork)
- SkillImprover (无原子写入/冷却/审计)
- 技能测试
- SkillForge

### 5.2 建议的演进路线

```
Phase 1: 基础自进化 (抄 Hermes/ZeroClaw)
  - SkillImprover: 原子写入 + 冷却 + 审计跟踪
  - review_prompt.md: review Agent 系统提示
  - maybe_run_skill_review(): 对话结束后触发
  - 递归保护: task_local
  - 技能管理工具: SkillListTool, SkillViewTool, SkillManageTool

Phase 2: 安全增强
  - 技能安全审计 (命令/路径/权限)
  - review 前检查改进内容的合法性
  - 不保存负面断言 (prompt 过滤)

Phase 3: 自动发现 (如有需求)
  - SkillForge: GitHub 仓库扫描
  - 评估 + 集成
```

### 5.3 关键设计决策

1. **用单个 review Agent** (抄 Hermes/ZeroClaw) -- 比 Claude 的多 Agent 更简单
2. **原子写入** (抄 ZeroClaw) -- 临时文件 → 验证 → rename
3. **双冷却** (抄 ZeroClaw) -- 内存 + 磁盘 (updated_at)
4. **审计跟踪** (抄 ZeroClaw) -- HTML 注释历史
5. **类级技能** (三系统一致) -- 不保存一次性任务
6. **不保存负面断言** (抄 Hermes/ZeroClaw) -- 防止自我约束
7. **递归保护** (抄 Hermes/ZeroClaw) -- task_local
8. **不做 SkillForge** -- 太复杂, 后续按需

### 5.4 不做的

| 功能 | 原因 |
|------|------|
| SkillForge (GitHub 发现) | 太复杂, 需要网络 + git |
| 多 Agent review (Claude 风格) | 单 Agent 够用, 简单 |
| SKILL.toml 格式 | SKILL.md 够用 |
| 技能缓存 | 量小, 直接读文件 |
| 技能测试框架 | 后续按需 |
| 技能质量评分 | 后续按需 |
