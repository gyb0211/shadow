//! FileEdit 工具 -- patch 风格的文件编辑
//!
//! 不像 FileWriteTool 那样全文覆盖, 而是精确替换文件中的某段文本.
//! 适用于对已有文件做局部修改, 避免覆盖整个文件内容.

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{Value, json};
use shadow_core::{Attributable, Tool, ToolResult, tool_attribution};
use std::time::Duration;

/// FileEdit 工具 -- 精确替换文件中的文本片段
///
/// 行为:
/// - `old_text` 必须在文件中唯一匹配 (除非 `replace_all = true`)
/// - 如果 `old_text` 不存在, 返回错误
/// - 如果 `old_text` 不唯一且 `replace_all = false`, 返回错误要求提供更多上下文
/// - 成功后返回 diff 风格的输出
///
/// 这是一个敏感操作 (修改文件), 需要 approval.
pub struct FileEditTool;



#[async_trait]
impl Tool for FileEditTool {
    fn name(&self) -> &str {
        "file_edit"
    }

    fn description(&self) -> &str {
        "精确替换文件中的文本片段. 参数: path (文件路径), old_text (要替换的文本), \
         new_text (替换后的文本), replace_all (可选, true 时替换所有匹配). \
         old_text 必须在文件中唯一匹配, 否则需要提供更多上下文或设置 replace_all=true."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "要编辑的文件路径"
                },
                "old_text": {
                    "type": "string",
                    "description": "要被替换的文本 (必须在文件中存在)"
                },
                "new_text": {
                    "type": "string",
                    "description": "替换后的新文本"
                },
                "replace_all": {
                    "type": "boolean",
                    "description": "是否替换所有匹配 (默认 false, 要求 old_text 唯一)",
                    "default": false
                }
            },
            "required": ["path", "old_text", "new_text"]
        })
    }

    /// FileEdit 工具需要审批 -- 会修改文件, 是敏感操作
    fn requires_approval(&self) -> bool {
        true
    }

    /// 超时 10 秒 -- 文件编辑应快速完成
    fn timeout(&self) -> Option<Duration> {
        Some(Duration::from_secs(10))
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        // 解析参数
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("缺少 path 参数"))?;

        let old_text = args
            .get("old_text")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("缺少 old_text 参数"))?;

        let new_text = args
            .get("new_text")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("缺少 new_text 参数"))?;

        // replace_all 默认 false
        let replace_all = args
            .get("replace_all")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // old_text 不能为空 -- 空字符串匹配所有位置, 没有意义
        if old_text.is_empty() {
            return Ok(ToolResult::err(
                "old_text 不能为空字符串 -- 请提供要替换的实际文本内容",
            ));
        }

        // 读取文件内容
        let content = match tokio::fs::read_to_string(path).await {
            Ok(c) => c,
            Err(e) => {
                return Ok(ToolResult::err(format!("读取文件失败 '{path}': {e}")));
            }
        };

        // 统计 old_text 在文件中的匹配次数
        let match_count = content.matches(old_text).count();

        // 情况 1: 没有匹配
        if match_count == 0 {
            return Ok(ToolResult::err(format!(
                "在文件 '{path}' 中未找到要替换的文本. \
                 请检查 old_text 是否正确 (注意空格、换行等不可见字符)"
            )));
        }

        // 情况 2: 多处匹配但未启用 replace_all -- 要求提供更多上下文
        if match_count > 1 && !replace_all {
            return Ok(ToolResult::err(format!(
                "old_text 在文件中匹配了 {match_count} 处, 不是唯一匹配. \
                 请提供更多上下文使 old_text 唯一, 或设置 replace_all=true 替换所有匹配"
            )));
        }

        // 执行替换
        let new_content = if replace_all {
            content.replace(old_text, new_text)
        } else {
            // 唯一匹配, 替换一次
            content.replacen(old_text, new_text, 1)
        };

        // 原子写入 -- 先写临时文件再 rename, 避免写入中断导致文件损坏
        let target = std::path::Path::new(path);
        if let Err(e) = write_atomic(target, &new_content).await {
            return Ok(ToolResult::err(format!("写入文件失败 '{path}': {e}")));
        }

        // 生成 diff 风格的输出
        let diff = generate_diff(old_text, new_text, replace_all, match_count);

        Ok(ToolResult::ok(format!(
            "已编辑 {path} ({match_count} 处替换)\n\n{diff}"
        )))
    }
}

/// 原子写入 -- 先写入临时文件, 再 rename 到目标路径
///
/// 与 FileWriteTool 的 write_atomic 逻辑一致, 避免写入过程中崩溃导致文件损坏.
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

/// 生成 diff 风格的输出
///
/// 以 `-` 开头表示删除的行, `+` 开头表示新增的行.
fn generate_diff(old_text: &str, new_text: &str, replace_all: bool, match_count: usize) -> String {
    let mut diff = String::new();
    diff.push_str("--- 原文件\n");
    diff.push_str("+++ 修改后\n");

    // 按行拆分 old_text 和 new_text, 逐行对比
    let old_lines: Vec<&str> = old_text.lines().collect();
    let new_lines: Vec<&str> = new_text.lines().collect();

    // 简单的逐行 diff: 标记删除的旧行和新增的新行
    for line in &old_lines {
        diff.push_str(&format!("- {line}\n"));
    }
    for line in &new_lines {
        diff.push_str(&format!("+ {line}\n"));
    }

    if replace_all && match_count > 1 {
        diff.push_str(&format!(
            "\n(以上 diff 展示单处替换, 共 {match_count} 处相同替换)"
        ));
    }

    diff
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 辅助函数: 创建临时文件并写入初始内容
    async fn setup_test_file(path: &str, content: &str) {
        tokio::fs::write(path, content).await.unwrap();
    }

    /// 辅助函数: 读取文件内容
    async fn read_file(path: &str) -> String {
        tokio::fs::read_to_string(path).await.unwrap()
    }

    /// 辅助函数: 清理临时文件
    async fn cleanup(path: &str) {
        tokio::fs::remove_file(path).await.ok();
    }

    #[tokio::test]
    async fn edit_normal_replacement() {
        // 测试: 正常唯一替换
        let tool = FileEditTool;
        let path = "/tmp/shadow_test_fileedit_normal.txt";

        setup_test_file(path, "hello world\nfoo bar\n").await;

        let result = tool
            .execute(json!({
                "path": path,
                "old_text": "hello world",
                "new_text": "hello shadow"
            }))
            .await
            .unwrap();

        assert!(result.success);
        assert!(result.output.contains("已编辑"));
        assert!(result.output.contains("- hello world"));
        assert!(result.output.contains("+ hello shadow"));

        let content = read_file(path).await;
        assert_eq!(content, "hello shadow\nfoo bar\n");

        cleanup(path).await;
    }

    #[tokio::test]
    async fn edit_no_match() {
        // 测试: old_text 在文件中不存在, 应返回错误
        let tool = FileEditTool;
        let path = "/tmp/shadow_test_fileedit_nomatch.txt";

        setup_test_file(path, "hello world\n").await;

        let result = tool
            .execute(json!({
                "path": path,
                "old_text": "nonexistent text",
                "new_text": "replacement"
            }))
            .await
            .unwrap();

        assert!(!result.success);
        assert!(result.error.unwrap().contains("未找到"));

        // 文件内容应未被修改
        let content = read_file(path).await;
        assert_eq!(content, "hello world\n");

        cleanup(path).await;
    }

    #[tokio::test]
    async fn edit_multiple_matches_without_replace_all() {
        // 测试: 多处匹配但 replace_all=false, 应返回错误要求更多上下文
        let tool = FileEditTool;
        let path = "/tmp/shadow_test_fileedit_multi.txt";

        setup_test_file(path, "foo bar\nfoo baz\nfoo qux\n").await;

        let result = tool
            .execute(json!({
                "path": path,
                "old_text": "foo",
                "new_text": "FOO"
            }))
            .await
            .unwrap();

        assert!(!result.success);
        let err = result.error.unwrap();
        assert!(err.contains("3 处"));
        assert!(err.contains("唯一匹配"));
        assert!(err.contains("replace_all"));

        // 文件内容应未被修改
        let content = read_file(path).await;
        assert_eq!(content, "foo bar\nfoo baz\nfoo qux\n");

        cleanup(path).await;
    }

    #[tokio::test]
    async fn edit_replace_all_multiple_matches() {
        // 测试: 多处匹配 + replace_all=true, 应全部替换
        let tool = FileEditTool;
        let path = "/tmp/shadow_test_fileedit_replaceall.txt";

        setup_test_file(path, "foo bar\nfoo baz\nfoo qux\n").await;

        let result = tool
            .execute(json!({
                "path": path,
                "old_text": "foo",
                "new_text": "FOO",
                "replace_all": true
            }))
            .await
            .unwrap();

        assert!(result.success);
        assert!(result.output.contains("3 处替换"));

        let content = read_file(path).await;
        assert_eq!(content, "FOO bar\nFOO baz\nFOO qux\n");

        cleanup(path).await;
    }

    #[tokio::test]
    async fn edit_replace_multiline_text() {
        // 测试: 替换多行文本
        let tool = FileEditTool;
        let path = "/tmp/shadow_test_fileedit_multiline.txt";

        let original = "fn main() {\n    println!(\"hello\");\n}\n";
        setup_test_file(path, original).await;

        let old_text = "    println!(\"hello\");";
        let new_text = "    println!(\"world\");\n    println!(\"shadow\");";

        let result = tool
            .execute(json!({
                "path": path,
                "old_text": old_text,
                "new_text": new_text
            }))
            .await
            .unwrap();

        assert!(result.success);

        let content = read_file(path).await;
        assert_eq!(
            content,
            "fn main() {\n    println!(\"world\");\n    println!(\"shadow\");\n}\n"
        );

        cleanup(path).await;
    }

    #[tokio::test]
    async fn edit_empty_old_text_rejected() {
        // 测试: old_text 为空字符串应被拒绝
        let tool = FileEditTool;
        let path = "/tmp/shadow_test_fileedit_empty.txt";

        setup_test_file(path, "some content\n").await;

        let result = tool
            .execute(json!({
                "path": path,
                "old_text": "",
                "new_text": "replacement"
            }))
            .await
            .unwrap();

        assert!(!result.success);
        assert!(result.error.unwrap().contains("不能为空"));

        cleanup(path).await;
    }

    #[tokio::test]
    async fn edit_nonexistent_file() {
        // 测试: 编辑不存在的文件应返回错误
        let tool = FileEditTool;
        let path = "/tmp/shadow_test_fileedit_nonexistent.txt";

        // 确保文件不存在
        cleanup(path).await;

        let result = tool
            .execute(json!({
                "path": path,
                "old_text": "something",
                "new_text": "other"
            }))
            .await
            .unwrap();

        assert!(!result.success);
        assert!(result.error.unwrap().contains("读取文件失败"));
    }

    #[tokio::test]
    async fn edit_unique_match_with_replace_all() {
        // 测试: 唯一匹配时 replace_all=true 也能正常工作
        let tool = FileEditTool;
        let path = "/tmp/shadow_test_fileedit_unique_replaceall.txt";

        setup_test_file(path, "only one match here\n").await;

        let result = tool
            .execute(json!({
                "path": path,
                "old_text": "only one",
                "new_text": "just one",
                "replace_all": true
            }))
            .await
            .unwrap();

        assert!(result.success);

        let content = read_file(path).await;
        assert_eq!(content, "just one match here\n");

        cleanup(path).await;
    }

    #[tokio::test]
    async fn edit_replacement_to_empty_string() {
        // 测试: 替换为空字符串 (删除文本)
        let tool = FileEditTool;
        let path = "/tmp/shadow_test_fileedit_delete.txt";

        setup_test_file(path, "hello world cruel\n").await;

        let result = tool
            .execute(json!({
                "path": path,
                "old_text": " cruel",
                "new_text": ""
            }))
            .await
            .unwrap();

        assert!(result.success);

        let content = read_file(path).await;
        assert_eq!(content, "hello world\n");

        cleanup(path).await;
    }

    #[tokio::test]
    async fn edit_preserves_other_content() {
        // 测试: 只替换目标文本, 其他内容保持不变
        let tool = FileEditTool;
        let path = "/tmp/shadow_test_fileedit_preserve.txt";

        let original = "line1\nline2\nline3\nline4\nline5\n";
        setup_test_file(path, original).await;

        let result = tool
            .execute(json!({
                "path": path,
                "old_text": "line3",
                "new_text": "LINE3"
            }))
            .await
            .unwrap();

        assert!(result.success);

        let content = read_file(path).await;
        assert_eq!(content, "line1\nline2\nLINE3\nline4\nline5\n");

        cleanup(path).await;
    }

    #[test]
    fn file_edit_requires_approval() {
        // 测试: FileEdit 工具需要审批
        let tool = FileEditTool;
        assert!(tool.requires_approval());
    }

    #[test]
    fn file_edit_timeout_is_10_seconds() {
        // 测试: 超时为 10 秒
        let tool = FileEditTool;
        assert_eq!(tool.timeout(), Some(Duration::from_secs(10)));
    }

    #[test]
    fn file_edit_name_and_schema() {
        // 测试: 工具名称和 schema 基本结构
        let tool = FileEditTool;
        assert_eq!(tool.name(), "file_edit");
        assert!(!tool.description().is_empty());

        let schema = tool.parameters_schema();
        let props = schema.get("properties").unwrap().as_object().unwrap();
        assert!(props.contains_key("path"));
        assert!(props.contains_key("old_text"));
        assert!(props.contains_key("new_text"));
        assert!(props.contains_key("replace_all"));

        let required = schema.get("required").unwrap().as_array().unwrap();
        assert!(required.contains(&json!("path")));
        assert!(required.contains(&json!("old_text")));
        assert!(required.contains(&json!("new_text")));
    }
}
