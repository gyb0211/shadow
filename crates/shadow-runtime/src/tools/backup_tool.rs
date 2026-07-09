//! 文件备份工具 -- 创建文件或目录的备份副本
//!
//! 文件: 复制为 path.suffix (默认 .bak)
//! 目录: tar.gz 压缩
//! 支持自动轮转 (max_backups)

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{Value, json};
use std::path::Path;
use std::time::Duration;

use shadow_core::{Attributable, Role, Tool, ToolResult, ToolSpec};

/// 文件备份工具
pub struct BackupTool;

impl BackupTool {
    pub fn new() -> Self {
        Self
    }

    /// 获取带序号的备份文件路径列表 (按时间排序, 旧的在前)
    fn list_backups(path: &Path, suffix: &str) -> Vec<std::path::PathBuf> {
        let parent = path.parent().unwrap_or(Path::new("."));
        let stem = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

        let mut backups = Vec::new();
        if let Ok(entries) = std::fs::read_dir(parent) {
            for entry in entries.flatten() {
                if let Some(name) = entry.file_name().to_str() {
                    // 匹配 path.bak, path.bak.1, path.bak.2 等
                    if name.starts_with(&format!("{stem}{suffix}")) {
                        backups.push(entry.path());
                    }
                }
            }
        }
        backups.sort();
        backups
    }

    /// 清理多余的备份文件, 保留最新的 max_backups 个
    fn rotate_backups(path: &Path, suffix: &str, max_backups: usize) -> usize {
        let mut backups = Self::list_backups(path, suffix);
        if backups.len() <= max_backups {
            return 0;
        }

        let to_remove = backups.len() - max_backups;
        for backup in backups.drain(..to_remove) {
            let _ = std::fs::remove_file(&backup);
        }
        to_remove
    }
}

impl Default for BackupTool {
    fn default() -> Self {
        Self::new()
    }
}



#[async_trait]
impl Tool for BackupTool {
    fn name(&self) -> &str {
        "backup"
    }

    fn description(&self) -> &str {
        "创建文件或目录的备份。文件复制为 .bak, 目录压缩为 tar.gz。支持自动轮转。"
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "要备份的文件或目录路径"
                },
                "suffix": {
                    "type": "string",
                    "description": "备份后缀 (默认 .bak)",
                    "default": ".bak"
                },
                "max_backups": {
                    "type": "integer",
                    "description": "最大保留备份数 (默认 5, 超出删除最旧的)",
                    "default": 5
                }
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let path_str = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("缺少 path 参数"))?;

        let suffix = args
            .get("suffix")
            .and_then(|v| v.as_str())
            .unwrap_or(".bak");

        let max_backups = args
            .get("max_backups")
            .and_then(|v| v.as_u64())
            .unwrap_or(5) as usize;

        let path = Path::new(path_str);

        // 检查路径存在
        if !path.exists() {
            return Ok(ToolResult::err(format!("路径不存在: {path_str}")));
        }

        if path.is_file() {
            // 文件备份: 复制
            let backup_path = format!("{path_str}{suffix}");

            match tokio::fs::copy(path, &backup_path).await {
                Ok(bytes) => {
                    // 轮转旧备份
                    let removed = Self::rotate_backups(path, suffix, max_backups);

                    let mut output = format!(
                        "备份完成: {} -> {} ({} bytes)",
                        path_str, backup_path, bytes
                    );
                    if removed > 0 {
                        output.push_str(&format!("\n已清理 {removed} 个旧备份"));
                    }
                    Ok(ToolResult::ok(output))
                }
                Err(e) => Ok(ToolResult::err(format!("备份失败: {e}"))),
            }
        } else if path.is_dir() {
            // 目录备份: tar.gz 压缩
            let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
            let dir_name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("backup");
            let backup_path = format!("{path_str}_{timestamp}.tar.gz");

            // 调用 tar 命令压缩
            let output = tokio::process::Command::new("tar")
                .arg("czf")
                .arg(&backup_path)
                .arg("-C")
                .arg(path.parent().unwrap_or(Path::new(".")))
                .arg(dir_name)
                .output()
                .await;

            match output {
                Ok(out) if out.status.success() => {
                    // 获取备份文件大小
                    let size = std::fs::metadata(&backup_path)
                        .map(|m| m.len())
                        .unwrap_or(0);

                    // 轮转旧备份 (匹配 _*.tar.gz)
                    let parent = path.parent().unwrap_or(Path::new("."));
                    let pattern = format!("{}_", dir_name);
                    let mut tar_backups: Vec<_> = std::fs::read_dir(parent)
                        .into_iter()
                        .flatten()
                        .flatten()
                        .filter(|e| {
                            e.file_name()
                                .to_str()
                                .map(|n| n.starts_with(&pattern) && n.ends_with(".tar.gz"))
                                .unwrap_or(false)
                        })
                        .map(|e| e.path())
                        .collect();
                    tar_backups.sort();

                    let mut removed_count = 0;
                    if tar_backups.len() > max_backups {
                        let to_remove = tar_backups.len() - max_backups;
                        for backup in tar_backups.drain(..to_remove) {
                            let _ = std::fs::remove_file(&backup);
                        }
                        removed_count = to_remove;
                    }

                    let mut msg = format!(
                        "目录备份完成: {} -> {} ({} bytes)",
                        path_str, backup_path, size
                    );
                    if removed_count > 0 {
                        msg.push_str(&format!("\n已清理 {removed_count} 个旧备份"));
                    }
                    Ok(ToolResult::ok(msg))
                }
                Ok(out) => {
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    Ok(ToolResult::err(format!("tar 压缩失败: {stderr}")))
                }
                Err(e) => Ok(ToolResult::err(format!("执行 tar 失败: {e}"))),
            }
        } else {
            Ok(ToolResult::err(format!("不支持的路径类型: {path_str}")))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[tokio::test]
    async fn test_backup_file() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        let mut f = std::fs::File::create(&file_path).unwrap();
        f.write_all(b"hello backup").unwrap();

        let tool = BackupTool::new();
        let result = tool
            .execute(json!({
                "path": file_path.to_str().unwrap(),
                "suffix": ".bak"
            }))
            .await
            .unwrap();

        assert!(result.success);
        assert!(result.output.contains("备份完成"));

        let backup_path = format!("{}.bak", file_path.to_str().unwrap());
        assert!(Path::new(&backup_path).exists());

        // 验证内容
        let content = std::fs::read_to_string(&backup_path).unwrap();
        assert_eq!(content, "hello backup");
    }

    #[tokio::test]
    async fn test_backup_nonexistent() {
        let tool = BackupTool::new();
        let result = tool
            .execute(json!({
                "path": "/nonexistent/path/file.txt"
            }))
            .await
            .unwrap();

        assert!(!result.success);
        assert!(result.error.unwrap().contains("不存在"));
    }

    #[tokio::test]
    async fn test_backup_with_custom_suffix() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("data.txt");
        std::fs::write(&file_path, "data").unwrap();

        let tool = BackupTool::new();
        let result = tool
            .execute(json!({
                "path": file_path.to_str().unwrap(),
                "suffix": ".old"
            }))
            .await
            .unwrap();

        assert!(result.success);
        let backup_path = format!("{}.old", file_path.to_str().unwrap());
        assert!(Path::new(&backup_path).exists());
    }

    #[tokio::test]
    async fn test_backup_overwrite_existing() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("config.txt");
        std::fs::write(&file_path, "new content").unwrap();

        // 先创建一个旧备份
        let old_backup = format!("{}.bak", file_path.to_str().unwrap());
        std::fs::write(&old_backup, "old content").unwrap();

        let tool = BackupTool::new();
        let result = tool
            .execute(json!({
                "path": file_path.to_str().unwrap(),
                "suffix": ".bak"
            }))
            .await
            .unwrap();

        assert!(result.success);
        // 新备份应该覆盖旧的
        let content = std::fs::read_to_string(&old_backup).unwrap();
        assert_eq!(content, "new content");
    }

    #[test]
    fn test_tool_metadata() {
        let tool = BackupTool::new();
        assert_eq!(tool.name(), "backup");
        assert!(!tool.description().is_empty());
        assert_eq!(tool.timeout(), Some(Duration::from_secs(60)));
    }

    #[test]
    fn test_schema() {
        let tool = BackupTool::new();
        let schema = tool.parameters_schema();
        assert!(schema["properties"].get("path").is_some());
        assert!(schema["properties"].get("suffix").is_some());
        assert!(schema["properties"].get("max_backups").is_some());
    }
}
