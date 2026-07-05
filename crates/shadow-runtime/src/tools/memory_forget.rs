//! MemoryForget 工具 -- 按 key 删除一条记忆

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;

use shadow_core::{tool_attribution, Attributable, Memory, Tool, ToolResult};

/// MemoryForget 工具 -- 按 key 删除指定记忆条目
///
/// 让 LLM 能主动遗忘不再需要的信息 (如过时的偏好、临时上下文),
/// 删除操作需要审批 (requires_approval = true).
pub struct MemoryForgetTool {
    memory: Arc<dyn Memory>,
}

impl MemoryForgetTool {
    /// 创建 MemoryForgetTool, 需要传入 Memory 后端
    pub fn new(memory: Arc<dyn Memory>) -> Self {
        Self { memory }
    }
}

impl Attributable for MemoryForgetTool {
    tool_attribution!("memory_forget");
}

#[async_trait]
impl Tool for MemoryForgetTool {
    fn name(&self) -> &str {
        "memory_forget"
    }

    fn description(&self) -> &str {
        "删除一条记忆。按 key 删除指定的记忆条目。"
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "key": {
                    "type": "string",
                    "description": "要删除的记忆键名 (唯一标识)"
                }
            },
            "required": ["key"]
        })
    }

    fn timeout(&self) -> Option<Duration> {
        Some(Duration::from_secs(10))
    }

    /// 删除操作需要审批
    fn requires_approval(&self) -> bool {
        true
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let key = args
            .get("key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("缺少 key 参数"))?;

        match self.memory.forget(key).await {
            Ok(true) => Ok(ToolResult::ok(format!("已删除记忆: key='{key}'"))),
            Ok(false) => Ok(ToolResult::err(format!("未找到记忆: key='{key}'"))),
            Err(e) => Ok(ToolResult::err(format!("删除记忆失败: {e}"))),
        }
    }
}

// ── 单元测试 ──
#[cfg(test)]
mod tests {
    use super::*;
    use shadow_core::MemoryCategory;
    use shadow_memory::sqlite::SqliteMemory;

    /// 测试: 删除存在的记忆条目
    #[tokio::test]
    async fn test_forget_existing() {
        let dir = tempfile::tempdir().unwrap();
        let mem: Arc<dyn Memory> = Arc::new(SqliteMemory::new(dir.path()).unwrap());

        // 先存储一条
        mem.store("lang", "Rust", MemoryCategory::Core, None)
            .await
            .unwrap();

        let tool = MemoryForgetTool::new(Arc::clone(&mem));
        let result = tool
            .execute(json!({"key": "lang"}))
            .await
            .unwrap();

        assert!(result.success);
        assert!(result.output.contains("已删除记忆"));

        // 确认已被删除
        assert!(mem.get("lang").await.unwrap().is_none());
    }

    /// 测试: 删除不存在的 key 应返回错误
    #[tokio::test]
    async fn test_forget_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let mem: Arc<dyn Memory> = Arc::new(SqliteMemory::new(dir.path()).unwrap());

        let tool = MemoryForgetTool::new(mem);
        let result = tool
            .execute(json!({"key": "nonexistent"}))
            .await
            .unwrap();

        assert!(!result.success);
        assert!(result.error.unwrap().contains("未找到记忆"));
    }

    /// 测试: 缺少 key 参数应返回错误
    #[tokio::test]
    async fn test_forget_missing_key() {
        let dir = tempfile::tempdir().unwrap();
        let mem: Arc<dyn Memory> = Arc::new(SqliteMemory::new(dir.path()).unwrap());

        let tool = MemoryForgetTool::new(mem);
        let result = tool.execute(json!({})).await;

        assert!(result.is_err());
    }

    /// 测试: requires_approval 为 true
    #[test]
    fn test_requires_approval() {
        let dir = tempfile::tempdir().unwrap();
        let mem: Arc<dyn Memory> = Arc::new(SqliteMemory::new(dir.path()).unwrap());
        let tool = MemoryForgetTool::new(mem);
        assert!(tool.requires_approval());
    }
}
