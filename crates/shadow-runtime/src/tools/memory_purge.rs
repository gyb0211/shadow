//! MemoryPurge 工具 -- 批量清除记忆

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;

use shadow_core::{Attributable, Memory, MemoryCategory, Tool, ToolResult, tool_attribution};

/// MemoryPurge 工具 -- 批量清除指定分类的记忆
///
/// 支持按 category 过滤后批量删除, 也可不指定 category 删除全部.
/// 出于安全考虑, 必须传入 confirm=true 才会执行删除.
/// requires_approval = true.
pub struct MemoryPurgeTool {
    memory: Arc<dyn Memory>,
}

impl MemoryPurgeTool {
    /// 创建 MemoryPurgeTool, 需要传入 Memory 后端
    pub fn new(memory: Arc<dyn Memory>) -> Self {
        Self { memory }
    }
}



#[async_trait]
impl Tool for MemoryPurgeTool {
    fn name(&self) -> &str {
        "memory_purge"
    }

    fn description(&self) -> &str {
        "批量清除记忆。可按分类过滤, 必须设置 confirm=true 才会执行。"
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "category": {
                    "type": "string",
                    "description": "要清除的记忆分类 (如 core/daily/conversation). 不指定则清除全部."
                },
                "confirm": {
                    "type": "boolean",
                    "description": "安全确认, 必须为 true 才执行删除操作",
                    "default": false
                }
            },
            "required": ["confirm"]
        })
    }

    fn timeout(&self) -> Option<Duration> {
        Some(Duration::from_secs(30))
    }

    /// 批量删除操作需要审批
    fn requires_approval(&self) -> bool {
        true
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        // 安全确认检查
        let confirm = args
            .get("confirm")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if !confirm {
            return Ok(ToolResult::err(
                "安全确认未通过: 请设置 confirm=true 以确认批量删除操作",
            ));
        }

        // 解析可选的 category 过滤
        let category = args.get("category").and_then(|v| v.as_str());
        let category_filter = category.map(MemoryCategory::from_name);

        // 列出待删除的条目
        let entries = match self.memory.list(category_filter.as_ref()).await {
            Ok(e) => e,
            Err(e) => {
                return Ok(ToolResult::err(format!("列出记忆失败: {e}")));
            }
        };

        let total = entries.len();
        if total == 0 {
            let scope = category.unwrap_or("全部");
            return Ok(ToolResult::ok(format!(
                "没有需要清除的记忆 (category={scope})"
            )));
        }

        // 逐个删除
        let mut deleted = 0usize;
        let mut errors = Vec::new();
        for entry in &entries {
            match self.memory.forget(&entry.key).await {
                Ok(true) => deleted += 1,
                Ok(false) => {} // key 不存在, 跳过
                Err(e) => errors.push(format!("key='{}': {e}", entry.key)),
            }
        }

        let scope = category.unwrap_or("全部");
        if errors.is_empty() {
            Ok(ToolResult::ok(format!(
                "已清除 {deleted}/{total} 条记忆 (category={scope})"
            )))
        } else {
            Ok(ToolResult::ok(format!(
                "已清除 {deleted}/{total} 条记忆 (category={scope}), 失败 {} 条: {}",
                errors.len(),
                errors.join("; ")
            )))
        }
    }
}

// ── 单元测试 ──
#[cfg(test)]
mod tests {
    use super::*;
    use shadow_memory::sqlite::SqliteMemory;

    /// 测试: 未确认时应拒绝执行
    #[tokio::test]
    async fn test_purge_without_confirm() {
        let dir = tempfile::tempdir().unwrap();
        let mem: Arc<dyn Memory> = Arc::new(SqliteMemory::new(dir.path()).unwrap());

        mem.store("a", "内容A", MemoryCategory::Core, None)
            .await
            .unwrap();

        let tool = MemoryPurgeTool::new(mem);
        let result = tool.execute(json!({"confirm": false})).await.unwrap();

        assert!(!result.success);
        assert!(result.error.unwrap().contains("安全确认"));
    }

    /// 测试: 确认后清除指定 category
    #[tokio::test]
    async fn test_purge_by_category() {
        let dir = tempfile::tempdir().unwrap();
        let mem: Arc<dyn Memory> = Arc::new(SqliteMemory::new(dir.path()).unwrap());

        mem.store("a", "内容A", MemoryCategory::Core, None)
            .await
            .unwrap();
        mem.store("b", "内容B", MemoryCategory::Daily, None)
            .await
            .unwrap();
        mem.store("c", "内容C", MemoryCategory::Core, None)
            .await
            .unwrap();

        let tool = MemoryPurgeTool::new(Arc::clone(&mem));
        let result = tool
            .execute(json!({"category": "core", "confirm": true}))
            .await
            .unwrap();

        assert!(result.success);
        assert!(result.output.contains("2/2"));

        // core 应被清空, daily 应保留
        let core_entries = mem.list(Some(&MemoryCategory::Core)).await.unwrap();
        assert!(core_entries.is_empty());

        let daily_entries = mem.list(Some(&MemoryCategory::Daily)).await.unwrap();
        assert_eq!(daily_entries.len(), 1);
    }

    /// 测试: 不指定 category 时清除全部
    #[tokio::test]
    async fn test_purge_all() {
        let dir = tempfile::tempdir().unwrap();
        let mem: Arc<dyn Memory> = Arc::new(SqliteMemory::new(dir.path()).unwrap());

        mem.store("a", "内容A", MemoryCategory::Core, None)
            .await
            .unwrap();
        mem.store("b", "内容B", MemoryCategory::Daily, None)
            .await
            .unwrap();

        let tool = MemoryPurgeTool::new(Arc::clone(&mem));
        let result = tool.execute(json!({"confirm": true})).await.unwrap();

        assert!(result.success);
        assert!(result.output.contains("2/2"));
        assert_eq!(mem.count().await.unwrap(), 0);
    }

    /// 测试: 清除空记忆库
    #[tokio::test]
    async fn test_purge_empty() {
        let dir = tempfile::tempdir().unwrap();
        let mem: Arc<dyn Memory> = Arc::new(SqliteMemory::new(dir.path()).unwrap());

        let tool = MemoryPurgeTool::new(mem);
        let result = tool.execute(json!({"confirm": true})).await.unwrap();

        assert!(result.success);
        assert!(result.output.contains("没有需要清除"));
    }

    /// 测试: requires_approval 为 true
    #[test]
    fn test_requires_approval() {
        let dir = tempfile::tempdir().unwrap();
        let mem: Arc<dyn Memory> = Arc::new(SqliteMemory::new(dir.path()).unwrap());
        let tool = MemoryPurgeTool::new(mem);
        assert!(tool.requires_approval());
    }
}
