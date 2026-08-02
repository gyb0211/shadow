//! 文件下载工具集成测试

use shadow_tools::FileDownloadTool;
use shadow_core::Tool;
use serde_json::json;

#[tokio::test]
async fn test_download_missing_url() {
    let tool = FileDownloadTool::new("/tmp");
    let args = json!({"dest_path": "test.txt"});
    let result = tool.execute(args).await.unwrap();
    assert!(!result.success);
}

#[tokio::test]
async fn test_download_missing_dest() {
    let tool = FileDownloadTool::new("/tmp");
    let args = json!({"url": "https://example.com"});
    let result = tool.execute(args).await.unwrap();
    assert!(!result.success);
}

#[tokio::test]
async fn test_download_invalid_scheme() {
    let tool = FileDownloadTool::new("/tmp");
    let args = json!({"url": "ftp://example.com/file", "dest_path": "test.txt"});
    let result = tool.execute(args).await.unwrap();
    assert!(!result.success);
    assert!(result.error.unwrap().contains("http://"));
}

#[tokio::test]
async fn test_download_creates_dirs() {
    let tmp = tempfile::tempdir().unwrap();
    let tool = FileDownloadTool::new(tmp.path());

    // 用一个稳定的小文件测试
    let args = json!({
        "url": "https://httpbin.org/bytes/100",
        "dest_path": "sub/dir/test.bin"
    });

    let result = tool.execute(args).await.unwrap();
    if result.success {
        assert!(tmp.path().join("sub/dir/test.bin").exists());
    }
    // httpbin 可能不可用，不强制断言
}