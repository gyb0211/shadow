//! 文件读写工具集成测试

use shadow_tools::{FileReadTool, FileWriteTool};
use shadow_core::Tool;
use serde_json::json;

#[tokio::test]
async fn test_file_write_and_read() {
    let tmp = tempfile::tempdir().unwrap();
    let workspace = tmp.path();
    
    // 写入文件
    let write_tool = FileWriteTool::new(workspace);
    let write_args = json!({
        "path": "test.txt",
        "content": "line1\nline2\nline3"
    });
    let write_result = write_tool.execute(write_args).await.unwrap();
    assert!(write_result.success, "文件写入应该成功");
    
    // 读取验证
    let read_tool = FileReadTool::new(workspace);
    let read_args = json!({"path": "test.txt"});
    let read_result = read_tool.execute(read_args).await.unwrap();
    assert!(read_result.success, "文件读取应该成功");
    assert!(read_result.output.contains("line1"), "应该包含 line1");
    assert!(read_result.output.contains("3|line3"), "应该包含带行号的 line3");
}

#[tokio::test]
async fn test_file_read_with_pagination() {
    let tmp = tempfile::tempdir().unwrap();
    let content: String = (1..=20).map(|i| format!("line{}\n", i)).collect();
    tokio::fs::write(tmp.path().join("many.txt"), &content).await.unwrap();
    
    let tool = FileReadTool::new(tmp.path());
    let args = json!({
        "path": "many.txt",
        "offset": 10,
        "limit": 3
    });
    
    let result = tool.execute(args).await.unwrap();
    assert!(result.success);
    assert!(result.output.contains("10|line10"), "应该从第10行开始");
    assert!(result.output.contains("12|line12"), "应该到第12行结束");
    assert!(!result.output.contains("13|line13"), "不应该包含第13行");
}

#[tokio::test]
async fn test_file_read_nonexistent() {
    let tmp = tempfile::tempdir().unwrap();
    let tool = FileReadTool::new(tmp.path());
    let args = json!({"path": "nonexistent.txt"});
    
    let result = tool.execute(args).await.unwrap();
    assert!(!result.success, "不存在的文件应该失败");
    assert!(result.error.is_some(), "应该有错误信息");
}

#[tokio::test]
async fn test_file_write_creates_dirs() {
    let tmp = tempfile::tempdir().unwrap();
    let tool = FileWriteTool::new(tmp.path());
    let args = json!({
        "path": "deep/nested/file.txt",
        "content": "content"
    });
    
    let result = tool.execute(args).await.unwrap();
    assert!(result.success, "应该自动创建目录");
    
    // 验证文件存在
    let file_path = tmp.path().join("deep/nested/file.txt");
    assert!(file_path.exists(), "文件应该存在");
}