//! FileWrite 工具 -- 写入文件内容

use shadow_core::{tool_attribution, Attributable, Tool, ToolResult};
use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

/// FileWrite 工具 -- 将内容写入指定路径的文件
///
/// 如果父目录不存在会自动创建. 默认覆盖已有文件, 支持 append 追加模式.
/// 写入采用原子方式: 先写入 .tmp 临时文件, 再 rename 到目标路径,
/// 避免写入过程中被中断导致文件损坏.
pub struct FileWriteTool;

impl Attributable for FileWriteTool {
    tool_attribution!("file_write");
}

#[async_trait]
impl Tool for FileWriteTool {
    fn name(&self) -> &str {
        "file_write"
    }

    fn description(&self) -> &str {
        "写入文件内容. 参数: path (文件路径), content (文件内容), \
         append (可选, true 时追加而非覆盖). 覆盖已有文件."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "要写入的文件路径"
                },
                "content": {
                    "type": "string",
                    "description": "要写入的文件内容"
                },
                "append": {
                    "type": "boolean",
                    "description": "是否追加模式 (默认 false, 覆盖)",
                    "default": false
                }
            },
            "required": ["path", "content"]
        })
    }

    /// FileWrite 工具需要审批 -- 会修改文件系统, 危险性较高
    fn requires_approval(&self) -> bool {
        true
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("缺少 path 参数"))?;

        let content = args
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("缺少 content 参数"))?;

        // 是否追加模式 (默认 false)
        let append = args.get("append").and_then(|v| v.as_bool()).unwrap_or(false);

        // 确保父目录存在
        let target = std::path::Path::new(path);
        if let Some(parent) = target.parent()
            && !parent.as_os_str().is_empty()
        {
            tokio::fs::create_dir_all(parent).await.ok();
        }

        if append {
            // 追加模式: 直接追加到文件末尾 (不需要原子写入)
            write_append(target, content).await?;
        } else {
            // 覆盖模式: 原子写入 -- 先写 .tmp 再 rename
            write_atomic(target, content).await?;
        }

        Ok(ToolResult::ok(format!(
            "已写入 {path} ({} 字节, {})",
            content.len(),
            if append { "追加" } else { "覆盖" }
        )))
    }
}

/// 原子写入 -- 先写入临时文件, 再 rename 到目标路径
///
/// 这样即使写入过程中崩溃, 目标文件也不会处于半写状态.
async fn write_atomic(target: &std::path::Path, content: &str) -> Result<()> {
    // 生成临时文件路径: 在目标路径后加 .tmp 后缀
    let tmp_path = {
        let mut s = target.to_string_lossy().into_owned();
        s.push_str(".tmp");
        std::path::PathBuf::from(s)
    };

    // 写入临时文件
    tokio::fs::write(&tmp_path, content)
        .await
        .map_err(|e| anyhow::anyhow!("写入临时文件失败 '{tmp_path:?}': {e}"))?;

    // 原子 rename 到目标路径
    tokio::fs::rename(&tmp_path, target)
        .await
        .map_err(|e| {
            // rename 失败时清理临时文件
            let tmp_clone = tmp_path.clone();
            tokio::spawn(async move {
                tokio::fs::remove_file(&tmp_clone).await.ok();
            });
            anyhow::anyhow!("重命名文件失败 '{target:?}': {e}")
        })?;

    Ok(())
}

/// 追加写入 -- 读取已有内容, 拼接后整体写入
///
/// 追加模式不需要原子写入, 直接读取+拼接+写入即可.
async fn write_append(target: &std::path::Path, content: &str) -> Result<()> {
    // 读取已有内容 (文件不存在时视为空)
    let existing = tokio::fs::read_to_string(target).await.unwrap_or_default();

    // 拼接后整体写入
    let combined = format!("{existing}{content}");
    tokio::fs::write(target, &combined)
        .await
        .map_err(|e| anyhow::anyhow!("追加写入失败 '{target:?}': {e}"))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn write_and_verify() {
        let tool = FileWriteTool;
        let path = "/tmp/shadow_test_filewrite.txt";

        // 写入
        let result = tool
            .execute(json!({"path": path, "content": "hello shadow"}))
            .await
            .unwrap();
        assert!(result.success);

        // 验证
        let content = tokio::fs::read_to_string(path).await.unwrap();
        assert_eq!(content, "hello shadow");

        // 清理
        tokio::fs::remove_file(path).await.ok();
    }

    #[tokio::test]
    async fn write_append_mode() {
        let tool = FileWriteTool;
        let path = "/tmp/shadow_test_filewrite_append.txt";

        // 第一次写入
        tool.execute(json!({"path": path, "content": "first\n"}))
            .await
            .unwrap();

        // 追加写入
        let result = tool
            .execute(json!({"path": path, "content": "second\n", "append": true}))
            .await
            .unwrap();
        assert!(result.success);
        assert!(result.output.contains("追加"));

        // 验证两段内容都在
        let content = tokio::fs::read_to_string(path).await.unwrap();
        assert_eq!(content, "first\nsecond\n");

        // 清理
        tokio::fs::remove_file(path).await.ok();
    }

    #[tokio::test]
    async fn write_overwrite_replaces_content() {
        let tool = FileWriteTool;
        let path = "/tmp/shadow_test_filewrite_overwrite.txt";

        // 第一次写入
        tool.execute(json!({"path": path, "content": "old content"}))
            .await
            .unwrap();

        // 覆盖写入 (不传 append, 默认 false)
        tool.execute(json!({"path": path, "content": "new content"}))
            .await
            .unwrap();

        let content = tokio::fs::read_to_string(path).await.unwrap();
        assert_eq!(content, "new content");

        // 清理
        tokio::fs::remove_file(path).await.ok();
    }

    #[tokio::test]
    async fn write_creates_parent_dirs() {
        let tool = FileWriteTool;
        let path = "/tmp/shadow_test_dir/sub/file.txt";

        let result = tool
            .execute(json!({"path": path, "content": "nested"}))
            .await
            .unwrap();
        assert!(result.success);

        let content = tokio::fs::read_to_string(path).await.unwrap();
        assert_eq!(content, "nested");

        // 清理
        tokio::fs::remove_file(path).await.ok();
        tokio::fs::remove_dir_all("/tmp/shadow_test_dir").await.ok();
    }

    #[test]
    fn file_write_requires_approval() {
        let tool = FileWriteTool;
        assert!(tool.requires_approval());
    }
}
