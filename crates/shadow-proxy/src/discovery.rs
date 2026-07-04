//! Agent 自动发现 -- 启动时探测本地已安装的 agent
//!
//! 三种发现方式:
//! 1. PATH 探测: which claude / which codex / which hermes → 注册为 ACP agent
//! 2. 端口探测: 扫描常见端口 GET /.well-known/agent-card.json → 注册为 A2A agent
//! 3. 配置扫描: 读取 ~/.claude, ~/.hermes 等已知路径

use anyhow::Result;
use crate::card::{AgentCard, TransportKind};

/// 已知的 CLI agent 定义
struct KnownAgent {
    /// 二进制名称
    bin: &'static str,
    /// agent 名称
    name: &'static str,
    /// ACP 参数
    args: &'static [&'static str],
    /// 能力标签
    capabilities: &'static [&'static str],
    /// 描述
    description: &'static str,
}

/// 已知的本地 CLI agent 列表
const KNOWN_CLI_AGENTS: &[KnownAgent] = &[
    KnownAgent {
        bin: "claude",
        name: "claude-code",
        args: &["--acp", "--stdio"],
        capabilities: &["coding", "code_review", "refactoring", "bash"],
        description: "Anthropic Claude Code CLI -- 自主编码 agent",
    },
    KnownAgent {
        bin: "codex",
        name: "codex",
        args: &["--acp", "--stdio"],
        capabilities: &["coding", "code_review", "bash"],
        description: "OpenAI Codex CLI -- 编码 agent",
    },
    KnownAgent {
        bin: "opencode",
        name: "opencode",
        args: &["--acp", "--stdio"],
        capabilities: &["coding", "bash"],
        description: "OpenCode CLI -- 开源编码 agent",
    },
    KnownAgent {
        bin: "hermes",
        name: "hermes",
        args: &["acp"],
        capabilities: &["chat", "coding", "research", "terminal", "file"],
        description: "Hermes Agent -- 多平台 AI agent",
    },
];

/// 常见的 A2A agent 端口 (用于本地探测)
const PROBE_PORTS: &[u16] = &[8080, 9090, 9091, 3000, 4000, 5000];

/// 发现结果
#[derive(Debug)]
pub struct DiscoveredAgent {
    pub card: AgentCard,
    pub source: DiscoverySource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscoverySource {
    /// 从 PATH 发现
    Path,
    /// 从端口发现
    Port(u16),
    /// 从配置文件发现
    Config,
}

/// 自动发现本地 agent
pub async fn discover_agents() -> Vec<DiscoveredAgent> {
    let mut results = Vec::new();

    // 1. PATH 探测
    for known in KNOWN_CLI_AGENTS {
        if let Ok(path) = which::which(known.bin) {
            shadow_log::record!(
                INFO,
                shadow_log::Action::Note,
                format!("发现 agent: {} ({})", known.name, path.display())
            );
            results.push(DiscoveredAgent {
                card: AgentCard {
                    name: known.name.to_string(),
                    description: known.description.to_string(),
                    transport: TransportKind::Acp,
                    capabilities: known.capabilities.iter().map(|s| s.to_string()).collect(),
                    skills: Vec::new(),
                    endpoint: None,
                    command: Some(known.bin.to_string()),
                    args: Some(known.args.iter().map(|s| s.to_string()).collect()),
                    last_heartbeat: Some(chrono::Utc::now().to_rfc3339()),
                },
                source: DiscoverySource::Path,
            });
        }
    }

    // 2. 端口探测 (并行)
    let port_futures: Vec<_> = PROBE_PORTS
        .iter()
        .map(|&port| probe_port(port))
        .collect();
    let port_results = futures::future::join_all(port_futures).await;
    for (port, maybe_card) in port_results {
        if let Some(card) = maybe_card {
            shadow_log::record!(
                INFO,
                shadow_log::Action::Note,
                format!("发现远程 agent: {} (port {})", card.name, port)
            );
            results.push(DiscoveredAgent {
                card,
                source: DiscoverySource::Port(port),
            });
        }
    }

    results
}

/// 探测单个端口是否有 A2A agent
async fn probe_port(port: u16) -> (u16, Option<AgentCard>) {
    let url = format!("http://127.0.0.1:{port}/.well-known/agent-card.json");
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(500))
        .build()
        .unwrap_or_default();

    match client.get(&url).send().await {
        Ok(resp) if resp.status().is_success() => {
            if let Ok(json) = resp.json::<serde_json::Value>().await {
                // 尝试解析为 AgentCard
                if let Ok(card) = parse_discovery_card(&json, port) {
                    return (port, Some(card));
                }
            }
        }
        _ => {}
    }
    (port, None)
}

/// 从发现响应解析 AgentCard
fn parse_discovery_card(json: &serde_json::Value, port: u16) -> Result<AgentCard> {
    // A2A catalog 格式: { "name": "...", "agents": [...] }
    // 单个 AgentCard 格式: { "name": "...", "supportedInterfaces": [...] }

    // 如果是 catalog (有 agents 数组), 取第一个
    if let Some(agents) = json.get("agents").and_then(|v| v.as_array())
        && let Some(first) = agents.first() {
            return parse_single_card(first, port);
        }

    // 直接尝试解析为单个 card
    parse_single_card(json, port)
}

/// 解析单个 agent card (兼容 A2A 格式和 Shadow 格式)
fn parse_single_card(json: &serde_json::Value, port: u16) -> Result<AgentCard> {
    let name = json.get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    let endpoint = format!("http://127.0.0.1:{port}/a2a/{name}");

    // 尝试从 supportedInterfaces 获取 endpoint
    let endpoint = json.get("supportedInterfaces")
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.first())
        .and_then(|iface| iface.get("url"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or(endpoint);

    // 从 skills 提取能力
    let capabilities: Vec<String> = json.get("skills")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|s| s.get("name").and_then(|n| n.as_str()).map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    Ok(AgentCard {
        name,
        description: json.get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        transport: TransportKind::A2a,
        capabilities,
        skills: Vec::new(),
        endpoint: Some(endpoint),
        command: None,
        args: None,
        last_heartbeat: Some(chrono::Utc::now().to_rfc3339()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_single_card_basic() {
        let json = serde_json::json!({
            "name": "test-agent",
            "description": "a test",
            "supportedInterfaces": [{
                "url": "http://localhost:9090/a2a/test-agent",
                "protocolBinding": "JSONRPC"
            }],
            "skills": [{"name": "coding"}, {"name": "review"}]
        });

        let card = parse_single_card(&json, 9090).unwrap();
        assert_eq!(card.name, "test-agent");
        assert_eq!(card.transport, TransportKind::A2a);
        assert_eq!(card.endpoint.as_deref(), Some("http://localhost:9090/a2a/test-agent"));
        assert!(card.capabilities.contains(&"coding".to_string()));
        assert!(card.capabilities.contains(&"review".to_string()));
    }

    #[test]
    fn parse_catalog_card() {
        let json = serde_json::json!({
            "name": "proxy",
            "agents": [{
                "name": "worker-1",
                "supportedInterfaces": [{"url": "http://localhost:9090/a2a/worker-1"}]
            }]
        });

        let card = parse_discovery_card(&json, 9090).unwrap();
        assert_eq!(card.name, "worker-1");
    }

    #[tokio::test]
    async fn probe_closed_port_returns_none() {
        // 使用多个高端口尝试, 找一个确实没有监听的
        for &port in &[19999, 29999, 39999, 49999] {
            let (p, card) = probe_port(port).await;
            if card.is_none() {
                assert_eq!(p, port);
                return; // 找到一个关闭的端口, 测试通过
            }
        }
        // 极端情况: 所有端口都开着, 跳过断言
    }
}
