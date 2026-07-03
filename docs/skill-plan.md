# Shadow Skills 执行计划

> 基于 skill-design.md, 分 3 步执行, 不写代码 (本次只规划)

## 当前状态

Shadow 已有 skills 系统 (mod.rs 893行 + skill_tool.rs 320行):
- Skill / SkillTool / SkillFrontmatter 结构
- SKILL.md 解析 (自实现 YAML 解析器)
- SkillsService.load() 从 ~/.shadow/skills/ 加载
- SkillShellTool: shell 命令执行 + 参数替换 + 超时
- SkillsService.all_tools() 转为 Box<dyn Tool>

问题: Agent 不自动加载技能, 需要手动调用。Prompt 不注入。

## Step 1: Agent 集成 (P0)

文件: crates/shadow-runtime/src/agent.rs + crates/shadow-tui/src/lib.rs + src/main.rs

- [ ] AgentBuilder 加 .skills(SkillsService) 方法
- [ ] Agent 构建: 技能工具注册到 ToolRegistry
- [ ] Agent 构建: 技能 prompts + description 拼接到 system_prompt
- [ ] build_skill_prompt() 函数: 生成技能上下文文本
- [ ] TUI build_agent(): 加载技能 + 注入 Agent
- [ ] CLI chat_via_agent(): 加载技能 + 注入 Agent
- [ ] 测试: 有技能时 system_prompt 包含技能描述

## Step 2: HTTP 工具 + 安全审计 (P1)

文件: crates/shadow-runtime/src/skills/skill_http.rs (新建) + mod.rs

- [ ] SkillHttpTool: 执行 HTTP 类型技能工具
  - URL 参数替换 ({param})
  - GET/POST 请求
  - 响应截断 (1MB)
  - 超时 (30s)
- [ ] SkillsService.all_tools() 注册 http 类型
- [ ] audit_skill() 函数: 基础命令审计
  - 检查 rm -rf / mkfs / curl|sh 等危险命令
  - 有 warning 记录日志, 不 drop
- [ ] 测试: HTTP 工具执行
- [ ] 测试: 审计检测危险命令

## Step 3: 技能管理工具 (P2)

文件: crates/shadow-runtime/src/tools/skill_list.rs (新建) + skill_info.rs (新建)

- [ ] SkillListTool: LLM 可调用, 列出已安装技能
- [ ] SkillInfoTool: LLM 可调用, 查看技能详情 (工具列表 + prompts)
- [ ] 注册到 default_tools()
- [ ] 测试

## 后续 (不在本次范围)

- 技能创建 (从模板生成 SKILL.md)
- 技能测试框架
- 技能缓存 (修改时间检测)
- 技能自改进 (LLM 分析 + 重写)
- SkillForge (GitHub 自动发现)
- OpenSkills 仓库同步
- locked_args (锁定参数)
- 多源合并 (workspace + bundle)
