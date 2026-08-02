//! Jira 工具 -- 与 Jira REST API 交互
//!
//! 支持 Jira Server/DC（Basic Auth: 用户名+密码）
//! 和 Jira Cloud（Basic Auth: email+api_token）
//!
//! 参考 ZeroClaw jira_tool.rs，简化实现核心 4 个 action:
//! - get_ticket: 获取工单
//! - search_tickets: JQL 搜索
//! - comment_ticket: 添加评论
//! - create_ticket: 创建工单

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use shadow_core::{Tool, ToolResult};
use std::time::Duration;

/// Jira 工具
pub struct JiraTool {
    /// Jira 实例地址（如 http://jira.wb-intra.com）
    base_url: String,
    /// 认证信息
    auth: JiraAuth,
    /// 允许的操作
    allowed_actions: Vec<String>,
    /// HTTP 客户端
    http: reqwest::Client,
    /// 请求超时
    timeout: Duration,
}

/// Jira 认证方式
enum JiraAuth {
    /// Server/DC: 用户名+密码
    Basic { username: String, password: String },
    /// Cloud: email+api_token
    Cloud { email: String, api_token: String },
}

impl JiraTool {
    /// 从配置创建 JiraTool
    pub fn from_config(config: &shadow_config::schema::JiraConfig) -> Result<Self> {
        if config.base_url.is_empty() {
            anyhow::bail!("Jira base_url must not be empty");
        }

        let auth = if let Some(email) = &config.email {
            // Cloud 模式
            let token = config.password.as_deref().unwrap_or("");
            if token.is_empty() {
                anyhow::bail!("Jira Cloud requires password (api_token)");
            }
            JiraAuth::Cloud {
                email: email.clone(),
                api_token: token.to_string(),
            }
        } else if let Some(username) = &config.username {
            // Server/DC 模式
            let password = config.password.as_deref().unwrap_or("");
            if password.is_empty() {
                anyhow::bail!("Jira Server requires password");
            }
            JiraAuth::Basic {
                username: username.clone(),
                password: password.to_string(),
            }
        } else {
            anyhow::bail!("Jira requires either username or email for authentication");
        };

        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs))
            .build()
            .context("Failed to build HTTP client for Jira")?;

        Ok(Self {
            base_url: config.base_url.trim_end_matches('/').to_string(),
            auth,
            allowed_actions: config.allowed_actions.clone(),
            http,
            timeout: Duration::from_secs(config.timeout_secs),
        })
    }

    /// API 版本: Cloud 用 v3, Server 用 v2
    fn api_version(&self) -> &str {
        match &self.auth {
            JiraAuth::Cloud { .. } => "3",
            JiraAuth::Basic { .. } => "2",
        }
    }

    /// 添加认证头
    fn authenticated(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.auth {
            JiraAuth::Cloud { email, api_token } => {
                req.basic_auth(email, Some(api_token))
            }
            JiraAuth::Basic { username, password } => {
                req.basic_auth(username, Some(password))
            }
        }
    }

    /// 检查 action 是否允许
    fn is_action_allowed(&self, action: &str) -> bool {
        self.allowed_actions.iter().any(|a| a == action)
    }

    // ── API 方法 ──────────────────────────────────────────────────

    /// 获取工单
    async fn get_ticket(&self, issue_key: &str) -> Result<ToolResult> {
        validate_issue_key(issue_key)?;
        let ver = self.api_version();
        let url = format!("{}/rest/api/{}/issue/{}", self.base_url, ver, issue_key);

        let req = self.http.get(&url).query(&[
            ("fields", "summary"),
            ("fields", "status"),
            ("fields", "priority"),
            ("fields", "assignee"),
            ("fields", "description"),
            ("fields", "created"),
            ("fields", "updated"),
            ("fields", "comment"),
        ]).timeout(self.timeout);

        let resp = self.authenticated(req).send().await
            .context("Jira get_ticket request failed")?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Ok(ToolResult::err(format!("Jira get_ticket failed ({status}): {}", truncate(&text, 500))));
        }

        let raw: Value = resp.json().await
            .context("Failed to parse Jira response")?;

        let shaped = shape_ticket(&raw);
        Ok(ToolResult::ok(serde_json::to_string_pretty(&shaped).unwrap_or_default()))
    }

    /// JQL 搜索工单
    async fn search_tickets(&self, jql: &str, max_results: u32) -> Result<ToolResult> {
        let max_results = max_results.clamp(1, 999);
        let ver = self.api_version();
        let url = format!("{}/rest/api/{}/search", self.base_url, ver);

        let body = json!({
            "jql": jql,
            "maxResults": max_results,
            "fields": ["summary", "status", "priority", "assignee", "created", "updated"]
        });

        let req = self.http.post(&url).json(&body).timeout(self.timeout);
        let resp = self.authenticated(req).send().await
            .context("Jira search request failed")?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Ok(ToolResult::err(format!("Jira search failed ({status}): {}", truncate(&text, 500))));
        }

        let raw: Value = resp.json().await
            .context("Failed to parse Jira search response")?;

        let issues: Vec<Value> = raw["issues"]
            .as_array()
            .map(|arr| arr.iter().map(shape_ticket_brief).collect())
            .unwrap_or_default();

        let output = json!({
            "total": raw["total"],
            "issues": issues,
        });

        Ok(ToolResult::ok(serde_json::to_string_pretty(&output).unwrap_or_default()))
    }

    /// 添加评论
    async fn comment_ticket(&self, issue_key: &str, comment: &str) -> Result<ToolResult> {
        validate_issue_key(issue_key)?;
        if comment.is_empty() {
            return Ok(ToolResult::err("Comment must not be empty"));
        }

        let ver = self.api_version();
        let url = format!("{}/rest/api/{}/issue/{}/comment", self.base_url, ver, issue_key);

        let body = json!({ "body": comment });
        let req = self.http.post(&url).json(&body).timeout(self.timeout);
        let resp = self.authenticated(req).send().await
            .context("Jira comment request failed")?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Ok(ToolResult::err(format!("Jira comment failed ({status}): {}", truncate(&text, 500))));
        }

        let raw: Value = resp.json().await.unwrap_or_default();
        let output = json!({
            "id": raw["id"],
            "author": raw["author"]["displayName"],
            "body": raw["body"],
            "created": raw["created"],
        });

        Ok(ToolResult::ok(serde_json::to_string_pretty(&output).unwrap_or_default()))
    }

    /// 创建工单
    async fn create_ticket(
        &self,
        project_key: &str,
        issue_type: &str,
        summary: &str,
        description: Option<&str>,
    ) -> Result<ToolResult> {
        validate_project_key(project_key)?;
        if summary.trim().is_empty() {
            return Ok(ToolResult::err("Summary must not be empty"));
        }
        if issue_type.trim().is_empty() {
            return Ok(ToolResult::err("Issue type must not be empty"));
        }

        let ver = self.api_version();
        let url = format!("{}/rest/api/{}/issue", self.base_url, ver);

        let mut fields = serde_json::Map::new();
        fields.insert("project".into(), json!({ "key": project_key }));
        fields.insert("issuetype".into(), json!({ "name": issue_type }));
        fields.insert("summary".into(), json!(summary));

        if let Some(desc) = description {
            fields.insert("description".into(), json!(desc));
        }

        let body = json!({ "fields": Value::Object(fields) });
        let req = self.http.post(&url).json(&body).timeout(self.timeout);
        let resp = self.authenticated(req).send().await
            .context("Jira create_ticket request failed")?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Ok(ToolResult::err(format!("Jira create_ticket failed ({status}): {}", truncate(&text, 500))));
        }

        let raw: Value = resp.json().await.unwrap_or_default();
        let key = raw["key"].as_str().unwrap_or("");
        let output = json!({
            "id": raw["id"],
            "key": key,
            "browse_url": format!("{}/browse/{}", self.base_url, key),
        });

        Ok(ToolResult::ok(serde_json::to_string_pretty(&output).unwrap_or_default()))
    }

    /// 验证凭据
    async fn myself(&self) -> Result<ToolResult> {
        let ver = self.api_version();
        let url = format!("{}/rest/api/{}/myself", self.base_url, ver);

        let req = self.http.get(&url).timeout(self.timeout);
        let resp = self.authenticated(req).send().await
            .context("Jira myself request failed")?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Ok(ToolResult::err(format!("Jira authentication failed ({status}): {}", truncate(&text, 500))));
        }

        let raw: Value = resp.json().await.unwrap_or_default();
        let output = json!({
            "name": raw["name"],
            "display_name": raw["displayName"],
            "email": raw["emailAddress"],
            "active": raw["active"],
        });

        Ok(ToolResult::ok(serde_json::to_string_pretty(&output).unwrap_or_default()))
    }
}

shadow_core::tool_attribution!(JiraTool, shadow_core::ToolKind::HttpRequest);

#[async_trait]
impl Tool for JiraTool {
    fn name(&self) -> &str {
        "jira"
    }

    fn description(&self) -> &str {
        "Interact with Jira: get tickets, search with JQL, add comments, create tickets, and verify credentials."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["get_ticket", "search_tickets", "comment_ticket", "create_ticket", "myself"],
                    "description": "The Jira action to perform."
                },
                "issue_key": {
                    "type": "string",
                    "description": "Jira issue key, e.g. 'PROJ-123'. Required for get_ticket and comment_ticket."
                },
                "jql": {
                    "type": "string",
                    "description": "JQL query for search_tickets. Example: 'project = PROJ AND status = \"In Progress\" ORDER BY updated DESC'"
                },
                "max_results": {
                    "type": "integer",
                    "description": "Max results for search_tickets. Default 25, max 999.",
                    "default": 25
                },
                "comment": {
                    "type": "string",
                    "description": "Comment text for comment_ticket."
                },
                "project_key": {
                    "type": "string",
                    "description": "Project key for create_ticket, e.g. 'PROJ'."
                },
                "issue_type": {
                    "type": "string",
                    "description": "Issue type for create_ticket, e.g. 'Task', 'Bug', 'Story'."
                },
                "summary": {
                    "type": "string",
                    "description": "Ticket title for create_ticket."
                },
                "description": {
                    "type": "string",
                    "description": "Ticket description for create_ticket. Optional."
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> Result<ToolResult> {
        let action = args.get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: action"))?;

        // 检查 action 是否允许
        if !self.is_action_allowed(action) {
            return Ok(ToolResult::err(format!(
                "Action '{}' is not enabled. Allowed: {}",
                action,
                self.allowed_actions.join(", ")
            )));
        }

        match action {
            "get_ticket" => {
                let issue_key = args.get("issue_key").and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("get_ticket requires issue_key"))?;
                self.get_ticket(issue_key).await
            }
            "search_tickets" => {
                let jql = args.get("jql").and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("search_tickets requires jql"))?;
                let max = args.get("max_results").and_then(|v| v.as_u64())
                    .map(|n| n as u32).unwrap_or(25);
                self.search_tickets(jql, max).await
            }
            "comment_ticket" => {
                let issue_key = args.get("issue_key").and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("comment_ticket requires issue_key"))?;
                let comment = args.get("comment").and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("comment_ticket requires comment"))?;
                self.comment_ticket(issue_key, comment).await
            }
            "create_ticket" => {
                let project_key = args.get("project_key").and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("create_ticket requires project_key"))?;
                let issue_type = args.get("issue_type").and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("create_ticket requires issue_type"))?;
                let summary = args.get("summary").and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("create_ticket requires summary"))?;
                let desc = args.get("description").and_then(|v| v.as_str());
                self.create_ticket(project_key, issue_type, summary, desc).await
            }
            "myself" => {
                self.myself().await
            }
            other => {
                Ok(ToolResult::err(format!("Unknown action: {other}")))
            }
        }
    }
}

// ── 工具函数 ──────────────────────────────────────────────────

/// 验证 issue key 格式（如 PROJ-123）
fn validate_issue_key(key: &str) -> Result<()> {
    let valid = key.split_once('-').is_some_and(|(project, number)| {
        !project.is_empty()
            && project.chars().all(|c| c.is_ascii_alphanumeric())
            && !number.is_empty()
            && number.chars().all(|c| c.is_ascii_digit())
    });
    if valid {
        Ok(())
    } else {
        anyhow::bail!("Invalid issue key '{key}'. Expected format: PROJECT-123");
    }
}

/// 验证 project key 格式（如 PROJ）
fn validate_project_key(key: &str) -> Result<()> {
    let valid = !key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric());
    if valid {
        Ok(())
    } else {
        anyhow::bail!("Invalid project key '{key}'. Expected ASCII alphanumeric, e.g. PROJ");
    }
}

/// 截断字符串
fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max])
    }
}

/// 精简工单详情
fn shape_ticket(raw: &Value) -> Value {
    let f = &raw["fields"];
    json!({
        "key": raw["key"],
        "summary": f["summary"],
        "status": f["status"]["name"],
        "priority": f["priority"]["name"],
        "assignee": f["assignee"]["displayName"],
        "description": f["description"],
        "created": f["created"],
        "updated": f["updated"],
        "comments": f["comment"]["comments"].as_array()
            .map(|arr| arr.iter().map(|c| json!({
                "author": c["author"]["displayName"],
                "body": c["body"],
                "created": c["created"],
            })).collect::<Vec<_>>())
            .unwrap_or_default(),
    })
}

/// 精简工单摘要（搜索结果用）
fn shape_ticket_brief(raw: &Value) -> Value {
    let f = &raw["fields"];
    json!({
        "key": raw["key"],
        "summary": f["summary"],
        "status": f["status"]["name"],
        "priority": f["priority"]["name"],
        "assignee": f["assignee"]["displayName"],
        "created": f["created"],
        "updated": f["updated"],
    })
}