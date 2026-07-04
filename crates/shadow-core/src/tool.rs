//! 工具 trait -- agent 可调用的能力

use crate::attribution::{Attributable, Role};
use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

/// 工具执行结果
#[derive(Debug, Clone)]
pub struct ToolResult {
    pub success: bool,
    pub output: String,
    pub error: Option<String>,
}

impl ToolResult {
    /// 成功结果
    #[must_use]
    pub fn ok(output: impl Into<String>) -> Self {
        Self { success: true, output: output.into(), error: None }
    }

    /// 失败结果
    #[must_use]
    pub fn err(error: impl Into<String>) -> Self {
        Self { success: false, output: String::new(), error: Some(error.into()) }
    }
}

/// 工具规格 -- 描述给 LLM 的工具元信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

/// 工具 trait
///
/// 每个工具实现此 trait, agent 通过 execute() 调用
#[async_trait]
pub trait Tool: Attributable {
    /// 工具名称
    fn name(&self) -> &str;

    /// 工具描述 (给 LLM 看)
    fn description(&self) -> &str;

    /// 参数 JSON Schema
    fn parameters_schema(&self) -> Value;

    /// 执行工具
    async fn execute(&self, args: Value) -> Result<ToolResult>;

    /// 工具超时 -- None 表示不限制, Agent 用 tokio::time::timeout 包装
    /// 默认 30 秒
    fn timeout(&self) -> Option<Duration> {
        Some(Duration::from_secs(30))
    }

    /// 是否需要审批 -- Supervised 模式下执行前请求用户确认
    /// 默认 false, 敏感工具 (Shell/FileWrite) 覆盖为 true
    fn requires_approval(&self) -> bool {
        false
    }

    /// 校验参数是否符合 parameters_schema() (P1.6)
    ///
    /// 默认实现使用 jsonschema crate 做 JSON Schema 运行时校验:
    /// 1. 从 parameters_schema() 获取 schema
    /// 2. 编译 schema 为校验器 (自动检测 draft 版本)
    /// 3. 校验 args 是否符合 schema
    ///
    /// 工具可覆盖此方法实现自定义校验逻辑 (如跨字段约束检查).
    ///
    /// # 参数
    /// - `args`: 工具调用参数 (serde_json::Value)
    ///
    /// # 返回
    /// - `Ok(())`: 参数符合 schema
    /// - `Err(String)`: 校验失败, 包含明确的错误描述
    fn validate_args(&self, args: &Value) -> std::result::Result<(), String> {
        let schema = self.parameters_schema();

        // 非对象 schema 跳过校验 (兼容空 schema 或 null)
        if !schema.is_object() {
            return Ok(());
        }

        // 编译 JSON Schema (自动检测 draft 版本)
        let validator = jsonschema::validator_for(&schema)
            .map_err(|e| format!("参数 schema 编译失败: {e}"))?;

        // 校验参数
        if let Err(err) = validator.validate(args) {
            return Err(format!("参数校验失败: {err}"));
        }

        Ok(())
    }

    /// 工具规格 (name + description + parameters)
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name().to_string(),
            description: self.description().to_string(),
            parameters: self.parameters_schema(),
        }
    }
}

/// 工具归属 -- 所有工具的 Role 都是 Tool
pub struct ToolAttribution {
    name: String,
}

impl ToolAttribution {
    #[must_use]
    pub fn new(name: &str) -> Self {
        Self { name: name.to_string() }
    }
}

impl Attributable for ToolAttribution {
    fn role(&self) -> Role {
        Role::Tool
    }
    fn alias(&self) -> &str {
        &self.name
    }
}

/// 宏: 为工具 struct 快速实现 Attributable
#[macro_export]
macro_rules! tool_attribution {
    ($name:expr) => {
        fn role(&self) -> $crate::attribution::Role {
            $crate::attribution::Role::Tool
        }
        fn alias(&self) -> &str {
            $name
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 测试用工具 -- 有明确的参数 schema
    struct SchemaTool;

    impl Attributable for SchemaTool {
        fn role(&self) -> Role {
            Role::Tool
        }
        fn alias(&self) -> &str {
            "schema_tool"
        }
    }

    #[async_trait]
    impl Tool for SchemaTool {
        fn name(&self) -> &str {
            "schema_tool"
        }
        fn description(&self) -> &str {
            "测试工具 -- 有参数 schema"
        }
        fn parameters_schema(&self) -> Value {
            json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "count": { "type": "integer", "minimum": 0 }
                },
                "required": ["name"]
            })
        }
        async fn execute(&self, _args: Value) -> Result<ToolResult> {
            Ok(ToolResult::ok("ok"))
        }
    }

    /// 测试用工具 -- 空 schema (无约束)
    struct EmptySchemaTool;

    impl Attributable for EmptySchemaTool {
        fn role(&self) -> Role {
            Role::Tool
        }
        fn alias(&self) -> &str {
            "empty_schema_tool"
        }
    }

    #[async_trait]
    impl Tool for EmptySchemaTool {
        fn name(&self) -> &str {
            "empty_schema_tool"
        }
        fn description(&self) -> &str {
            "测试工具 -- 空 schema"
        }
        fn parameters_schema(&self) -> Value {
            json!({})
        }
        async fn execute(&self, _args: Value) -> Result<ToolResult> {
            Ok(ToolResult::ok("ok"))
        }
    }

    // ---- 校验通过测试 ----

    #[test]
    fn validate_args_passes_valid_args() {
        let tool = SchemaTool;
        let args = json!({"name": "hello", "count": 5});
        assert!(tool.validate_args(&args).is_ok());
    }

    #[test]
    fn validate_args_passes_partial_args() {
        // 只提供 required 字段, 可选字段缺失也应通过
        let tool = SchemaTool;
        let args = json!({"name": "hello"});
        assert!(tool.validate_args(&args).is_ok());
    }

    #[test]
    fn validate_args_passes_extra_properties() {
        // 额外属性默认允许 (无 additionalProperties: false)
        let tool = SchemaTool;
        let args = json!({"name": "hello", "extra": true});
        assert!(tool.validate_args(&args).is_ok());
    }

    // ---- 校验失败测试 ----

    #[test]
    fn validate_args_rejects_missing_required() {
        let tool = SchemaTool;
        let args = json!({"count": 5});
        let result = tool.validate_args(&args);
        assert!(result.is_err());
        // 错误信息应包含缺失的字段名
        assert!(result.unwrap_err().contains("name"));
    }

    #[test]
    fn validate_args_rejects_wrong_type() {
        // name 应为 string, 传入 integer
        let tool = SchemaTool;
        let args = json!({"name": 123});
        let result = tool.validate_args(&args);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("参数校验失败"));
    }

    #[test]
    fn validate_args_rejects_constraint_violation() {
        // count 应 >= 0, 传入 -1
        let tool = SchemaTool;
        let args = json!({"name": "hello", "count": -1});
        let result = tool.validate_args(&args);
        assert!(result.is_err());
    }

    #[test]
    fn validate_args_rejects_wrong_root_type() {
        // schema 要求根类型为 object, 传入 array
        let tool = SchemaTool;
        let args = json!([1, 2, 3]);
        let result = tool.validate_args(&args);
        assert!(result.is_err());
    }

    // ---- 空 schema 测试 ----

    #[test]
    fn validate_args_empty_schema_accepts_anything() {
        // 空 schema {} 无任何约束, 任何参数都应通过
        let tool = EmptySchemaTool;
        let args = json!({"anything": true, "any_type": 42});
        assert!(tool.validate_args(&args).is_ok());
    }

    #[test]
    fn validate_args_empty_schema_accepts_non_object() {
        // 空 schema 不限定根类型, 非 object 也可通过
        let tool = EmptySchemaTool;
        let args = json!("just a string");
        assert!(tool.validate_args(&args).is_ok());
    }
}
