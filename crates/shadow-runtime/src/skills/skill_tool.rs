//! 技能 Shell 工具 -- 从 SKILL.md 定义的 shell 命令工具
//!
//! 每个 SkillShellTool 由 SkillTool 定义构造:
//! - 工具名格式: `{skill_name}__{tool_name}`
//! - 命令模板中的 `{arg_name}` 占位符会被模型提供的参数值替换
//! - 锁定的参数 (command 模板和 arg 名称) 不可被模型覆盖

use agent_core::{Attributable, Role, Tool, ToolResult};
use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::time::Duration;

use super::SkillTool;

/// stdout 最大输出字节数 (10KB), 超过则截断
const MAX_STDOUT_BYTES: usize = 10 * 1024;

/// Shell 技能工具 -- 执行 SKILL.md 中定义的 shell 命令
///
/// 从 SkillTool 定义构造:
/// - `command` 是命令模板, 可含 `{arg_name}` 占位符
/// - `args` 列出模型可提供的参数名
/// - 模型只能为声明的参数提供值, 不能修改命令模板本身 (锁定)
///
/// # 示例
/// ```ignore
/// use shadow_runtime::skills::{SkillTool, SkillShellTool};
///
/// let tool_def = SkillTool {
///     name: "status".to_string(),
///     description: "查看 git 状态".to_string(),
///     kind: "shell".to_string(),
///     command: "git status {path}".to_string(),
///     args: vec!["path".to_string()],
/// };
/// let tool = SkillShellTool::new("git-helper", tool_def);
/// // 工具名: "git-helper__status"
/// ```
pub struct SkillShellTool {
    /// 完整工具名: "{skill_name}__{tool_name}"
    full_name: String,
    /// 工具描述 (给 LLM 看)
    description: String,
    /// 命令模板 (可含 {arg_name} 占位符)
    command: String,
    /// 参数名列表 (模型可提供值, 命令模板本身锁定不可覆盖)
    args: Vec<String>,
}

impl SkillShellTool {
    /// 从技能名和工具定义构造
    ///
    /// # 参数
    /// - `skill_name`: 技能名称 (用于生成工具全名)
    /// - `tool_def`: 技能工具定义
    pub fn new(skill_name: &str, tool_def: SkillTool) -> Self {
        let full_name = format!("{}__{}", skill_name, tool_def.name);
        Self {
            full_name,
            description: tool_def.description,
            command: tool_def.command,
            args: tool_def.args,
        }
    }
}

impl Attributable for SkillShellTool {
    fn role(&self) -> Role {
        Role::Tool
    }
    fn alias(&self) -> &str {
        &self.full_name
    }
}

#[async_trait]
impl Tool for SkillShellTool {
    fn name(&self) -> &str {
        &self.full_name
    }

    fn description(&self) -> &str {
        &self.description
    }

    /// 参数 schema -- 为每个声明的参数生成 string 类型
    ///
    /// 模型只能提供 args 列表中的参数, command 模板本身不可覆盖
    fn parameters_schema(&self) -> Value {
        let mut properties = serde_json::Map::new();
        for arg in &self.args {
            properties.insert(
                arg.clone(),
                json!({
                    "type": "string",
                    "description": format!("参数 {} 的值", arg)
                }),
            );
        }
        json!({
            "type": "object",
            "properties": properties,
            "required": []
        })
    }

    /// 技能工具需要审批 -- 涉及 shell 命令执行
    fn requires_approval(&self) -> bool {
        true
    }

    /// 超时 30 秒
    fn timeout(&self) -> Option<Duration> {
        Some(Duration::from_secs(30))
    }

    /// 执行工具
    ///
    /// 流程:
    /// 1. 从模型提供的参数中提取值
    /// 2. 将值替换到命令模板的 {arg_name} 占位符中
    /// 3. 通过 sh -c 执行命令
    /// 4. 返回 stdout (截断超长输出) + stderr
    async fn execute(&self, args: Value) -> Result<ToolResult> {
        // 从模型提供的参数中提取值, 替换命令模板中的占位符
        let mut command = self.command.clone();

        for arg_name in &self.args {
            let value = args
                .get(arg_name)
                .and_then(|v| v.as_str())
                .unwrap_or("");
            // 替换 {arg_name} 占位符
            command = command.replace(&format!("{{{}}}", arg_name), value);
        }

        tracing::debug!("技能工具执行命令: {}", command);

        // 使用 sh -c 执行命令
        let output = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(&command)
            .output()
            .await
            .map_err(|e| anyhow::anyhow!("执行命令失败: {e}"))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        // 截断超长 stdout
        let stdout = truncate_stdout(&stdout);

        if output.status.success() {
            // 成功: 返回 stdout (如果有 stderr 也附加)
            let result = if stderr.is_empty() {
                stdout
            } else {
                format!("{stdout}\n[stderr]\n{stderr}")
            };
            Ok(ToolResult::ok(result))
        } else {
            // 失败: 返回 exit code + stderr
            let code = output.status.code().unwrap_or(-1);
            Ok(ToolResult::err(format!(
                "命令退出码: {code}\n[stdout]\n{stdout}\n[stderr]\n{stderr}"
            )))
        }
    }
}

/// 截断超长 stdout -- 超过 MAX_STDOUT_BYTES 时只保留前部分并附加提示
fn truncate_stdout(stdout: &str) -> String {
    if stdout.len() <= MAX_STDOUT_BYTES {
        return stdout.to_string();
    }

    // 在字节边界安全截断 (避免截断 UTF-8 字符的中间)
    let mut end = MAX_STDOUT_BYTES;
    while end > 0 && !stdout.is_char_boundary(end) {
        end -= 1;
    }

    format!(
        "{}\n\n[stdout 已截断, 显示前 {end} / 共 {} 字节]",
        &stdout[..end],
        stdout.len()
    )
}

// ── 单元测试 ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_tool(name: &str, command: &str, args: Vec<&str>) -> SkillShellTool {
        SkillShellTool::new(
            "test-skill",
            SkillTool {
                name: name.to_string(),
                description: format!("测试工具 {}", name),
                kind: "shell".to_string(),
                command: command.to_string(),
                args: args.into_iter().map(String::from).collect(),
            },
        )
    }

    #[test]
    fn test_tool_name_format() {
        let tool = make_tool("status", "git status", vec![]);
        assert_eq!(tool.name(), "test-skill__status");
    }

    #[test]
    fn test_tool_description() {
        let tool = make_tool("status", "git status", vec![]);
        assert_eq!(tool.description(), "测试工具 status");
    }

    #[test]
    fn test_tool_requires_approval() {
        let tool = make_tool("status", "git status", vec![]);
        assert!(tool.requires_approval());
    }

    #[test]
    fn test_tool_timeout() {
        let tool = make_tool("status", "git status", vec![]);
        assert_eq!(tool.timeout(), Some(Duration::from_secs(30)));
    }

    #[test]
    fn test_parameters_schema_no_args() {
        let tool = make_tool("status", "git status", vec![]);
        let schema = tool.parameters_schema();
        assert!(schema["properties"].as_object().unwrap().is_empty());
    }

    #[test]
    fn test_parameters_schema_with_args() {
        let tool = make_tool("run", "docker run {image}", vec!["image"]);
        let schema = tool.parameters_schema();
        let props = schema["properties"].as_object().unwrap();
        assert!(props.contains_key("image"));
        assert_eq!(props["image"]["type"], "string");
    }

    #[tokio::test]
    async fn test_execute_simple_command() {
        let tool = make_tool("echo", "echo hello", vec![]);
        let result = tool.execute(json!({})).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("hello"));
    }

    #[tokio::test]
    async fn test_execute_with_arg_substitution() {
        let tool = make_tool("echo", "echo {text}", vec!["text"]);
        let result = tool.execute(json!({"text": "world"})).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("world"));
    }

    #[tokio::test]
    async fn test_execute_with_missing_arg() {
        // 缺少参数值 -- 占位符被替换为空字符串
        let tool = make_tool("echo", "echo {text}", vec!["text"]);
        let result = tool.execute(json!({})).await.unwrap();
        assert!(result.success);
        // echo 后面是空字符串, 输出为空行
        assert!(result.output.trim().is_empty());
    }

    #[tokio::test]
    async fn test_execute_multiple_args() {
        let tool = make_tool("echo", "echo {a} {b}", vec!["a", "b"]);
        let result = tool.execute(json!({"a": "hello", "b": "world"})).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("hello world"));
    }

    #[tokio::test]
    async fn test_execute_failed_command() {
        let tool = make_tool("false", "false", vec![]);
        let result = tool.execute(json!({})).await.unwrap();
        assert!(!result.success);
        assert!(result.error.is_some());
    }

    #[tokio::test]
    async fn test_execute_stdout_truncation() {
        // 输出约 20KB, 超过 10KB 限制
        let tool = make_tool("yes", "yes x | head -c 20480", vec![]);
        let result = tool.execute(json!({})).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("[stdout 已截断"));
    }

    #[test]
    fn test_attribution() {
        let tool = make_tool("status", "git status", vec![]);
        assert_eq!(tool.role(), Role::Tool);
        assert_eq!(tool.alias(), "test-skill__status");
    }

    #[test]
    fn test_spec() {
        let tool = make_tool("status", "git status", vec!["path"]);
        let spec = tool.spec();
        assert_eq!(spec.name, "test-skill__status");
        assert_eq!(spec.description, "测试工具 status");
        assert!(spec.parameters["properties"]["path"].is_object());
    }
}
