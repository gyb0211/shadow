//! Git 操作工具集成测试

use shadow_tools::GitOperationsTool;
use shadow_core::Tool;
use serde_json::json;

fn init_test_repo() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    std::process::Command::new("git")
        .args(["init"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    std::process::Command::new("git")
        .args(["config", "user.email", "test@test.com"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    std::process::Command::new("git")
        .args(["config", "user.name", "Test"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    tmp
}

#[tokio::test]
async fn test_git_status_clean_repo() {
    let tmp = init_test_repo();
    let tool = GitOperationsTool::new(tmp.path());

    let args = json!({"operation": "status"});
    let result = tool.execute(args).await.unwrap();
    assert!(result.success, "{:?}", result.error);

    let output: serde_json::Value = serde_json::from_str(&result.output).unwrap();
    assert!(output["clean"].as_bool().unwrap_or(false));
}

#[tokio::test]
async fn test_git_status_with_changes() {
    let tmp = init_test_repo();
    std::fs::write(tmp.path().join("test.txt"), "hello").unwrap();

    let tool = GitOperationsTool::new(tmp.path());
    let args = json!({"operation": "status"});
    let result = tool.execute(args).await.unwrap();
    assert!(result.success);

    let output: serde_json::Value = serde_json::from_str(&result.output).unwrap();
    assert!(!output["clean"].as_bool().unwrap_or(true));
    assert!(output["untracked"].as_array().unwrap().len() > 0);
}

#[tokio::test]
async fn test_git_add_and_commit() {
    let tmp = init_test_repo();
    std::fs::write(tmp.path().join("test.txt"), "hello").unwrap();

    let tool = GitOperationsTool::new(tmp.path());

    // add
    let args = json!({"operation": "add", "paths": "test.txt"});
    let result = tool.execute(args).await.unwrap();
    assert!(result.success, "{:?}", result.error);

    // commit
    let args = json!({"operation": "commit", "message": "initial commit"});
    let result = tool.execute(args).await.unwrap();
    assert!(result.success, "{:?}", result.error);
}

#[tokio::test]
async fn test_git_log() {
    let tmp = init_test_repo();
    std::fs::write(tmp.path().join("test.txt"), "hello").unwrap();

    let tool = GitOperationsTool::new(tmp.path());

    tool.execute(json!({"operation": "add", "paths": "."})).await.unwrap();
    tool.execute(json!({"operation": "commit", "message": "test commit"})).await.unwrap();

    let args = json!({"operation": "log", "limit": 5});
    let result = tool.execute(args).await.unwrap();
    assert!(result.success);

    let output: serde_json::Value = serde_json::from_str(&result.output).unwrap();
    let commits = output["commits"].as_array().unwrap();
    assert!(!commits.is_empty());
    assert_eq!(commits[0]["message"], "test commit");
}

#[tokio::test]
async fn test_git_branch() {
    let tmp = init_test_repo();
    std::fs::write(tmp.path().join("test.txt"), "hello").unwrap();

    let tool = GitOperationsTool::new(tmp.path());
    tool.execute(json!({"operation": "add", "paths": "."})).await.unwrap();
    tool.execute(json!({"operation": "commit", "message": "init"})).await.unwrap();

    let args = json!({"operation": "branch"});
    let result = tool.execute(args).await.unwrap();
    assert!(result.success);

    let output: serde_json::Value = serde_json::from_str(&result.output).unwrap();
    assert!(!output["current"].as_str().unwrap().is_empty());
    assert!(!output["branches"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn test_git_checkout() {
    let tmp = init_test_repo();
    std::fs::write(tmp.path().join("test.txt"), "hello").unwrap();

    let tool = GitOperationsTool::new(tmp.path());
    tool.execute(json!({"operation": "add", "paths": "."})).await.unwrap();
    tool.execute(json!({"operation": "commit", "message": "init"})).await.unwrap();

    // checkout 到不存在的分支应该失败
    let args = json!({"operation": "checkout", "branch": "nonexistent"});
    let result = tool.execute(args).await.unwrap();
    assert!(!result.success, "checkout 到不存在分支应该失败");
}

#[tokio::test]
async fn test_git_not_a_repo() {
    let tmp = tempfile::tempdir().unwrap();
    let tool = GitOperationsTool::new(tmp.path());

    let args = json!({"operation": "status"});
    let result = tool.execute(args).await.unwrap();
    assert!(!result.success);
    assert!(result.error.unwrap().contains("Not a git repository"));
}

#[tokio::test]
async fn test_git_missing_operation() {
    let tmp = init_test_repo();
    let tool = GitOperationsTool::new(tmp.path());

    let args = json!({});
    let result = tool.execute(args).await.unwrap();
    assert!(!result.success);
}

#[tokio::test]
async fn test_git_commit_without_message() {
    let tmp = init_test_repo();
    let tool = GitOperationsTool::new(tmp.path());

    let args = json!({"operation": "commit"});
    let result = tool.execute(args).await.unwrap();
    assert!(!result.success);
    assert!(result.error.unwrap().contains("message"));
}

#[tokio::test]
async fn test_git_stash() {
    let tmp = init_test_repo();
    std::fs::write(tmp.path().join("test.txt"), "hello").unwrap();
    let tool = GitOperationsTool::new(tmp.path());
    tool.execute(json!({"operation": "add", "paths": "."})).await.unwrap();
    tool.execute(json!({"operation": "commit", "message": "init"})).await.unwrap();

    // 修改文件后 stash
    std::fs::write(tmp.path().join("test.txt"), "modified").unwrap();

    let args = json!({"operation": "stash", "action": "push", "message": "test stash"});
    let result = tool.execute(args).await.unwrap();
    assert!(result.success, "{:?}", result.error);

    // list
    let args = json!({"operation": "stash", "action": "list"});
    let result = tool.execute(args).await.unwrap();
    assert!(result.success);

    // pop
    let args = json!({"operation": "stash", "action": "pop"});
    let result = tool.execute(args).await.unwrap();
    assert!(result.success, "{:?}", result.error);
}