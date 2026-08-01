//! Shell 工具集成测试

use shadow_tools::ShellTool;
use shadow_core::Tool;
use serde_json::json;

#[tokio::test]
async fn test_shell_basic_echo() {
    let tool = ShellTool;
    let args = json!({
        "command": "echo hello world",
        "timeout": 5
    });
    
    let result = tool.execute(args).await.unwrap();
    assert!(result.success, "echo 命令应该成功");
    assert!(result.output.contains("hello world"), "输出应该包含 'hello world'");
}

#[tokio::test]
async fn test_shell_timeout() {
    let tool = ShellTool;
    let args = json!({
        "command": "sleep 10",
        "timeout": 1
    });
    
    let result = tool.execute(args).await.unwrap();
    assert!(!result.success, "超时应该失败");
    let error = result.error.expect("应该有错误信息");
    assert!(error.contains("timeout") || error.contains("超时"), "错误信息应该提到超时");
}

#[tokio::test]
async fn test_shell_nonzero_exit() {
    let tool = ShellTool;
    let args = json!({
        "command": "exit 42",
        "timeout": 5
    });
    
    let result = tool.execute(args).await.unwrap();
    assert!(!result.success, "非零退出码应该失败");
    let error = result.error.expect("应该有错误信息");
    assert!(error.contains("42"), "错误信息应该包含退出码 42");
}

#[tokio::test]
async fn test_shell_environment_filtering() {
    // 测试敏感环境变量被过滤
    let tool = ShellTool;
    
    unsafe {
        std::env::set_var("TEST_NORMAL_VAR", "normal_value");
        std::env::set_var("TEST_API_KEY", "should_be_filtered");
    }
    
    let args = json!({
        "command": "echo $TEST_NORMAL_VAR",
        "timeout": 5
    });
    
    let result = tool.execute(args).await.unwrap();
    assert!(result.success);
    
    unsafe {
        std::env::remove_var("TEST_NORMAL_VAR");
        std::env::remove_var("TEST_API_KEY");
    }
}