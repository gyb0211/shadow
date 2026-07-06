//! HTTP 技能工具 -- 执行 kind="http" 的 SkillTool
//!
//! 从 SKILL.md 中 `kind: http` 的工具定义构造. 命令模板作为 URL,
//! 其中的 `{arg_name}` 占位符会被模型提供的参数值替换, 然后发起 HTTP GET 请求.
//!
//! # 示例 SKILL.md
//! ```yaml
//! tools:
//!   - name: lookup
//!     description: 查询 IP 信息
//!     kind: http
//!     command: https://ipinfo.io/{ip}/json
//!     args:
//!       - ip
//! ```
//!
//! 模型调用 `skill__lookup` 并传入 `{"ip": "8.8.8.8"}` 时,
//! 会请求 `https://ipinfo.io/8.8.8.8/json` 并返回响应体.

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{Value, json};
use shadow_core::{Attributable, Role, Tool, ToolResult};
use std::time::Duration;

use super::SkillTool;

/// 响应体最大字节数 (1MB), 超过则截断
const MAX_BODY_BYTES: usize = 1024 * 1024;

/// HTTP 技能工具 -- 执行 SKILL.md 中定义的 HTTP GET 请求
///
/// 从 SkillTool 定义构造:
/// - `command` 作为 URL 模板, 可含 `{arg_name}` 占位符
/// - `args` 列出模型可提供的参数名
///
/// 工具名格式: `{skill_name}__{tool_name}` (与 SkillShellTool 一致)
pub struct SkillHttpTool {
    /// 完整工具名: "{skill_name}__{tool_name}"
    full_name: String,
    /// 工具定义 (描述、URL 模板、参数)
    tool_def: SkillTool,
}

impl SkillHttpTool {
    /// 从技能名和工具定义构造
    ///
    /// # 参数
    /// - `skill_name`: 技能名称 (用于生成工具全名)
    /// - `tool_def`: 技能工具定义 (command 字段作为 URL 模板)
    pub fn new(skill_name: &str, tool_def: SkillTool) -> Self {
        let full_name = format!("{}__{}", skill_name, tool_def.name);
        Self {
            full_name,
            tool_def,
        }
    }
}

impl Attributable for SkillHttpTool {
    fn role(&self) -> Role {
        Role::Tool
    }
    fn alias(&self) -> &str {
        &self.full_name
    }
}

#[async_trait]
impl Tool for SkillHttpTool {
    fn name(&self) -> &str {
        &self.full_name
    }

    fn description(&self) -> &str {
        &self.tool_def.description
    }

    /// 参数 schema -- 为每个声明的参数生成 string 类型
    ///
    /// 模型只能提供 args 列表中的参数, URL 模板本身不可覆盖
    fn parameters_schema(&self) -> Value {
        let mut properties = serde_json::Map::new();
        let mut required = Vec::new();
        for arg in &self.tool_def.args {
            properties.insert(
                arg.clone(),
                json!({
                    "type": "string",
                    "description": format!("参数 {} 的值", arg)
                }),
            );
            required.push(arg.clone());
        }
        json!({
            "type": "object",
            "properties": properties,
            "required": required,
        })
    }

    /// 执行 HTTP GET 请求
    ///
    /// 流程:
    /// 1. 将模型提供的参数值替换到 URL 模板的 `{arg_name}` 占位符中
    /// 2. 发起 HTTP GET 请求 (30 秒超时)
    /// 3. 读取响应体 (截断超过 1MB 的部分)
    /// 4. 根据状态码返回成功/失败结果
    async fn execute(&self, args: Value) -> Result<ToolResult> {
        // URL 构建: 替换 command 中的 {param} 占位符
        let mut url = self.tool_def.command.clone();
        if let Some(obj) = args.as_object() {
            for (key, val) in obj {
                if let Some(s) = val.as_str() {
                    url = url.replace(&format!("{{{}}}", key), s);
                }
            }
        }

        tracing::debug!("HTTP 技能工具请求: {} ({})", url, self.full_name);

        // 构建 HTTP 客户端 (30 秒超时)
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| anyhow::anyhow!("构建 HTTP 客户端失败: {e}"))?;

        // 发起 GET 请求
        // 注: 将请求失败转为 ToolResult::err (而非 Err), 便于 agent 循环处理
        let resp = match client.get(&url).send().await {
            Ok(r) => r,
            Err(e) => {
                return Ok(ToolResult::err(format!("HTTP 请求失败: {e}")));
            }
        };

        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();

        // 截断超长响应体 (1MB)
        let body = truncate_body(&body);

        if status.is_success() {
            Ok(ToolResult::ok(format!("HTTP {status}\n{body}")))
        } else {
            Ok(ToolResult::err(format!("HTTP {status}: {body}")))
        }
    }

    /// 超时 30 秒
    fn timeout(&self) -> Option<Duration> {
        Some(Duration::from_secs(30))
    }
}

/// 截断超长响应体 -- 超过 MAX_BODY_BYTES 时只保留前部分并附加提示
///
/// 在字节边界安全截断 (避免截断 UTF-8 字符的中间)
fn truncate_body(body: &str) -> String {
    if body.len() <= MAX_BODY_BYTES {
        return body.to_string();
    }

    // 在字节边界安全截断 (避免截断 UTF-8 字符的中间)
    let mut end = MAX_BODY_BYTES;
    while end > 0 && !body.is_char_boundary(end) {
        end -= 1;
    }

    format!("{}...(已截断)", &body[..end])
}

// ── 单元测试 ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_tool(name: &str, command: &str, args: Vec<&str>) -> SkillHttpTool {
        SkillHttpTool::new(
            "test-skill",
            SkillTool {
                name: name.to_string(),
                description: format!("测试 HTTP 工具 {}", name),
                kind: "http".to_string(),
                command: command.to_string(),
                args: args.into_iter().map(String::from).collect(),
            },
        )
    }

    #[test]
    fn test_tool_name_format() {
        let tool = make_tool("lookup", "https://example.com", vec![]);
        assert_eq!(tool.name(), "test-skill__lookup");
    }

    #[test]
    fn test_tool_description() {
        let tool = make_tool("lookup", "https://example.com", vec![]);
        assert_eq!(tool.description(), "测试 HTTP 工具 lookup");
    }

    #[test]
    fn test_tool_timeout() {
        let tool = make_tool("lookup", "https://example.com", vec![]);
        assert_eq!(tool.timeout(), Some(Duration::from_secs(30)));
    }

    #[test]
    fn test_attribution() {
        let tool = make_tool("lookup", "https://example.com", vec![]);
        assert_eq!(tool.role(), Role::Tool);
        assert_eq!(tool.alias(), "test-skill__lookup");
    }

    #[test]
    fn test_parameters_schema_no_args() {
        let tool = make_tool("lookup", "https://example.com", vec![]);
        let schema = tool.parameters_schema();
        assert!(schema["properties"].as_object().unwrap().is_empty());
        // 无参数时 required 为空数组
        assert!(schema["required"].as_array().unwrap().is_empty());
    }

    #[test]
    fn test_parameters_schema_with_args() {
        let tool = make_tool("lookup", "https://ipinfo.io/{ip}/json", vec!["ip"]);
        let schema = tool.parameters_schema();
        let props = schema["properties"].as_object().unwrap();
        assert!(props.contains_key("ip"));
        assert_eq!(props["ip"]["type"], "string");
        // ip 应在 required 列表中
        let required = schema["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v == "ip"));
    }

    #[test]
    fn test_spec() {
        let tool = make_tool("lookup", "https://ipinfo.io/{ip}/json", vec!["ip"]);
        let spec = tool.spec();
        assert_eq!(spec.name, "test-skill__lookup");
        assert_eq!(spec.description, "测试 HTTP 工具 lookup");
        assert!(spec.parameters["properties"]["ip"].is_object());
    }

    #[test]
    fn test_truncate_body_short() {
        let s = "hello world";
        assert_eq!(truncate_body(s), s);
    }

    #[test]
    fn test_truncate_body_long() {
        // 构造超过 1MB 的字符串
        let s = "x".repeat(MAX_BODY_BYTES + 100);
        let truncated = truncate_body(&s);
        assert!(truncated.ends_with("(已截断)"));
        assert!(truncated.len() < s.len());
    }

    #[test]
    fn test_truncate_body_utf8_boundary() {
        // 中文字符占 3 字节, 确保截断不会在字符中间断开
        let chunk = "你好"; // 6 字节
        let s = chunk.repeat(MAX_BODY_BYTES / 6 + 10);
        let truncated = truncate_body(&s);
        // 截断后的内容应是合法 UTF-8 (String 保证这点, 这里验证不 panic)
        assert!(truncated.contains("(已截断)"));
    }

    #[tokio::test]
    async fn test_execute_invalid_url() {
        // 无效 URL 应返回错误结果 (而非 panic)
        let tool = make_tool("bad", "not-a-valid-url", vec![]);
        let result = tool.execute(json!({})).await.unwrap();
        assert!(!result.success);
        assert!(result.error.unwrap().contains("HTTP 请求失败"));
    }
}
