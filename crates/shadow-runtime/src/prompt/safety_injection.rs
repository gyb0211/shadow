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
#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompt::{PromptContext, SystemPromptBuilder};
    use std::path::PathBuf;

    /// 构造测试用 PromptContext
    fn make_ctx() -> PromptContext {
        PromptContext {
            alias: "shadow".to_string(),
            model: "gpt-4o".to_string(),
            tool_count: 5,
            workspace_dir: PathBuf::from("/tmp/workspace"),
        }
    }

    /// 测试: name 与 priority
    #[test]
    fn test_name_and_priority() {
        let section = SafetyInjectionSection::new(SecurityPolicy::new(), AutonomyLevel::Full);
        assert_eq!(section.name(), "safety_injection");
        assert_eq!(section.priority(), 75);
    }

    /// 测试: 默认策略渲染包含黑名单、禁止路径, 不包含白名单 (默认空)
    #[test]
    fn test_render_default_policy() {
        let section = SafetyInjectionSection::new(SecurityPolicy::new(), AutonomyLevel::Full);
        let text = section.render(&make_ctx());

        // 标题
        assert!(text.contains("安全策略注入"), "应包含标题");
        // 自治等级
        assert!(text.contains("[自治等级]"), "应包含自治等级");
        assert!(
            text.contains("完全自主模式"),
            "Full 等级应包含完全自主描述"
        );
        // 黑名单 (默认包含 rm -rf /, mkfs 等)
        assert!(text.contains("[命令黑名单]"), "应包含命令黑名单");
        assert!(text.contains("rm -rf /"), "黑名单应包含 rm -rf /");
        assert!(text.contains("mkfs"), "黑名单应包含 mkfs");
        // 禁止路径
        assert!(text.contains("[禁止路径]"), "应包含禁止路径");
        assert!(text.contains("/etc"), "禁止路径应包含 /etc");
        assert!(text.contains("/root"), "禁止路径应包含 /root");
        // 默认无工作目录限制 -> 不应出现工作目录限制行
        assert!(
            !text.contains("[工作目录限制]"),
            "无工作目录限制时不应输出该行"
        );
        // 默认无白名单 -> 不应出现白名单行
        assert!(
            !text.contains("[命令白名单]"),
            "白名单为空时不应输出该行"
        );
    }

    /// 测试: Supervised 自治等级描述
    #[test]
    fn test_autonomy_supervised() {
        let section =
            SafetyInjectionSection::new(SecurityPolicy::new(), AutonomyLevel::Supervised);
        let text = section.render(&make_ctx());
        assert!(
            text.contains("受监督模式"),
            "Supervised 等级应包含受监督描述"
        );
    }

    /// 测试: ReadOnly 自治等级描述
    #[test]
    fn test_autonomy_readonly() {
        let section = SafetyInjectionSection::new(SecurityPolicy::new(), AutonomyLevel::ReadOnly);
        let text = section.render(&make_ctx());
        assert!(text.contains("只读模式"), "ReadOnly 等级应包含只读描述");
    }

    /// 测试: 设置命令白名单后应输出白名单行
    #[test]
    fn test_render_with_whitelist() {
        let policy = SecurityPolicy::new().with_allowed_commands(vec![
            "ls".to_string(),
            "cat".to_string(),
            "git".to_string(),
        ]);
        let section = SafetyInjectionSection::new(policy, AutonomyLevel::Full);
        let text = section.render(&make_ctx());

        assert!(text.contains("[命令白名单]"), "应包含命令白名单行");
        assert!(text.contains("ls"), "白名单应包含 ls");
        assert!(text.contains("cat"), "白名单应包含 cat");
        assert!(text.contains("git"), "白名单应包含 git");
    }

    /// 测试: 设置工作目录后应输出工作目录限制行
    #[test]
    fn test_render_with_workspace() {
        let policy =
            SecurityPolicy::new().with_workspace(PathBuf::from("/tmp/shadow-workspace"));
        let section = SafetyInjectionSection::new(policy, AutonomyLevel::Supervised);
        let text = section.render(&make_ctx());

        assert!(text.contains("[工作目录限制]"), "应包含工作目录限制行");
        assert!(
            text.contains("/tmp/shadow-workspace"),
            "应包含具体工作目录路径"
        );
    }

    /// 测试: 自定义禁止路径
    #[test]
    fn test_render_with_custom_forbidden_paths() {
        let policy = SecurityPolicy::new().with_forbidden_paths(vec![
            "/custom/secret".to_string(),
            "/var/private".to_string(),
        ]);
        let section = SafetyInjectionSection::new(policy, AutonomyLevel::Full);
        let text = section.render(&make_ctx());

        assert!(text.contains("/custom/secret"), "应包含自定义禁止路径");
        assert!(text.contains("/var/private"), "应包含自定义禁止路径");
        // 覆盖后不应再出现默认的 /etc
        assert!(!text.contains("/etc"), "覆盖后不应包含默认 /etc");
    }

    /// 测试: 完整策略渲染 (白名单 + 黑名单 + 禁止路径 + 工作目录)
    #[test]
    fn test_render_full_policy() {
        let policy = SecurityPolicy::new()
            .with_workspace(PathBuf::from("/home/user/proj"))
            .with_allowed_commands(vec!["ls".to_string(), "grep".to_string()]);
        let section = SafetyInjectionSection::new(policy, AutonomyLevel::ReadOnly);
        let text = section.render(&make_ctx());

        // 四类约束齐全
        assert!(text.contains("[自治等级]"));
        assert!(text.contains("[命令白名单]"));
        assert!(text.contains("[命令黑名单]"));
        assert!(text.contains("[禁止路径]"));
        assert!(text.contains("[工作目录限制]"));
    }

    /// 测试: 在 SystemPromptBuilder 中, SafetyInjectionSection(75) 应排在
    /// Safety(70) 之后、Workspace(80) 之前
    #[test]
    fn test_priority_between_safety_and_workspace() {
        let builder = SystemPromptBuilder::with_defaults().section(SafetyInjectionSection::new(
            SecurityPolicy::new().with_workspace(PathBuf::from("/tmp/ws")),
            AutonomyLevel::Supervised,
        ));
        let prompt = builder.build(&make_ctx());

        let workspace_pos = prompt.find("工作目录:").unwrap_or(usize::MAX);
        let injection_pos = prompt.find("安全策略注入:").unwrap_or(usize::MAX);
        let safety_pos = prompt.find("安全约束:").unwrap_or(usize::MAX);

        // priority 降序: workspace(80) > safety_injection(75) > safety(70)
        assert!(
            workspace_pos < injection_pos,
            "Workspace(80) 应在 SafetyInjection(75) 之前"
        );
        assert!(
            injection_pos < safety_pos,
            "SafetyInjection(75) 应在 Safety(70) 之前"
        );
    }
}
