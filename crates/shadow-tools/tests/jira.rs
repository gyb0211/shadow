//! Jira 工具集成测试

use shadow_tools::JiraTool;
use shadow_core::Tool;
use serde_json::json;

#[test]
fn test_jira_tool_creation_server() {
    unsafe { std::env::set_var("JIRA_PASSWORD", "testpass"); }

    let config = shadow_config::schema::JiraConfig {
        enabled: true,
        base_url: "http://jira.wb-intra.com".to_string(),
        username: Some("testuser".to_string()),
        email: None,
        allowed_actions: vec!["get_ticket".to_string(), "search_tickets".to_string()],
        timeout_secs: 30,
    };

    let tool = JiraTool::from_config(&config);
    assert!(tool.is_ok(), "Should create JiraTool for Server mode");
    let tool = tool.unwrap();
    assert_eq!(tool.name(), "jira");

    unsafe { std::env::remove_var("JIRA_PASSWORD"); }
}

#[test]
fn test_jira_tool_creation_cloud() {
    unsafe { std::env::set_var("JIRA_PASSWORD", "api-token"); }

    let config = shadow_config::schema::JiraConfig {
        enabled: true,
        base_url: "https://yourco.atlassian.net".to_string(),
        username: None,
        email: Some("user@company.com".to_string()),
        allowed_actions: vec!["get_ticket".to_string()],
        timeout_secs: 30,
    };

    let tool = JiraTool::from_config(&config);
    assert!(tool.is_ok(), "Should create JiraTool for Cloud mode");

    unsafe { std::env::remove_var("JIRA_PASSWORD"); }
}

#[test]
fn test_jira_tool_missing_base_url() {
    unsafe { std::env::set_var("JIRA_PASSWORD", "testpass"); }

    let config = shadow_config::schema::JiraConfig {
        enabled: true,
        base_url: String::new(),
        username: Some("testuser".to_string()),
        ..Default::default()
    };

    let result = JiraTool::from_config(&config);
    assert!(result.is_err(), "Should fail without base_url");

    unsafe { std::env::remove_var("JIRA_PASSWORD"); }
}

#[test]
fn test_jira_tool_missing_password_env() {
    // 确保环境变量不存在
    unsafe { std::env::remove_var("JIRA_PASSWORD"); }

    let config = shadow_config::schema::JiraConfig {
        enabled: true,
        base_url: "http://jira.example.com".to_string(),
        username: Some("testuser".to_string()),
        ..Default::default()
    };

    let result = JiraTool::from_config(&config);
    assert!(result.is_err(), "Should fail without JIRA_PASSWORD env var");
}

#[test]
fn test_jira_tool_missing_credentials() {
    let config = shadow_config::schema::JiraConfig {
        enabled: true,
        base_url: "http://jira.example.com".to_string(),
        username: None,
        email: None,
        ..Default::default()
    };

    let result = JiraTool::from_config(&config);
    assert!(result.is_err(), "Should fail without credentials");
}

#[test]
fn test_jira_tool_parameters_schema() {
    unsafe { std::env::set_var("JIRA_PASSWORD", "testpass"); }

    let config = shadow_config::schema::JiraConfig {
        enabled: true,
        base_url: "http://jira.example.com".to_string(),
        username: Some("testuser".to_string()),
        ..Default::default()
    };

    let tool = JiraTool::from_config(&config).unwrap();
    let schema = tool.parameters_schema();

    assert_eq!(schema["properties"]["action"]["enum"][0], "get_ticket");
    assert_eq!(schema["properties"]["action"]["enum"][1], "search_tickets");
    assert_eq!(schema["properties"]["action"]["enum"][2], "comment_ticket");
    assert_eq!(schema["properties"]["action"]["enum"][3], "create_ticket");
    assert_eq!(schema["properties"]["action"]["enum"][4], "myself");
    assert_eq!(schema["required"][0], "action");
}

// 注意：以下测试需要有效的 Jira 凭据
// 运行: cargo test --test jira -- --ignored

#[tokio::test]
#[ignore]
async fn test_jira_myself_real() {
    // 需要设置环境变量: export JIRA_PASSWORD="your_password"
    let config = shadow_config::schema::JiraConfig {
        enabled: true,
        base_url: "http://jira.wb-intra.com".to_string(),
        username: Some("your_username".to_string()),
        allowed_actions: vec!["myself".to_string()],
        timeout_secs: 30,
        ..Default::default()
    };

    let tool = match JiraTool::from_config(&config) {
        Ok(t) => t,
        Err(e) => {
            println!("需要设置 JIRA_PASSWORD 环境变量: {}", e);
            return;
        }
    };
    let args = json!({"action": "myself"});
    let result = tool.execute(args).await.unwrap();

    if result.success {
        println!("Jira 认证成功: {}", result.output);
    } else {
        println!("Jira 认证失败: {:?}", result.error);
    }
}

#[tokio::test]
#[ignore]
async fn test_jira_search_real() {
    // 需要设置环境变量: export JIRA_PASSWORD="your_password"
    let config = shadow_config::schema::JiraConfig {
        enabled: true,
        base_url: "http://jira.wb-intra.com".to_string(),
        username: Some("your_username".to_string()),
        allowed_actions: vec!["search_tickets".to_string()],
        timeout_secs: 30,
        ..Default::default()
    };

    let tool = match JiraTool::from_config(&config) {
        Ok(t) => t,
        Err(e) => {
            println!("需要设置 JIRA_PASSWORD 环境变量: {}", e);
            return;
        }
    };
    let args = json!({
        "action": "search_tickets",
        "jql": "project IS NOT EMPTY ORDER BY updated DESC",
        "max_results": 5
    });
    let result = tool.execute(args).await.unwrap();

    if result.success {
        println!("搜索结果: {}", result.output);
    } else {
        println!("搜索失败: {:?}", result.error);
    }
}