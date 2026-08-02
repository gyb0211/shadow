//! Gateway 服务器启动 -- axum HTTP + rust-embed 前端嵌入
//!
//! 启动流程:
//! 1. 加载 Shadow 配置
//! 2. 生成/加载 JWT 密钥
//! 3. 构建 axum Router (API 路由 + 前端静态文件)
//! 4. 绑定 127.0.0.1:port 启动 HTTP 服务

use std::net::SocketAddr;

use axum::{
    body::Body,
    extract::State,
    http::{header, StatusCode, Uri},
    response::{IntoResponse, Response},
    Router,
};
use rust_embed::Embed;
use tower_http::cors::CorsLayer;

use crate::routes;
use crate::state::GatewayState;

/// 嵌入前端编译产物 (crates/shadow-webui/dist/)
#[derive(Embed)]
#[folder = "../shadow-webui/dist/"]
struct WebuiAsset;

/// 启动 Gateway 服务器
pub async fn run_gateway(config: shadow_config::Config, port: u16) -> anyhow::Result<()> {
    let data_dir = config.data_dir.clone();
    let config_path = config.config_path.clone();

    // JWT 密钥 -- 从 ~/.shadow/.jwt_secret 加载或随机生成
    let jwt_secret = load_or_create_jwt_secret(&data_dir)?;

    let state = GatewayState {
        jwt_secret,
        config: std::sync::Arc::new(tokio::sync::RwLock::new(config)),
        config_path,
        data_dir,
        daemon_running: false,
    };

    let app = Router::new()
        .merge(routes::routes())
        .fallback(static_handler)
        .layer(CorsLayer::permissive())
        .with_state(state);

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("Gateway 启动: http://{addr}");
    axum::serve(listener, app).await?;
    Ok(())
}

/// 静态文件处理 -- 从 rust-embed 读取前端资源，SPA fallback 到 index.html
async fn static_handler(State(_state): State<GatewayState>, uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');

    // 精确匹配静态文件
    if !path.is_empty() {
        if let Some(file) = WebuiAsset::get(path) {
            return build_response(path, file.data.into_owned());
        }
    }

    // SPA fallback: 所有未匹配的路由返回 index.html
    if let Some(file) = WebuiAsset::get("index.html") {
        return build_response("index.html", file.data.into_owned());
    }

    (StatusCode::NOT_FOUND, "前端资源未找到").into_response()
}

/// 构建文件响应 (自动推断 MIME 类型)
fn build_response(path: &str, data: Vec<u8>) -> Response {
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    Response::builder()
        .header(header::CONTENT_TYPE, mime.as_ref())
        .body(Body::from(data))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

/// 加载或创建 JWT 密钥
fn load_or_create_jwt_secret(data_dir: &std::path::Path) -> anyhow::Result<String> {
    use std::fs;
    let key_path = data_dir.join(".jwt_secret");

    if key_path.exists() {
        let secret = fs::read_to_string(&key_path)?;
        if !secret.trim().is_empty() {
            return Ok(secret.trim().to_string());
        }
    }

    // 生成随机密钥 (32 字节 hex 编码)
    let secret = uuid::Uuid::new_v4().to_string() + &uuid::Uuid::new_v4().to_string();
    fs::create_dir_all(data_dir)?;
    fs::write(&key_path, &secret)?;
    // Unix 权限保护
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = fs::Permissions::from_mode(0o600);
        let _ = fs::set_permissions(&key_path, perms);
    }
    Ok(secret)
}
