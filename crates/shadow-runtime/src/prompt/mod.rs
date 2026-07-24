//! 系统提示构建器 -- 可插拔的系统提示组成部分
//!
//! 借鉴 ZeroClaw 的 PromptSection 设计, 但大幅精简:
//! - 5 个基础 section: DateTime / Identity / ToolHonesty / Safety / Workspace
//! - SystemPromptBuilder 按 priority 排序后拼接
//!
//! # 子模块
//! - [`safety_injection`]: 安全策略注入 -- 将具体安全约束写入 system prompt
//! - [`context_compressor`]: 上下文压缩 -- 工具输出预清理 (不含 LLM 摘要)
//! - [`injection_guard`]: Prompt 注入防护 -- 检测上下文文件中的注入攻击
//! - [`caching`]: Anthropic prompt 缓存 -- system_and_3 策略
//! - [`truncation`]: 工具输出截断 -- 头尾保留 + JSON 感知
//! - [`bootstrap`]: Bootstrap 文件系统 -- 加载 workspace 身份文件注入 prompt
//! - [`tools_payload`]: ToolsPayload 多态 -- 不同 provider 的工具格式适配
//! - [`prompt_guided`]: PromptGuided 降级 -- 不支持原生工具时用文本解析
//! - [`persona`]: Persona 系统 -- 多角色切换

use chrono::Local;
use std::path::PathBuf;

// ── 子模块 ─────────────────────────────────────────────────────────
/// Bootstrap 文件系统 -- 加载 workspace 身份文件注入 prompt
pub mod bootstrap;
/// Anthropic prompt 缓存 -- system_and_3 策略
pub mod caching;
/// 上下文压缩 -- 工具输出预清理 (不含 LLM 摘要, 那个需要 Provider)
pub mod context_compressor;
/// Prompt 注入防护 -- 检测上下文文件中的注入攻击
pub mod injection_guard;
/// Persona 系统 -- 多角色切换
pub mod persona;
/// PromptGuided 降级 -- 不支持原生工具时用文本解析
pub mod prompt_guided;
/// 安全策略注入 -- 将具体安全约束写入 system prompt
// pub mod safety_injection;
/// ToolsPayload 多态 -- 不同 provider 的工具格式适配
pub mod tools_payload;
/// 工具输出截断 -- 头尾保留 + JSON 感知
pub mod truncation;

// 重新导出子模块的公共 API, 方便外部通过 `prompt::` 直接使用
pub use bootstrap::BootstrapSection;
pub use caching::apply_cache_control;
pub use context_compressor::{
    estimate_tokens, prune_old_tool_outputs, prune_to_fit, should_compress,
};
pub use injection_guard::{ScanResult, scan_context_content};
pub use persona::{Persona, PersonaSection, default_persona, get_persona, list_personas};
pub use prompt_guided::{
    ParsedToolCall, build_prompt_guided_instructions, parse_prompt_guided_response,
};
// pub use safety_injection::SafetyInjectionSection;
use shadow_config::autonomy::AutonomyLevel;
pub use tools_payload::{ToolFormat, ToolsPayload, convert_tools};
pub use truncation::{truncate_tool_message, truncate_tool_result};

/// 系统提示段 -- 可插拔的系统提示组成部分
pub trait PromptSection: Send + Sync {
    /// 段名称
    fn name(&self) -> &str;
    /// 渲染为文本
    fn render(&self, ctx: &PromptContext) -> String;
    /// 优先级 (数值越大越靠前, 默认 0)
    fn priority(&self) -> i32 {
        0
    }
}

/// 提示上下文 -- 渲染系统提示时所需的运行时信息
pub struct PromptContext {
    /// agent 别名
    pub alias: String,
    /// 模型名称
    pub model: String,
    /// 可用工具数量
    pub tool_count: usize,
    /// 工作目录
    pub workspace_dir: PathBuf,
}

/// 系统提示构建器 -- 持有多个 PromptSection, 按 priority 排序后拼接
pub struct SystemPromptBuilder {
    sections: Vec<Box<dyn PromptSection>>,
}

impl SystemPromptBuilder {
    /// 创建空的构建器
    pub fn new() -> Self {
        Self {
            sections: Vec::new(),
        }
    }

    /// 创建包含 5 个基础 section 的构建器
    pub fn with_defaults() -> Self {
        Self::new()
            .section(IdentitySection)
            .section(DateTimeSection)
            .section(WorkspaceSection)
            .section(SafetySection::default())
            .section(ToolHonestySection)
    }

    /// 添加一个 section
    pub fn section(mut self, section: impl PromptSection + 'static) -> Self {
        self.sections.push(Box::new(section));
        self
    }

    /// 构建系统提示文本 -- 按 priority 降序排序后拼接
    pub fn build(&self, ctx: &PromptContext) -> String {
        let mut sorted: Vec<&dyn PromptSection> =
            self.sections.iter().map(|s| s.as_ref()).collect();
        // priority 降序: 数值大的排前面
        sorted.sort_by_key(|s| std::cmp::Reverse(s.priority()));
        sorted
            .iter()
            .map(|s| s.render(ctx))
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}

impl Default for SystemPromptBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// ── 基础 Section 实现 ──

/// 日期时间段 -- 显示当前日期时间
pub struct DateTimeSection;

impl PromptSection for DateTimeSection {
    fn name(&self) -> &str {
        "datetime"
    }
    fn render(&self, _ctx: &PromptContext) -> String {
        format!("当前时间: {}", Local::now().format("%Y-%m-%d %H:%M:%S"))
    }
    fn priority(&self) -> i32 {
        90
    }
}

/// 身份段 -- agent 的身份介绍
pub struct IdentitySection;

impl PromptSection for IdentitySection {
    fn name(&self) -> &str {
        "identity"
    }
    fn render(&self, ctx: &PromptContext) -> String {
        format!("你是 {}, 一个有用的 AI 助手.", ctx.alias)
    }
    fn priority(&self) -> i32 {
        100
    }
}

/// 工具诚实性段 -- 约束 agent 不要编造工具调用结果
pub struct ToolHonestySection;

impl PromptSection for ToolHonestySection {
    fn name(&self) -> &str {
        "tool_honesty"
    }
    fn render(&self, _ctx: &PromptContext) -> String {
        "工具诚实性: 不要编造工具调用结果. 如果工具调用失败, 如实报告错误. 不要假装执行了未调用的工具."
            .to_string()
    }
    fn priority(&self) -> i32 {
        60
    }
}

/// 安全段 -- 安全约束 + 自主级别指令
pub struct SafetySection {
    /// 自主级别
    autonomy: AutonomyLevel,
}

impl SafetySection {
    /// 创建安全段, 指定自主级别
    pub fn new(autonomy: AutonomyLevel) -> Self {
        Self { autonomy }
    }
}

impl Default for SafetySection {
    fn default() -> Self {
        Self::new(AutonomyLevel::default())
    }
}

impl PromptSection for SafetySection {
    fn name(&self) -> &str {
        "safety"
    }
    fn render(&self, _ctx: &PromptContext) -> String {
        let autonomy_desc = match self.autonomy {
            AutonomyLevel::Full => "完全自主模式: 你可以自主执行所有操作, 无需审批.",
            AutonomyLevel::Supervised => "受监督模式: 敏感操作需要用户审批后方可执行.",
            AutonomyLevel::ReadOnly => "只读模式: 你只能读取信息, 不得执行任何写操作.",
        };
        format!("安全约束: 不要执行可能造成数据丢失或系统损坏的操作.\n{autonomy_desc}")
    }
    fn priority(&self) -> i32 {
        70
    }
}

/// 工作目录段 -- 显示当前工作目录信息
pub struct WorkspaceSection;

impl PromptSection for WorkspaceSection {
    fn name(&self) -> &str {
        "workspace"
    }
    fn render(&self, ctx: &PromptContext) -> String {
        format!("工作目录: {}", ctx.workspace_dir.display())
    }
    fn priority(&self) -> i32 {
        80
    }
}

// ── 单元测试 ──
