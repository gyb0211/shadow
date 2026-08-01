//! 文件编辑工具集成测试

use shadow_tools::{FileEditTool, FileWriteTool, FileReadTool};
use shadow_core::Tool;
use serde_json::json;

#[tokio::test]
async fn test_file_edit_single_occurrence() {
    let tmp = tempfile::tempdir().unwrap();
    let workspace = tmp.path();
    
    // 先写入文件
    let write_tool = FileWriteTool::new(workspace);
    write_tool.execute(json!({
        "path": "edit_test.txt",
        "content": "hello world\nfoo bar\nbaz qux"
    })).await.unwrap();
    
    // 编辑
    let edit_tool = FileEditTool::new(workspace);
    let args = json!({
        "path": "edit_test.txt",
        "old_string": "foo bar",
        "new_string": "FOO BAR"
    });
    
    let result = edit_tool.execute(args).await.unwrap();
    assert!(result.success, "单次替换应该成功");
    
    // 验证
    let read_tool = FileReadTool::new(workspace);
    let read_result = read_tool.execute(json!({"path": "edit_test.txt"})).await.unwrap();
    assert!(read_result.output.contains("FOO BAR"), "应该包含替换后的内容");
    assert!(!read_result.output.contains("foo bar"), "不应该包含旧内容");
}

#[tokio::test]
async fn test_file_edit_replace_all() {
    let tmp = tempfile::tempdir().unwrap();
    let workspace = tmp.path();
    
    // 写入有重复内容的文件
    let write_tool = FileWriteTool::new(workspace);
    write_tool.execute(json!({
        "path": "replace_all.txt",
        "content": "foo\nfoo\nfoo\nbar\nfoo"
    })).await.unwrap();
    
    // 替换所有 foo → FOO
    let edit_tool = FileEditTool::new(workspace);
    let args = json!({
        "path": "replace_all.txt",
        "old_string": "foo",
        "new_string": "FOO",
        "replace_all": true
    });
    
    let result = edit_tool.execute(args).await.unwrap();
    assert!(result.success, "替换所有应该成功");
    
    // 验证
    let read_tool = FileReadTool::new(workspace);
    let read_result = read_tool.execute(json!({"path": "replace_all.txt"})).await.unwrap();
    assert_eq!(read_result.output.matches("foo").count(), 0, "不应该有旧内容");
    assert_eq!(read_result.output.matches("FOO").count(), 4, "应该有4个新内容");
    assert!(read_result.output.contains("bar"), "bar 应该保持不变");
}

#[tokio::test]
async fn test_file_edit_no_match() {
    let tmp = tempfile::tempdir().unwrap();
    let workspace = tmp.path();
    
    let write_tool = FileWriteTool::new(workspace);
    write_tool.execute(json!({
        "path": "no_match.txt",
        "content": "hello world"
    })).await.unwrap();
    
    let edit_tool = FileEditTool::new(workspace);
    let args = json!({
        "path": "no_match.txt",
        "old_string": "not found",
        "new_string": "replacement"
    });
    
    let result = edit_tool.execute(args).await.unwrap();
    assert!(!result.success, "找不到匹配应该失败");
    assert!(result.error.is_some(), "应该有错误信息");
}