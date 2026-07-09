//! FileWrite 工具 -- 写入文件内容

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{Value, json};
use shadow_core::{Attributable, Tool, ToolResult, tool_attribution};

/// FileWrite 工具 -- 将内容写入指定路径的文件
///
/// 如果父目录不存在会自动创建. 默认覆盖已有文件, 支持 append 追加模式.
/// 写入采用原子方式: 先写入 .tmp 临时文件, 再 rename 到目标路径,
/// 避免写入过程中被中断导致文件损坏.
pub struct FileWriteTool;



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
        let append = args
            .get("append")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

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
    tokio::fs::rename(&tmp_path, target).await.map_err(|e| {
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
