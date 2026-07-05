//! SSRF 防护 -- 公共 URL 安全检查
//!
//! 禁止访问 localhost / 内网 IP / 保留地址, 防止服务端请求伪造攻击.
//! 被 file_download / file_upload / http_request 等网络工具复用.

use anyhow::Result;

/// 检查 URL 是否安全 (非内网地址)
///
/// - 仅允许 http/https 协议
/// - 禁止 localhost / 127.0.0.1 / 0.0.0.0 / ::1
/// - 禁止内网 IP (10.x / 172.16-31.x / 192.168.x / 169.254.x)
pub fn check_url(url: &str) -> Result<()> {
    let parsed = url::Url::parse(url)
        .map_err(|e| anyhow::anyhow!("无效的 URL: {e}"))?;

    // 仅允许 http/https
    match parsed.scheme() {
        "http" | "https" => {}
        other => anyhow::bail!("不支持的协议: {other}, 仅允许 http/https"),
    }

    // 检查主机名
    let host = parsed.host_str().unwrap_or("").to_lowercase();

    // 禁止 localhost
    if host == "localhost" || host == "127.0.0.1" || host == "0.0.0.0" || host == "::1" {
        anyhow::bail!("SSRF 防护: 禁止访问本地地址 {host}");
    }

    // 禁止内网 IP (10.x / 172.16-31.x / 192.168.x / 169.254.x)
    if let Some(ip) = host_to_ip(&host) {
        if is_private_ip(&ip) {
            anyhow::bail!("SSRF 防护: 禁止访问内网地址 {host}");
        }
    }

    Ok(())
}

/// 尝试将主机名解析为 IP (仅检查 IP 形式的主机名, 不做 DNS 解析)
fn host_to_ip(host: &str) -> Option<std::net::IpAddr> {
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        return Some(ip);
    }
    None
}

/// 判断是否为内网/保留 IP
fn is_private_ip(ip: &std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local() // 169.254.x.x
                || v4.is_unspecified() // 0.0.0.0
                || v4.is_documentation()
        }
        std::net::IpAddr::V6(v6) => {
            v6.is_loopback() || v6.is_unspecified() || v6.is_unicast_link_local()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_safe_urls() {
        assert!(check_url("https://example.com").is_ok());
        assert!(check_url("http://example.com/api/file.zip").is_ok());
        assert!(check_url("https://8.8.8.8/test").is_ok());
    }

    #[test]
    fn test_localhost_blocked() {
        assert!(check_url("http://localhost:8080").is_err());
        assert!(check_url("http://127.0.0.1:8080").is_err());
        assert!(check_url("http://0.0.0.0").is_err());
        assert!(check_url("http://[::1]:8080").is_err());
    }

    #[test]
    fn test_private_ip_blocked() {
        assert!(check_url("http://192.168.1.1").is_err());
        assert!(check_url("http://10.0.0.1").is_err());
        assert!(check_url("http://172.16.0.1").is_err());
        assert!(check_url("http://169.254.169.254").is_err());
    }

    #[test]
    fn test_bad_protocol_blocked() {
        assert!(check_url("ftp://example.com").is_err());
        assert!(check_url("file:///etc/passwd").is_err());
        assert!(check_url("gopher://localhost").is_err());
    }

    #[test]
    fn test_invalid_url() {
        assert!(check_url("not a url").is_err());
        assert!(check_url("").is_err());
    }

    #[test]
    fn test_private_ip_direct() {
        assert!(is_private_ip(&"127.0.0.1".parse().unwrap()));
        assert!(is_private_ip(&"192.168.1.1".parse().unwrap()));
        assert!(is_private_ip(&"10.0.0.1".parse().unwrap()));
        assert!(is_private_ip(&"172.16.0.1".parse().unwrap()));
        assert!(is_private_ip(&"169.254.169.254".parse().unwrap()));
        assert!(!is_private_ip(&"8.8.8.8".parse().unwrap()));
        assert!(!is_private_ip(&"1.1.1.1".parse().unwrap()));
    }
}
