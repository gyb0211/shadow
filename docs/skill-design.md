# Shadow Skills 设计文档

> 参考 ZeroClaw skills/ (12,065行, 21文件) + tools/skill_*.rs + skillforge/

## 一、ZeroClaw Skills 架构全貌

### 1.1 代码规模

```
skills/
├── mod.rs           3826行  核心: Skill struct + SkillTool + 加载/解析/注册
├── service.rs        752行  SkillsService: 统一读写接口
├── document.rs       741行  SKILL.md 解析/序列化 (frontmatter + body)
├── frontmatter.rs    187行  SkillFrontmatter 结构定义
├── cache.rs          586行  技能缓存 (避免重复解析)
├── creator.rs        911行  技能创建 (从模板生成 SKILL.md)
├── improver.rs       730行  技能自改进 (LLM 分析 + 重写)
├── review.rs         433行  技能审查 (质量评分)
├── audit.rs          901行  安全审计 (检查命令/路径/权限)
├── scaffold.rs       250行  脚手架 (创建技能目录结构)
├── reference.rs      162行  SkillRef: 技能引用 (name + bundle 消歧)
├── suggestions.rs    785行  缺失技能建议
├── testing.rs        527行  技能测试框架
├── bundle.rs          42行  技能包
├── constants.rs       22行  常量
├── skill_tool.rs       1行  re-export
├── skill_http.rs       1行  re-export
└── symlink_tests.rs   (测试)

skillforge/                    自动技能发现和集成
├── mod.rs           276行
├── scout.rs         344行  扫描 GitHub 仓库
├── evaluate.rs      272行  评估仓库质量
└── integrate.rs     316行  集成为 SKILL.md

tools/
├── skill_tool.rs   1182行  SkillShellTool: 执行 shell 类型技能工具
├── skill_http.rs         SkillHttpTool: 执行 HTTP 类型技能工具
└── skill_manage.rs       SkillManageTool: 管理技能 (list/install/remove)
```

### 1.2 Skill 数据结构

```rust
struct Skill {
    name: String,
    description: String,
    version: String,
    author: Option<String>,
    tags: Vec<String>,
    tools: Vec<SkillTool>,      // 工具定义
    prompts: Vec<String>,       // 提示文本
    slash_options: Vec<...>,    // Discord slash 命令选项
    location: Option<PathBuf>,  // 文件系统位置
}

struct SkillTool {
    name: String,
    description: String,
    kind: String,               // "shell" | "http" | "script" | "builtin" | "mcp"
    command: String,            // 命令/URL/脚本
    args: HashMap<String, String>,
    target: Option<String>,     // builtin/mcp 委托目标
    locked_args: HashMap<String, String>,  // 锁定参数 (不可被模型覆盖)
    timeout_secs: Option<u64>,
}
```

### 1.3 SKILL.md 格式

```markdown
---
name: git-helper
description: Git 操作辅助技能
version: 0.1.0
author: shadow
tags: [git, vcs]
tools:
  - name: status
    description: 查看 git 状态
    kind: shell
    command: git status
    timeout_secs: 30
  - name: commit
    description: 提交更改
    kind: shell
    command: git commit -m "{message}"
    args:
      message: 提交信息
prompts:
  - "你是一个 git 专家"
---

# Git Helper

这是技能的说明文档, 会作为附加 system prompt 注入...
```

### 1.4 技能来源 (四源合并)

```
1. Workspace: ~/.zeroclaw/workspace/skills/<name>/SKILL.md
2. OpenSkills: 开源技能仓库 (github.com/besoeasy/open-skills)
3. Plugin: WASM 插件携带的技能
4. Bundle: 配置的技能包 [skill_bundles.<alias>]

优先级: Workspace > Bundle > OpenSkills > Plugin
同名技能: 高优先级覆盖低优先级 (ShadowedSkill 记录)
```

### 1.5 技能工具执行流程

```
1. SkillsService.load() → 扫描目录 → 解析 SKILL.md → Vec<Skill>
2. register_skill_tools() → 每个 SkillTool → SkillShellTool/SkillHttpTool → Box<dyn Tool>
3. Agent 工具循环 → LLM 调用 skill__tool_name → execute()
4. SkillShellTool.execute():
   a. 参数替换: {arg_name} → 实际值
   b. 安全环境变量过滤 (只保留 PATH/HOME/TERM 等)
   c. 命令执行 (tokio::process::Command)
   d. 超时控制 (默认 60s)
   e. 输出截断 (最大 1MB)
   f. 工具名验证 (^[a-zA-Z0-9_-]{1,64}$)
```

### 1.6 安全审计 (audit.rs, 901行)

ZeroClaw 在加载技能时做安全审计:
- 命令审计: 检查 shell 命令是否包含危险操作 (rm -rf, mkfs, etc.)
- 路径审计: 检查是否访问工作目录外
- 权限审计: 检查是否需要提权
- 网络审计: 检查 HTTP 工具的目标 URL
- 审计结果: AuditReport, 有 findings 的技能被 drop

### 1.7 SkillForge (自动技能发现)

```
scout → 扫描 GitHub 仓库 (按语言/星数过滤)
evaluate → 评估仓库质量 (README 完整性/许可证/活跃度)
integrate → 生成 SKILL.md + 注册到技能目录
```

### 1.8 技能自改进 (improver.rs, 730行)

```
1. 分析技能使用历史 (成功率/失败原因)
2. LLM 分析: "这个技能哪里可以改进?"
3. 重写 SKILL.md (工具定义/提示文本)
4. 测试改进后的技能
5. 如果通过测试 → 替换原技能
```

## 二、Shadow 当前状态

### 2.1 已有的 (在 main 分支上)

```
crates/shadow-runtime/src/skills/
├── mod.rs           893行  Skill struct + SkillTool + SKILL.md 解析 + 目录加载
└── skill_tool.rs    ~320行  SkillShellTool: 执行 shell 类型技能工具
```

已实现:
- Skill / SkillTool / SkillFrontmatter 结构
- SKILL.md 解析 (自实现 YAML 解析器, 支持 frontmatter + body)
- SkillsService.load() 从 ~/.shadow/skills/ 加载
- SkillsService.all_tools() 转为 Box<dyn Tool>
- SkillShellTool: shell 命令执行 + 参数替换 + 超时 + 输出截断
- 目录加载: load_skills_from_dir() + load_skills()

### 2.2 与 ZeroClaw 的差距

| # | 维度 | ZeroClaw | Shadow | 差距 |
|---|------|----------|--------|------|
| 1 | 工具类型 | shell/http/script/builtin/mcp | shell only | P1: 加 http |
| 2 | 技能来源 | 四源合并 (workspace/open-skills/plugin/bundle) | 单源 (workspace) | P2: 单源够用 |
| 3 | 安全审计 | 901行, 5类审计 | 无 | P1: 加基础命令审计 |
| 4 | 技能缓存 | 586行, 修改时间检测 | 无 | P2: 量小不需要 |
| 5 | 技能创建 | 911行, 从模板生成 | 无 | P2: 手写 SKILL.md |
| 6 | 技能自改进 | 730行, LLM 分析+重写 | 无 | P3: 太复杂 |
| 7 | SkillForge | 1206行, GitHub 自动发现 | 无 | P3: 不需要 |
| 8 | 技能测试 | 527行, 测试框架 | 无 | P2 |
| 9 | 技能审查 | 433行, 质量评分 | 无 | P3 |
| 10 | 缺失建议 | 785行, 建议安装 | 无 | P3 |
| 11 | locked_args | 支持 (锁定参数) | 无 | P2 |
| 12 | slash_options | Discord 集成 | 无 | 不需要 |
| 13 | description_localizations | 多语言 | 无 | 不需要 |
| 14 | Agent 集成 | 自动注册技能工具 | 需要手动调用 all_tools() | P0 |
| 15 | Prompt 注入 | 技能 prompts 注入 system prompt | 无 | P0 |

## 三、Shadow Skills 改进设计

### 3.1 核心原则

Shadow 不需要 ZeroClaw 的全部功能 (12065行)。
聚焦 P0: 让技能系统真正可用 -- 自动加载 + 注册 + prompt 注入。

### 3.2 P0: Agent 集成 (最关键)

当前问题: SkillsService 存在但 Agent 不自动加载技能。
用户需要手动调 `SkillsService::load()` + `all_tools()` + 拼接 prompts。

改进:
```
Agent 构建:
  1. SkillsService::load(workspace) → 加载技能
  2. 技能工具 → ToolRegistry.register()
  3. 技能 prompts → 拼接到 system_prompt
  4. 技能描述 → 拼接到 system_prompt (让 LLM 知道有哪些技能)
```

AgentBuilder 加:
```rust
pub fn skills(&mut self, skills: SkillsService) -> &mut Self;
// 内部: 注册技能工具 + 拼接 prompts
```

### 3.3 P0: 技能 Prompt 注入

ZeroClaw 的做法: 技能的 description + body 注入 system prompt。
让 LLM 知道技能存在, 并在合适时机调用技能工具。

Shadow 设计:
```rust
fn build_skill_prompt(skills: &[Skill]) -> String {
    let mut parts = Vec::new();
    for skill in skills {
        parts.push(format!(
            "## 技能: {}\n{}\n工具: {}",
            skill.name,
            skill.description,
            skill.tools.iter().map(|t| t.name.as_str()).collect::<Vec<_>>().join(", ")
        ));
        for prompt in &skill.prompts {
            parts.push(prompt.clone());
        }
    }
    parts.join("\n\n")
}
```

拼接到 system_prompt 后面。

### 3.4 P1: HTTP 工具类型

SkillTool kind="http" 的执行:
```rust
pub struct SkillHttpTool {
    skill_name: String,
    tool_def: SkillTool,
}

// execute:
// 1. 从 args 构建 URL (替换 {param} 占位符)
// 2. 发 HTTP 请求 (GET/POST)
// 3. 返回响应体
```

### 3.5 P1: 基础安全审计

在 SkillsService::load() 后做简单审计:
```rust
fn audit_skill(skill: &Skill) -> Vec<String> {
    let mut warnings = Vec::new();
    for tool in &skill.tools {
        if tool.kind == "shell" {
            // 检查危险命令
            let cmd = &tool.command;
            if cmd.contains("rm -rf") { warnings.push("危险: rm -rf"); }
            if cmd.contains("mkfs") { warnings.push("危险: mkfs"); }
            if cmd.contains(">/dev/sd") { warnings.push("危险: 写磁盘设备"); }
            if cmd.contains("curl") && cmd.contains("|") && cmd.contains("sh") {
                warnings.push("危险: curl | sh 管道执行");
            }
        }
    }
    warnings
}
```

有 warning 的技能: 记录日志但仍加载 (不 drop, Shadow 信任用户)。

### 3.6 P2: 技能管理工具

让 LLM 通过工具调用管理技能:
```rust
// SkillListTool: 列出已安装技能
// SkillInfoTool: 查看技能详情
// (install/remove 暂不做, 需要网络交互)
```

### 3.7 不做的 (刻意精简)

| 功能 | 原因 |
|------|------|
| 四源合并 | Shadow 单用户, workspace 单源够用 |
| OpenSkills 仓库同步 | 需要网络 + git, 太复杂 |
| WASM 插件技能 | 不需要 |
| SkillForge 自动发现 | 不需要 |
| 技能自改进 | 需要 LLM 调用, 太贵 |
| slash_options | 无 Discord 集成 |
| 技能缓存 | 量小, 直接读文件 |
| locked_args | 暂不需要 |
| 技能测试框架 | 暂不需要 |
| 技能审查评分 | 暂不需要 |

## 四、与 ZeroClaw 的设计对齐度

| 维度 | 对齐 | 说明 |
|------|------|------|
| Skill struct | 部分 | Shadow 简化了 (去掉 version/author/tags/slash_options) |
| SkillTool struct | 部分 | Shadow 简化了 (去掉 locked_args/timeout_secs/target) |
| SKILL.md 格式 | 对齐 | frontmatter + body, 同样的 YAML 结构 |
| SkillShellTool | 对齐 | 命名 {skill}__{tool}, 参数替换, 超时, 输出截断 |
| SkillsService | 对齐 | load/list/find/all_tools |
| 目录结构 | 对齐 | ~/.shadow/skills/<name>/SKILL.md |
| Agent 集成 | 待做 | P0: 自动注册 + prompt 注入 |
| 安全审计 | 待做 | P1: 基础命令审计 |
| HTTP 工具 | 待做 | P1 |
