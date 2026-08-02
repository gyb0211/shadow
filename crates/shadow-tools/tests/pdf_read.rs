//! PDF 读取工具集成测试

use shadow_tools::PdfReadTool;
use shadow_core::Tool;
use serde_json::json;

#[tokio::test]
async fn test_pdf_read_missing_path() {
    let tool = PdfReadTool::new("/tmp");
    let args = json!({});
    let result = tool.execute(args).await.unwrap();
    assert!(!result.success);
}

#[tokio::test]
async fn test_pdf_read_nonexistent() {
    let tool = PdfReadTool::new("/tmp");
    let args = json!({"path": "/tmp/nonexistent_file_xyz123.pdf"});
    let result = tool.execute(args).await.unwrap();
    assert!(!result.success);
}

#[tokio::test]
async fn test_pdf_read_not_a_pdf() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    tokio::fs::write(tmp.path(), "this is not a pdf")
        .await
        .unwrap();

    let tool = PdfReadTool::new("/");
    let args = json!({"path": tmp.path().to_str().unwrap()});
    let result = tool.execute(args).await.unwrap();
    // 非 PDF 文件应该失败
    assert!(!result.success);
}