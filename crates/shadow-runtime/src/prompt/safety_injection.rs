//! 安全策略注入 -- 将具体安全约束写入 system prompt
//!
//! 与 [`super::SafetySection`] 的区别:
//! - `SafetySection` (priority 70): 只输出通用的安全约束 + 自治级别一句话描述
//! - `SafetyInjectionSection` (priority 75): 输出 **具体** 的安全策略 --
//!   命令白名单/黑名单、禁止路径、工作目录限制, 让 agent 在生成命令时
//!   就能感知到边界, 从源头减少危险操作.
//!
//! 优先级 75 设计在 Safety(70) 与 Workspace(80) 之间:
//! 先有通用安全声明 (70), 再注入具体策略 (75), 最后给出工作目录 (80).
//!
//! [`SafetySection`]: super::SafetySection

use super::{PromptContext, PromptSection};
use crate::security::SecurityPolicy;
use shadow_core::AutonomyLevel;

/// 安全策略注入段 -- 将 [`SecurityPolicy`] 中的具体约束渲染到 system prompt
///
/// 注入内容包括:
/// 1. 自治等级描述 (Full / Supervised / ReadOnly)
/// 2. 允许的命令白名单 (如果有)
/// 3. 禁止的命令黑名单 (如 `rm -rf`, `dd`, `mkfs` 等)
/// 4. 禁止的路径 (系统目录 `/etc`, `/root` 等)
/// 5. 工作目录限制
///
/// # 示例
/// ```ignore
/// use shadow_runtime::prompt::{SafetyInjectionSection, SystemPromptBuilder, PromptContext};
/// use shadow_runtime::security::SecurityPolicy;
/// use shadow_core::AutonomyLevel;
/// use std::path::PathBuf;
///
/// let policy = SecurityPolicy::new()
///     .with_workspace(PathBuf::from("/tmp/work"))
///     .with_allowed_commands(vec!["ls".to_string(), "cat".to_string()]);
/// let section = SafetyInjectionSection::new(policy, AutonomyLevel::Supervised);
/// let builder = SystemPromptBuilder::new().section(section);
/// ```
pub struct SafetyInjectionSection {
    /// 安全策略 (黑名单 / 白名单 / 禁止路径 / 工作目录)
    policy: SecurityPolicy,
    /// 自治等级
    autonomy: AutonomyLevel,
}

impl SafetyInjectionSection {
    /// 创建安全策略注入段
    ///
    /// # 参数
    /// - `policy`: 安全策略, 提供命令黑/白名单、禁止路径、工作目录
    /// - `autonomy`: 自治等级, 决定 agent 的自主程度描述
    #[must_use]
    pub fn new(policy: SecurityPolicy, autonomy: AutonomyLevel) -> Self {
        Self { policy, autonomy }
    }

    /// 渲染自治等级描述
    fn render_autonomy(&self) -> String {
        match self.autonomy {
            AutonomyLevel::Full => "完全自主模式: 你可以自主执行所有操作, 无需审批.".to_string(),
            AutonomyLevel::Supervised => "受监督模式: 敏感操作需要用户审批后方可执行.".to_string(),
            AutonomyLevel::ReadOnly => "只读模式: 你只能读取信息, 不得执行任何写操作.".to_string(),
        }
    }
}

impl PromptSection for SafetyInjectionSection {
    fn name(&self) -> &str {
        "safety_injection"
    }

    fn render(&self, _ctx: &PromptContext) -> String {
        let mut lines: Vec<String> = Vec::new();
        lines.push("安全策略注入:".to_string());

        // a. 自治等级描述
        lines.push(format!("[自治等级] {}", self.render_autonomy()));

        // b. 允许的命令白名单 (仅有内容时才输出)
        if !self.policy.allowed_commands().is_empty() {
            let whitelist = self.policy.allowed_commands().join(", ");
            lines.push(format!("[命令白名单] 仅允许执行以下命令: {whitelist}"));
        }

        // c. 禁止的命令黑名单
        if !self.policy.blocked_patterns().is_empty() {
            let blacklist = self.policy.blocked_patterns().join(", ");
            lines.push(format!(
                "[命令黑名单] 以下命令被严格禁止, 不得执行: {blacklist}"
            ));
        }

        // d. 禁止的路径 (系统目录)
        if !self.policy.forbidden_paths().is_empty() {
            let paths = self.policy.forbidden_paths().join(", ");
            lines.push(format!("[禁止路径] 禁止读写以下系统目录: {paths}"));
        }

        // e. 工作目录限制 (仅有限制时才输出)
        if let Some(ws) = self.policy.workspace() {
            lines.push(format!(
                "[工作目录限制] 所有文件操作必须限制在工作目录内: {}",
                ws.display()
            ));
        }

        lines.join("\n")
    }

    fn priority(&self) -> i32 {
        75
    }
}

// ── 单元测试 ──
