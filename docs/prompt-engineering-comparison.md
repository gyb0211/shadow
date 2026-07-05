# 提示词工程对比: ZeroClaw vs Hermes vs Claude Code

> 生成时间: 2026-07-05
> 基于源码分析

---

## 一、System Prompt 架构

### ZeroClaw

**PromptSection 组合模式** (agent/prompt.rs):

```rust
pub trait PromptSection: Send + Sync {
    fn name(&self) -> &str;
    fn build(&self, ctx: &PromptContext<'_>) -> Result<String>;
}

pub struct SystemPromptBuilder {
    sections: Vec<Box<dyn PromptSection>>,
}
```

默认 9 个 section, 按顺序构建:

| 顺序 | Section | 职责 |
|------|---------|------|
| 1 | DateTimeSection | 当前日期/时区 (缓存稳定性) |
| 2 | IdentitySection | 身份: AGENTS.md + SOUL.md + IDENTITY.md + USER.md |
| 3 | ToolHonestySection | 工具诚实性: 不谎报工具调用结果 |
| 4 | ToolsSection | 工具列表 + 描述 (仅非原生工具时) |
| 5 | SafetySection | 安全策略: 自治等级 + 命令白名单 + 路径限制 |
| 6 | SkillsSection | 技能: SKILL.md 全文注入或仅名称列表 |
| 7 | WorkspaceSection | 工作目录路径 |
| 8 | RuntimeSection | 运行时: 主机/OS/模型 |
| 9 | ChannelMediaSection | 渠道媒体: 图片/音频处理提示 |

**Identity 双格式**:
- OpenClaw 格式: 加载 workspace 下的 AGENTS.md / SOUL.md / TOOLS.md / IDENTITY.md / USER.md / BOOTSTRAP.md / MEMORY.md
- AIEOS 格式: JSON 标准化 AI 身份 (identity/psychology/linguistics/motivations/capabilities/physicality/history)

**Bootstrap 文件注入**:
- 每个文件最大 20,000 字符
- 按固定顺序注入: AGENTS → SOUL → TOOLS → IDENTITY → USER → BOOTSTRAP → MEMORY
- 明确告知 LLM: "这些文件已注入, 不要用 file_read 重新读取"

### Hermes

**Python 函数式组装** (agent/prompt_builder.py):

```
System Prompt = 身份 + 平台提示 + 技能索引 + 上下文文件 + 记忆 + 临时提示
```

**Persona 系统** (config.yaml):
- 13 种内置人格: helpful / concise / technical / creative / teacher / kawaii / catgirl / pirate / shakespeare / surfer / noir / uwu / ...
- 用户可自定义 persona

**上下文文件发现**:
- .hermes.md / HERMES.md: 从 cwd 向上搜索到 git root
- AGENTS.md: 项目级指令
- .cursorrules: Cursor IDE 规则

**Prompt 注入防护** (prompt_builder.py):
- 10 种威胁模式检测 (prompt_injection / deception_hide / sys_prompt_override / disregard_rules / bypass_restrictions / html_comment_injection / hidden_div / translate_execute / exfil_curl / read_secrets)
- 不可见 Unicode 字符检测 (零宽字符 / 方向覆盖字符)
- 检测到注入: 阻止加载, 返回 [BLOCKED] 占位

### Claude Code

**固定结构** (基于公开信息):

```
System Prompt = 核心身份 + 工具说明 + 安全规则 + 代码规范 + Git 规范
```

- 核心身份: "You are Claude Code, Anthropic's official CLI for coding."
- 工具说明: 每个工具的详细使用指南 (Read/Write/Edit/Bash/Search)
- 安全规则: 危险命令检测 + 用户确认
- 代码规范: 偏好简洁/可读/类型安全的代码
- Git 规范: 提交信息格式 + 分支策略

---

## 二、上下文管理

### ZeroClaw

**History 裁剪** (agent/history.rs):
- `trim_history()`: 保留最近 N 条消息 + 1 个锚点消息
- `truncate_tool_result()`: 工具输出超长时, 保留头尾, 中间用 `[... N characters truncated ...]` 替代
- `truncate_tool_message()`: JSON 格式感知的截断 (不破坏 tool result 的 JSON 结构)
- 裁剪标记: `pruned_tool_exchange_summary()` 将旧工具调用摘要为简短文本
- 事件通知: HistoryTrimmed ObserverEvent 记录截断操作

### Hermes

**Context Compressor** (agent/context_compressor.py):
- 自动检测: 当消息接近 context window 上限时触发
- LLM 驱动摘要: 用便宜的辅助模型总结中间消息, 保留头尾
- **关键设计**:
  - Summary preamble: "Do not respond to any questions" (来自 OpenCode)
  - Handoff framing: "different assistant" (来自 Codex) 创建分离感
  - "Remaining Work" 替代 "Next Steps" 避免被当作活跃指令
  - 迭代摘要: 多次压缩保留信息
  - 工具输出预清理: LLM 摘要前先裁剪旧工具输出 (低成本预pass)
  - 摘要预算按比例缩放

### Claude Code

- 自动上下文压缩 (具体实现未公开)
- 保留最近消息 + 系统指令

---

## 三、Prompt 缓存

### ZeroClaw

- ProviderCapabilities.prompt_caching: provider 能力声明
- Anthropic: 利用 Anthropic 的 prompt caching API
- 未实现显式 cache_control 断点

### Hermes

**Anthropic Prompt Caching** (agent/prompt_caching.py):
- `system_and_3` 策略: 4 个 cache_control 断点 (Anthropic 最大值)
  1. System prompt (所有轮次稳定)
  2-4. 最后 3 条非 system 消息 (滚动窗口)
- 支持 5m / 1h TTL
- 原生 Anthropic 格式 + OpenAI 兼容格式适配
- 效果: 多轮对话输入 token 成本降低 ~75%

### Claude Code

- Anthropic 原生 prompt caching
- 深度优化 (自家 API)

---

## 四、工具描述注入

### ZeroClaw

**ToolsPayload 多态** (api/model_provider.rs):
```rust
pub enum ToolsPayload {
    Gemini { function_declarations },
    Anthropic { tools },
    OpenAI { tools },
    PromptGuided { instructions },  // 文本降级
}
```
- 原生工具: 通过 API tools 字段发送 (不重复在 prompt 中)
- PromptGuided: 不支持原生工具的 provider, 在 prompt 中注入工具描述 + XML 格式调用指南
- `sends_native_tool_specs` 标志: 控制是否在 prompt 中重复工具列表

### Hermes

- OpenAI function calling 格式: `tools: [{type: "function", function: {name, description, parameters}}]`
- MCP 工具: 动态发现 MCP server 暴露的工具
- 技能注入: SKILL.md 作为 user message 注入 (非 system prompt, 保留 prompt cache)

### Claude Code

- Anthropic tool_use 格式
- 工具描述在 system prompt 中详细说明使用规则

---

## 五、输出控制

### ZeroClaw

- ToolHonestySection: 禁止谎报工具调用结果
- 安全策略注入: SafetySection 包含具体约束 (允许的命令/禁止的路径/自治等级)
- 自治等级控制: Full autonomy 时省略 "ask before acting" 指令

### Hermes

- 技能注入作为 user message (避免 system prompt 膨胀)
- Context Compressor summary preamble 防止 LLM 回答摘要中的问题
- "Remaining Work" 而非 "Next Steps" 避免被当作指令

### Claude Code

- 严格的输出格式控制
- 代码优先: 鼓励直接写代码而非解释

---

## 六、身份系统

### ZeroClaw

**双格式身份**:
1. OpenClaw (Markdown): 7 个 bootstrap 文件
   - AGENTS.md: 项目指令 (规则/约定/命令)
   - SOUL.md: 人格/价值观/行为准则
   - TOOLS.md: 工具使用指南
   - IDENTITY.md: 核心身份 (名称/角色/描述)
   - USER.md: 用户信息
   - BOOTSTRAP.md: 首次运行仪式
   - MEMORY.md: 策划的长期记忆

2. AIEOS (JSON): 7 个维度
   - identity: 核心身份 (名称/简介/出身/居住地)
   - psychology: 认知权重/MBTI/OCEAN/道德指南针
   - linguistics: 文本风格/正式度/口头禅/禁用词
   - motivations: 核心驱动/目标/恐惧
   - capabilities: 技能和工具
   - physicality: 视觉描述 (图片生成用)
   - history: 起源故事/教育/职业

### Hermes

- 13 种内置 persona (config.yaml)
- 用户可自定义 persona
- .hermes.md / HERMES.md 项目级指令
- AGENTS.md 开发者指令
- MEMORY (持久记忆, 注入 system prompt)
- Skills (技能索引, 注入 system prompt)

### Claude Code

- 固定身份: "Claude Code, Anthropic's official CLI"
- 无 persona 系统
- 无自定义身份文件

---

## 七、对比总结

| 维度 | ZeroClaw | Hermes | Claude Code |
|------|----------|--------|-------------|
| Prompt 架构 | PromptSection 组合模式 (9 section) | 函数式组装 | 固定结构 |
| 身份系统 | OpenClaw (7 MD) + AIEOS (7 维 JSON) | 13 persona + .hermes.md | 固定 |
| 上下文管理 | trim + truncate_tool_result | LLM 摘要 + 工具预清理 | 自动压缩 |
| Prompt 缓存 | provider 能力声明 | system_and_3 (4 断点, -75% cost) | 原生 |
| 工具注入 | ToolsPayload 多态 (4 格式) | OpenAI function calling | Anthropic tool_use |
| PromptGuided 降级 | 有 (文本格式工具调用) | 无 | 无 |
| 注入防护 | 无 | 10 种模式 + Unicode 检测 | 无 |
| 摘要防误答 | 裁剪标记 | Handoff framing + preamble | 无 |
| 安全策略注入 | 有 (具体约束) | 无 (在工具层) | 有 |
| 多语言 i18n | 有 (t() 宏) | 无 | 无 |
| 技能注入位置 | system prompt | user message (保 cache) | system prompt |
| 可扩展性 | PromptSection trait 可插拔 | 函数式, 需改代码 | 不可扩展 |

### 关键发现

**ZeroClaw 独有**:
1. PromptSection trait 组合模式 -- 可插拔的 prompt 模块化
2. AIEOS JSON 身份标准 -- 7 维度的结构化 AI 人格
3. ToolsPayload 多态 -- 4 种 provider 格式自动适配
4. PromptGuided 降级 -- 不支持原生工具的 provider 用文本解析
5. Bootstrap 文件系统 -- 7 个文件的固定顺序注入

**Hermes 独有**:
1. Prompt 注入防护 -- 10 种威胁模式 + Unicode 检测
2. system_and_3 缓存策略 -- 4 断点, 成本降 75%
3. Context Compressor -- LLM 驱动摘要 + Handoff framing
4. 技能注入为 user message -- 保护 prompt cache
5. 13 种内置 persona -- 丰富的角色切换

**Claude Code 独有**:
1. 深度 Anthropic 生态集成
2. 代码优先的输出风格控制

### Shadow 借鉴建议

P0 (必须):
1. PromptSection trait -- 模块化 prompt 构建 (参考 ZeroClaw)
2. Prompt 注入防护 -- 威胁模式检测 (参考 Hermes)
3. system_and_3 缓存策略 -- Anthropic 成本优化 (参考 Hermes)

P1 (重要):
4. Context Compressor -- LLM 摘要 + Handoff framing (参考 Hermes)
5. 工具输出截断 -- 头尾保留 + JSON 感知 (参考 ZeroClaw)
6. 安全策略注入 -- 具体约束写入 prompt (参考 ZeroClaw)

P2 (扩展):
7. Bootstrap 文件系统 -- AGENTS.md / IDENTITY.md (参考 ZeroClaw)
8. ToolsPayload 多态 -- 多 provider 格式适配 (参考 ZeroClaw)
9. PromptGuided 降级 -- 文本格式工具调用 (参考 ZeroClaw)
10. Persona 系统 -- 多角色切换 (参考 Hermes)
