//! 搜索工具集成测试

use shadow_tools::{GlobSearchTool, ContentSearchTool, FileWriteTool};
use shadow_core::Tool;
use serde_json::json;

#[tokio::test]
async fn test_glob_search() {
    let tmp = tempfile::tempdir().unwrap();
    let workspace = tmp.path();
    
    // 创建测试文件
    let write_tool = FileWriteTool::new(workspace);
    write_tool.execute(json!({"path": "a.rs", "content": "fn main() {}"})).await.unwrap();
    write_tool.execute(json!({"path": "b.py", "content": "print('hello')"})).await.unwrap();
    write_tool.execute(json!({"path": "sub/c.rs", "content": "mod test {}"})).await.unwrap();
    write_tool.execute(json!({"path": "test.txt", "content": "text file"})).await.unwrap();
    
    // 搜索所有 .rs 文件
    let tool = GlobSearchTool::new(workspace);
    let args = json!({"pattern": "**/*.rs"});
    let result = tool.execute(args).await.unwrap();
    assert!(result.success, "glob 搜索应该成功");
    assert!(result.output.contains("a.rs"), "应该找到 a.rs");
    assert!(result.output.contains("c.rs"), "应该找到 c.rs");
    assert!(!result.output.contains("b.py"), "不应该找到 b.py");
}

#[tokio::test]
async fn test_content_search() {
    let tmp = tempfile::tempdir().unwrap();
    let workspace = tmp.path();
    
    // 创建测试文件
    let write_tool = FileWriteTool::new(workspace);
    write_tool.execute(json!({"path": "code.rs", "content": "fn test() {\n    assert_eq(1, 1);\n}"})).await.unwrap();
    write_tool.execute(json!({"path": "other.txt", "content": "just some text"})).await.unwrap();
    
    // 搜索包含 "assert" 的行
    let tool = ContentSearchTool::new(workspace);
    let args = json!({"pattern": "assert"});
    let result = tool.execute(args).await.unwrap();
    assert!(result.success, "内容搜索应该成功");
    assert!(result.output.contains("code.rs"), "应该包含文件名");
    assert!(result.output.contains("assert_eq"), "应该包含匹配的行");
}

#[tokio::test]
async fn test_content_search_with_glob() {
    let tmp = tempfile::tempdir().unwrap();
    let workspace = tmp.path();
    
    // 创建测试文件
    let write_tool = FileWriteTool::new(workspace);
    write_tool.execute(json!({"path": "main.rs", "content": "fn main() {}"})).await.unwrap();
    write_tool.execute(json!({"path": "test.txt", "content": "fn helper() {}"})).await.unwrap();
    
    // 只在 .rs 文件中搜索 "fn"
    let tool = ContentSearchTool::new(workspace);
    let args = json!({
        "pattern": "fn",
        "file_glob": "*.rs"
    });
    let result = tool.execute(args).await.unwrap();
    assert!(result.success);
    assert!(result.output.contains("main.rs"), "应该在 main.rs 中找到");
    assert!(!result.output.contains("test.txt"), "不应该在 .txt 文件中查找");
}

#[tokio::test]
async fn test_glob_no_results() {
    let tmp = tempfile::tempdir().unwrap();
    let tool = GlobSearchTool::new(tmp.path());
    let args = json!({"pattern": "*.nonexistent"});
    
    let result = tool.execute(args).await.unwrap();
    assert!(result.success, "无结果也应该返回成功");
    assert!(result.output.contains("no files") || result.output.contains("No files"), 
            "应该提示没有文件");
}