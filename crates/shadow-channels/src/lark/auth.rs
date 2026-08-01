//! 飞书 / Lark 机器人配置
//!
//! 两种方式:
//! 1. 扫码自动创建机器人 (device-code 注册流程, 推荐)
//! 2. 手动输入 app_id + app_secret

use anyhow::{Result, anyhow, bail};
use serde_json::Value;
use std::time::Duration;

use super::platform::LarkPlatform;

// ── 注册 API 常量 ────────────────────────────────────

const ACCOUNTS_URLS: &[(&str, &str)] = &[
    ("feishu", "https://accounts.feishu.cn"),
    ("lark", "https://accounts.larksuite.com"),
];

const OPEN_URLS: &[(&str, &str)] = &[
    ("feishu", "https://open.feishu.cn"),
    ("lark", "https://open.larksuite.com"),
];

const REGISTRATION_PATH: &str = "/oauth/v1/app/registration";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

fn accounts_base_url(domain: &str) -> &str {
    ACCOUNTS_URLS
        .iter()
        .find(|(d, _)| *d == domain)
        .map(|(_, url)| *url)
        .unwrap_or("https://accounts.feishu.cn")
}

fn open_base_url(domain: &str) -> &str {
    OPEN_URLS
        .iter()
        .find(|(d, _)| *d == domain)
        .map(|(_, url)| *url)
        .unwrap_or("https://open.feishu.cn")
}

// ── 注册流程 ─────────────────────────────────────────

/// POST form-encoded data 到注册端点, 返回 JSON
async fn post_registration(base_url: &str, params: &[(&str, &str)]) -> Result<Value> {
    let url = format!("{base_url}{REGISTRATION_PATH}");
    let client = reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()?;
    let resp = client.post(&url).form(params).send().await?;
    let status = resp.status();
    let text = resp.text().await?;
    let json: Value = serde_json::from_str(&text)
        .map_err(|_| anyhow!("注册端点返回非 JSON: HTTP {status}, body={text}"))?;
    Ok(json)
}

/// 初始化注册, 验证环境支持 client_secret 认证
async fn init_registration(domain: &str) -> Result<()> {
    let base_url = accounts_base_url(domain);
    let res = post_registration(base_url, &[("action", "init")]).await?;
    let methods = res
        .get("supported_auth_methods")
        .and_then(|m| m.as_array())
        .cloned()
        .unwrap_or_default();
    let supported: Vec<&str> = methods.iter().filter_map(|v| v.as_str()).collect();
    if !supported.contains(&"client_secret") {
        bail!(
            "飞书注册环境不支持 client_secret 认证, 支持的方法: {:?}",
            supported
        );
    }
    Ok(())
}

/// 开始 device-code 流程, 返回 device_code, qr_url, interval, expire_in
struct DeviceCode {
    device_code: String,
    qr_url: String,
    interval: u64,
    expire_in: u64,
}

async fn begin_registration(domain: &str) -> Result<DeviceCode> {
    let base_url = accounts_base_url(domain);
    let res = post_registration(
        base_url,
        &[
            ("action", "begin"),
            ("archetype", "PersonalAgent"),
            ("auth_method", "client_secret"),
            ("request_user_info", "open_id"),
        ],
    )
    .await?;

    let device_code = res
        .get("device_code")
        .and_then(|c| c.as_str())
        .ok_or_else(|| anyhow!("注册未返回 device_code"))?
        .to_string();

    let mut qr_url = res
        .get("verification_uri_complete")
        .and_then(|u| u.as_str())
        .unwrap_or("")
        .to_string();
    if !qr_url.is_empty() {
        qr_url.push_str(if qr_url.contains('?') {
            "&from=shadow&tp=shadow"
        } else {
            "?from=shadow&tp=shadow"
        });
    }

    Ok(DeviceCode {
        device_code,
        qr_url,
        interval: res.get("interval").and_then(|i| i.as_u64()).unwrap_or(5),
        expire_in: res.get("expire_in").and_then(|e| e.as_u64()).unwrap_or(600),
    })
}

/// 轮询注册结果, 返回凭证
pub struct RegistrationResult {
    pub app_id: String,
    pub app_secret: String,
    pub domain: String,
    pub open_id: Option<String>,
}

async fn poll_registration(
    device_code: &str,
    interval: u64,
    expire_in: u64,
    domain: &str,
) -> Result<Option<RegistrationResult>> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(expire_in);
    let mut current_domain = domain.to_string();
    let mut domain_switched = false;
    let mut poll_count = 0u32;

    loop {
        if tokio::time::Instant::now() >= deadline {
            if poll_count > 0 {
                println!();
            }
            return Ok(None);
        }

        let base_url = accounts_base_url(&current_domain);
        let res = match post_registration(
            base_url,
            &[
                ("action", "poll"),
                ("device_code", device_code),
                ("tp", "ob_app"),
            ],
        )
        .await
        {
            Ok(r) => r,
            Err(_) => {
                tokio::time::sleep(Duration::from_secs(interval)).await;
                continue;
            }
        };

        poll_count += 1;
        if poll_count == 1 {
            print!("  正在获取配置结果...");
            use std::io::Write;
            let _ = std::io::stdout().flush();
        } else if poll_count % 6 == 0 {
            print!(".");
            use std::io::Write;
            let _ = std::io::stdout().flush();
        }

        // 域名自动检测
        let user_info = res.get("user_info").cloned().unwrap_or_default();
        let tenant_brand = user_info.get("tenant_brand").and_then(|b| b.as_str());
        if tenant_brand == Some("lark") && !domain_switched {
            current_domain = "lark".to_string();
            domain_switched = true;
        }

        // 成功
        if let (Some(client_id), Some(client_secret)) = (
            res.get("client_id").and_then(|c| c.as_str()),
            res.get("client_secret").and_then(|c| c.as_str()),
        ) {
            if poll_count > 0 {
                println!();
            }
            return Ok(Some(RegistrationResult {
                app_id: client_id.to_string(),
                app_secret: client_secret.to_string(),
                domain: current_domain.clone(),
                open_id: user_info
                    .get("open_id")
                    .and_then(|o| o.as_str())
                    .map(String::from),
            }));
        }

        // 终端错误
        let error = res.get("error").and_then(|e| e.as_str()).unwrap_or("");
        if error == "access_denied" || error == "expired_token" {
            if poll_count > 0 {
                println!();
            }
            return Ok(None);
        }

        // authorization_pending — 继续轮询
        tokio::time::sleep(Duration::from_secs(interval)).await;
    }
}

// ── probe_bot (验证机器人信息) ───────────────────────

/// 验证机器人连接, 返回 {bot_name, bot_open_id}
pub async fn probe_bot(app_id: &str, app_secret: &str, domain: &str) -> Result<Option<Value>> {
    let api_base = open_base_url(domain);

    // 获取 tenant_access_token
    let token_url = format!("{api_base}/open-apis/auth/v3/tenant_access_token/internal");
    let client = reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()?;
    let token_resp: Value = client
        .post(&token_url)
        .json(&serde_json::json!({
            "app_id": app_id,
            "app_secret": app_secret,
        }))
        .send()
        .await?
        .json()
        .await?;

    let token = match token_resp
        .get("tenant_access_token")
        .and_then(|t| t.as_str())
    {
        Some(t) => t,
        None => return Ok(None),
    };

    // 查询机器人信息
    let bot_url = format!("{api_base}/open-apis/bot/v3/info");
    let bot_resp = client
        .get(&bot_url)
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await?;

    let bot_data: Value = bot_resp.json().await?;
    if bot_data.get("code").and_then(|c| c.as_i64()) != Some(0) {
        return Ok(None);
    }

    let bot = bot_data
        .get("bot")
        .or_else(|| bot_data.get("data").and_then(|d| d.get("bot")))
        .cloned();

    Ok(bot.map(|b| {
        serde_json::json!({
            "bot_name": b.get("app_name").or_else(|| b.get("bot_name")),
            "bot_open_id": b.get("open_id"),
        })
    }))
}

// ── 二维码渲染 ───────────────────────────────────────

fn render_qr_terminal(url: &str) {
    match qrcode::QrCode::new(url.as_bytes()) {
        Ok(qr) => {
            // 紧凑渲染：用半高块字符 ▀▄ 让每个终端字符行表示两行 QR 模块
            // 相比 module_dimensions(2,1) 缩小到约 1/4 面积
            let w = qr.width();
            // 反色逻辑：qrcode dark=true → QR 数据点(黑) → 终端里用空格
            //          qrcode dark=false → QR 背景(白) → 终端里用 █
            let is_set = |x: usize, y: usize| -> bool {
                qr[(x, y)] == qrcode::Color::Dark
            };
            let mut y = 0;
            while y < w {
                let mut line = String::with_capacity(w);
                for x in 0..w {
                    let top = is_set(x, y);
                    let bot = if y + 1 < w { is_set(x, y + 1) } else { true };
                    match (top, bot) {
                        (true, true) => line.push(' '),    // 都黑 → 空格
                        (false, false) => line.push('█'), // 都白 → 全块
                        (true, false) => line.push('▄'),  // 上黑下白
                        (false, true) => line.push('▀'),  // 上白下黑
                    }
                }
                println!("  {line}");
                y += 2;
            }
        }
        Err(e) => eprintln!("  [QR 码生成失败: {e}]"),
    }
}

// ── 主流程入口 ───────────────────────────────────────

/// 扫码自动创建机器人
///
/// device-code 流程: init → begin → 显示 QR → poll → 拿到 app_id/app_secret
pub async fn qr_register(domain: &str) -> Result<Option<RegistrationResult>> {
    print!("  正在连接飞书 / Lark...");
    use std::io::Write;
    let _ = std::io::stdout().flush();

    init_registration(domain).await?;
    let begin = begin_registration(domain).await?;
    println!(" 完成.");

    println!();
    let qr_url = &begin.qr_url;
    if qr_url.is_empty() {
        bail!("注册未返回二维码 URL");
    }

    render_qr_terminal(qr_url);
    println!();
    println!("  扫描上方二维码, 或直接打开此链接:");
    println!("  {qr_url}");
    println!();

    let result =
        poll_registration(&begin.device_code, begin.interval, begin.expire_in, domain).await?;

    Ok(result)
}

/// 手动凭证验证
///
/// 用 app_id + app_secret 获取 tenant_access_token, 验证是否有效
pub async fn verify_credentials(
    app_id: &str,
    app_secret: &str,
    domain: &str,
) -> Result<Option<Value>> {
    probe_bot(app_id, app_secret, domain).await
}

// ── 测试 ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accounts_url_feishu() {
        assert_eq!(accounts_base_url("feishu"), "https://accounts.feishu.cn");
    }

    #[test]
    fn accounts_url_lark() {
        assert_eq!(accounts_base_url("lark"), "https://accounts.larksuite.com");
    }

    #[test]
    fn accounts_url_unknown_defaults_feishu() {
        assert_eq!(accounts_base_url("unknown"), "https://accounts.feishu.cn");
    }

    #[test]
    fn open_url_feishu() {
        assert_eq!(open_base_url("feishu"), "https://open.feishu.cn");
    }

    #[test]
    fn open_url_lark() {
        assert_eq!(open_base_url("lark"), "https://open.larksuite.com");
    }
}
