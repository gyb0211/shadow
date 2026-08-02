//! 工具列表路由: /api/tools

use axum::{Router, routing::get};
use serde::Serialize;

use crate::auth::middleware::AuthUser;
use crate::error::GatewayResult;
use crate::state::GatewayState;

pub fn routes() -> Router<GatewayState> {
    Router::new().route("/api/tools", get(list_tools))
}

#[derive(Serialize)]
struct ToolInfo {
    name: String,
    description: String,
}

async fn list_tools(_user: AuthUser) -> GatewayResult<axum::Json<Vec<ToolInfo>>> {
    // 返回已注册的工具列表
    let tools = vec![
        ToolInfo {
            name: "http_request".to_string(),
            description: "发送 HTTP 请求".to_string(),
        },
        ToolInfo {
            name: "web_fetch".to_string(),
            description: "抓取网页内容".to_string(),
        },
        ToolInfo {
            name: "web_search".to_string(),
            description: "搜索网页".to_string(),
        },
        ToolInfo {
            name: "memory_store".to_string(),
            description: "存储记忆".to_string(),
        },
        ToolInfo {
            name: "memory_recall".to_string(),
            description: "回忆记忆".to_string(),
        },
        ToolInfo {
            name: "file_read".to_string(),
            description: "读取文件".to_string(),
        },
        ToolInfo {
            name: "file_write".to_string(),
            description: "写入文件".to_string(),
        },
        ToolInfo {
            name: "file_download".to_string(),
            description: "下载文件".to_string(),
        },
        ToolInfo {
            name: "pdf_read".to_string(),
            description: "读取 PDF 文件".to_string(),
        },
        ToolInfo {
            name: "git_operations".to_string(),
            description: "Git 操作".to_string(),
        },
        ToolInfo {
            name: "jira".to_string(),
            description: "Jira 工单操作".to_string(),
        },
    ];
    Ok(axum::Json(tools))
}
