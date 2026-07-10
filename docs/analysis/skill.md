# Shadow 能力分析: Skill 技能系统

> ZeroClaw vs Hermes vs Shadow 三方对比

## 1. 设计理念对比

| 维度 | ZeroClaw | Hermes | Shadow |
|------|----------|--------|--------|
| 核心定位 | 知识注入 + 可执行工具定义 | 纯文本知识注入 | 知识注入 + 工具定义(未完成) |
| Skill是trait吗 | 不是, 是Tool的动态来源 | 不是, 是知识文本 | 不是, SkillTool是数据结构 |
| Skill是Tool吗 | SkillTool实现Tool trait | 不是 | SkillTool未实现Tool trait |
| 注入方式 | system prompt注入description | system prompt注入name+description | parse_skill_md解析 |
| 加载触发 | agent启动时 | agent启动时 + skill_view按需 | agent启动时(未接入) |

## 2. SKILL.md 格式对比

### frontmatter 字段

| 字段 | ZeroClaw | Hermes | Shadow |
|------|----------|--------|--------|
| name | String (必填) | String (必填) | String |
| description | String (必填) | String (必填) | String |
| version | Option<String> | String | 无 |
| author | Option<String> | String | 无 |
| license | Option<String> | String | 无 |
| category | Option<String> | 无 | 无 |
| tags | Vec<String> | metadata.hermes.tags | 无 |
| slash_options | Vec<SkillSlashOption> | 无 | 无 |
| metadata | 无 | metadata.hermes (tags/related_skills/dependencies) | 无 |
| tools | 无(frontmatter无,在body解析) | 无 | Vec<SkillTool> |
| prompts | 无 | 无 | Vec<String> |

### body 内容

| 维度 | ZeroClaw | Hermes | Shadow |
|------|----------|--------|--------|
| 格式 | Markdown 操作指南 | Markdown 操作指南 | Markdown |
| 作用 | 注入system prompt作为指南 | 注入system prompt作为指南 | 注入prompts |
| 附属文件 | references/ templates/ scripts/ | references/ templates/ scripts/ assets/ | 无 |

## 3. Skill 数据结构对比

### ZeroClaw

| 结构 | 字段 | 说明 |
|------|------|------|
| SkillFrontmatter | name, description, license, author, version, category, tags, slash_options | 完整frontmatter解析 |
| SkillDocument | frontmatter + body | SKILL.md完整表示 |
| EffectiveSkill | name, description, origin, directory, editable, bundle, shadowed | 运行时effective视图 |
| EffectiveSkillSet | skills + dropped | 解析结果(含审计丢弃) |
| SkillOrigin | Workspace/OpenSkills/Plugin/Bundle | 四源来源标记 |
| SkillRef | (name, bundle) | 技能引用(去歧义) |
| SkillSummary | ref, directory, frontmatter | 列表视图 |
| SkillSlashOption | name, description, type, required, choices, min, max | Discord slash command参数 |

### Hermes

| 结构 | 说明 |
|------|------|
| (无结构体) | skill是纯文本, 直接读SKILL.md内容注入 |
| .bundled_manifest | skill名->hash映射, 检测变更 |

### Shadow

| 结构 | 字段 | 说明 |
|------|------|------|
| Skill | name, description, tools(Vec<SkillTool>), prompts(Vec<String>) | 技能主体 |
| SkillTool | name, description, kind, command, args | 工具定义 |
| SkillsService | (load/from_skills/list/find/all_tools) | 服务层 |

## 4. SkillTool 对比 (可执行工具)

| 维度 | ZeroClaw | Hermes | Shadow |
|------|----------|--------|--------|
| 有无 | 有 (skill_tool.rs + skill_http.rs) | 无 | 有数据结构,未实现Tool trait |
| kind | shell/http/builtin | - | shell/http/builtin |
| 执行方式 | Tool::execute() 执行shell命令模板 | - | 未实现 |
| 命令模板 | {arg_name}占位符替换 | - | {arg_name}占位符 |
| HTTP工具 | skill_http.rs: 发HTTP请求 | - | 未实现 |
| 参数校验 | Tool trait的parameters_schema | - | 有args字段,无schema |

## 5. SkillsService 方法对比

| 方法 | ZeroClaw | Shadow | 差距 |
|------|----------|--------|------|
| new(config, install_root) | 有 | 无(用workspace_dir) | 设计不同 |
| list_skills | 有(含origin/editable/shadowed) | 有(返回&[Skill]) | ZeroClaw更丰富 |
| resolve_effective_skills | 四源合并+审计+去重 | 无 | 缺 |
| read_skill | 有(SkillDocument) | 无(直接读文件) | 缺 |
| write_skill | 有(仅bundle可写) | 无 | 缺 |
| scaffold_skill | 有(创建新skill) | 无 | 缺 |
| remove_skill | 有(Archive/Purge) | 无 | 缺 |
| resolve_ref | 有(name+bundle去歧义) | 无 | 缺 |
| list_bundles | 有 | 无 | 缺 |
| load | 无(在resolve_effective_skills内) | 有(load workspace) | Shadow有简化版 |
| from_skills | 无 | 有(从Vec<Skill>构造) | Shadow独有 |
| find | 无 | 有(按name查找) | Shadow独有 |
| all_tools | 无 | 有(转Vec<Box<dyn Tool>>) | Shadow独有但未实现 |
| parse_skill_md | 无(用SkillDocument::parse) | 无 | Shadow有 |

## 6. 四源合并对比 (ZeroClaw独有)

| 源 | 路径 | 优先级 | 说明 |
|----|------|--------|------|
| Workspace | agents/<alias>/workspace/skills/ | 最高 | agent私有技能 |
| OpenSkills | shared/open-skills/ (repo同步) | 中 | 开源技能库 |
| Plugin | plugins/<name>/skills/ (WASM) | 中 | 插件携带技能 |
| Bundle | shared/skills/<alias>/ (config配置) | 低 | 配置技能包 |

| 冲突处理 | 说明 |
|----------|------|
| shadowed | 同名高优先级加载, 低优先级记录到shadowed |
| dropped | 审计检查失败的记录到dropped |

Shadow: 单一源 (~/.shadow/skills/), 无冲突处理。

## 7. SkillForge 自改进对比 (ZeroClaw独有)

| 阶段 | 作用 | Shadow |
|------|------|--------|
| Scout | 从GitHub/ClawHub搜索候选skill | 无 |
| Evaluate | 评分(0.7阈值) | 无 |
| Integrate | 下载+生成manifest+安装 | 无 |
| auto_integrate | 自动集成(可配置) | 无 |
| scan_interval | 24小时扫描间隔 | 无 |

Shadow: 注释agent.rs中有skill_improver字段, 无实现。

## 8. 远程仓库对比

| 特性 | ZeroClaw | Hermes | Shadow |
|------|----------|--------|--------|
| 远程仓库 | open-skills repo + ClawHub + skills-registry | 无 | 无 |
| 自动同步 | 7天间隔, git clone/pull | 无 | 无 |
| ClawHub下载 | 50MiB限制, ZIP解压, 签名验证 | 无 | 无 |
| 额外注册源 | [[skills.extra_registries]]配置 | 无 | 无 |
| .bundled_manifest | 无 | 有(hash校验) | 无 |

## 9. Skill 注入 system prompt 对比

| 维度 | ZeroClaw | Hermes | Shadow |
|------|----------|--------|--------|
| 注入内容 | effective skills的description | name + description | prompts |
| 注入位置 | system prompt的skills section | system prompt的available_skills | system prompt |
| 按需加载 | read_skill工具读全文 | skill_view工具读全文 | 无 |
| 工具注入 | SkillTool注册到ToolRegistry | 无(纯知识) | 未接入 |
| 匹配机制 | agent判断任务匹配skill | agent判断+强制加载指令 | 无 |

## 10. Skill 管理工具对比

| 工具 | ZeroClaw | Hermes | Shadow |
|------|----------|--------|--------|
| 查看列表 | CLI: zeroclaw skills list | skill_manage(list) | SkillsService::list() |
| 查看内容 | CLI: zeroclaw skills read | skill_view(name) | 无 |
| 创建 | CLI: zeroclaw skills create | skill_manage(create) | 无 |
| 修改 | SkillsService::write_skill | skill_manage(update) | 无 |
| 删除 | SkillsService::remove_skill | skill_manage(delete) | 无 |
| 脚手架 | SkillsService::scaffold_skill | 无 | 无 |
| 安装 | git clone/ZIP下载 | 无 | 无 |

## 11. Shadow 差距表

| # | 能力 | ZeroClaw | Hermes | Shadow | 需要做什么 | 优先级 |
|---|------|----------|--------|--------|-----------|--------|
| 1 | SkillTool实现Tool trait | 有 | 无 | 数据结构有,未实现 | 实现Tool::execute(执行shell命令模板) | P0 |
| 2 | 注册到attribution.rs | 有 | 无 | 未注册 | tool_attribution!(SkillTool, ToolKind::Plugin) | P0 |
| 3 | default_tools加载skill | 有 | 启动时扫描 | 未接入 | 在default_tools中调用SkillsService::load+all_tools | P0 |
| 4 | frontmatter字段补全 | 完整 | 完整 | 只有name/description/tools/prompts | 补version/author/license/tags | P1 |
| 5 | references/templates支持 | 有 | 有 | 无 | skill_view可加载附属文件 | P1 |
| 6 | skill_view工具 | 无(用CLI) | 有 | 无 | 实现skill_view/skill_list/skill_manage工具 | P1 |
| 7 | 多源合并 | 四源 | 单源 | 单源 | 暂不需要,单源够用 | P3 |
| 8 | SkillForge自改进 | 有 | 无 | 无 | 暂不需要 | P3 |
| 9 | 远程仓库同步 | 有 | 无 | 无 | 暂不需要 | P3 |
| 10 | 冲突处理 | shadowed/dropped | 无 | 无 | 暂不需要 | P3 |
| 11 | slash_options | 有 | 无 | 无 | 暂不需要(无Discord) | P3 |

## Shadow 应该学谁

| 学什么 | 从谁学 | 为什么 |
|--------|--------|--------|
| SkillTool实现Tool trait | ZeroClaw | skill定义的工具应该可被LLM调用 |
| frontmatter格式 | Hermes | 简洁清晰, metadata.hermes.tags设计好 |
| references/templates | Hermes | 附属文件机制,skill_view加载 |
| skill_view/skill_manage工具 | Hermes | CLI/TUI需要skill管理能力 |
| 单源加载 | Hermes | 简单够用,不需要四源合并 |
| 不学SkillForge | - | 太重,自改进非P0 |
| 不学远程仓库 | - | 太重,手动安装够用 |
| 不学slash_options | - | 无Discord渠道 |
