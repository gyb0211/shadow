//! 系统提示构建器 -- 可插拔的系统提示组成部分
//!
//! 借鉴 ZeroClaw 的 PromptSection 设计, 但大幅精简:
//! - 5 个基础 section: DateTime / Identity / ToolHonesty / Safety / Workspace
//! - SystemPromptBuilder 按 priority 排序后拼接

use shadow_core::AutonomyLevel;
use chrono::Local;
use std::path::PathBuf;

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
        sorted.sort_by(|a, b| b.priority().cmp(&a.priority()));
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
#[cfg(test)]
mod tests {
    use super::*;

    /// 构造测试用 PromptContext
    fn make_ctx() -> PromptContext {
        PromptContext {
            alias: "shadow".to_string(),
            model: "gpt-4o".to_string(),
            tool_count: 5,
            workspace_dir: PathBuf::from("/tmp/workspace"),
        }
    }

    /// 测试: with_defaults 构建包含全部 5 个基础 section
    #[test]
    fn test_with_defaults() {
        let builder = SystemPromptBuilder::with_defaults();
        let ctx = make_ctx();
        let prompt = builder.build(&ctx);

        // 应包含身份信息
        assert!(prompt.contains("你是 shadow"), "应包含身份信息");
        // 应包含工作目录
        assert!(prompt.contains("/tmp/workspace"), "应包含工作目录");
        // 应包含安全约束
        assert!(prompt.contains("安全约束"), "应包含安全约束");
        // 应包含工具诚实性
        assert!(prompt.contains("工具诚实性"), "应包含工具诚实性约束");
        // 应包含当前时间
        assert!(prompt.contains("当前时间"), "应包含当前时间");
    }

    /// 测试: priority 排序 -- Identity(100) 应在 Safety(70) 之前, Safety 在 ToolHonesty(60) 之前
    #[test]
    fn test_priority_ordering() {
        let builder = SystemPromptBuilder::with_defaults();
        let ctx = make_ctx();
        let prompt = builder.build(&ctx);

        let identity_pos = prompt.find("你是 shadow").unwrap();
        let safety_pos = prompt.find("安全约束").unwrap();
        let honesty_pos = prompt.find("工具诚实性").unwrap();

        // priority 降序: identity(100) > safety(70) > tool_honesty(60)
        assert!(identity_pos < safety_pos, "identity 应在 safety 之前");
        assert!(safety_pos < honesty_pos, "safety 应在 tool_honesty 之前");
    }

    /// 测试: 自定义 section 添加 + 最高优先级排最前
    #[test]
    fn test_custom_section() {
        struct CustomSection;
        impl PromptSection for CustomSection {
            fn name(&self) -> &str {
                "custom"
            }
            fn render(&self, _ctx: &PromptContext) -> String {
                "自定义内容".to_string()
            }
            fn priority(&self) -> i32 {
                200 // 最高优先级
            }
        }

        let builder = SystemPromptBuilder::new().section(CustomSection);
        let ctx = make_ctx();
        let prompt = builder.build(&ctx);

        assert!(prompt.contains("自定义内容"));
        // 自定义 section 优先级最高, 应在最前面
        assert!(
            prompt.starts_with("自定义内容"),
            "最高优先级 section 应排在最前"
        );
    }
}
