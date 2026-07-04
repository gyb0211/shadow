//! Agent 卡片 -- 注册发现的数据结构

use serde::{Deserialize, Serialize};

/// 传输协议类型
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TransportKind {
    /// 进程内 (同 Shadow 实例)
    Local,
    /// ACP 子进程 (stdio JSON-RPC)
    Acp,
    /// A2A 远程 (HTTP JSON-RPC)
    A2a,
}

/// Agent 能力声明
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSkill {
    /// 技能 ID
    pub id: String,
    /// 技能名称
    pub name: String,
    /// 技能描述
    pub description: String,
}

/// Agent 卡片 -- 注册到 registry 的元数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentCard {
    /// agent 名称 (唯一标识)
    pub name: String,
    /// 描述
    #[serde(default)]
    pub description: String,
    /// 传输协议
    pub transport: TransportKind,
    /// 能力标签 (用于路由): ["coding", "review", "research"]
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// 技能列表
    #[serde(default)]
    pub skills: Vec<AgentSkill>,
    /// A2A endpoint (仅 transport=a2a): "http://host:port/a2a/agent_name"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    /// ACP 命令 (仅 transport=acp): "claude"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// ACP 参数 (仅 transport=acp): ["--acp", "--stdio"]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args: Option<Vec<String>>,
    /// 最后心跳时间 (RFC3339)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_heartbeat: Option<String>,
}

impl AgentCard {
    /// 创建本地 agent 卡片
    pub fn local(name: &str, capabilities: Vec<String>) -> Self {
        Self {
            name: name.to_string(),
            description: String::new(),
            transport: TransportKind::Local,
            capabilities,
            skills: Vec::new(),
            endpoint: None,
            command: None,
            args: None,
            last_heartbeat: Some(chrono::Utc::now().to_rfc3339()),
        }
    }

    /// 创建 ACP agent 卡片
    pub fn acp(name: &str, command: &str, args: Vec<String>, capabilities: Vec<String>) -> Self {
        Self {
            name: name.to_string(),
            description: String::new(),
            transport: TransportKind::Acp,
            capabilities,
            skills: Vec::new(),
            endpoint: None,
            command: Some(command.to_string()),
            args: Some(args),
            last_heartbeat: Some(chrono::Utc::now().to_rfc3339()),
        }
    }

    /// 创建 A2A agent 卡片
    pub fn a2a(name: &str, endpoint: &str, capabilities: Vec<String>) -> Self {
        Self {
            name: name.to_string(),
            description: String::new(),
            transport: TransportKind::A2a,
            capabilities,
            skills: Vec::new(),
            endpoint: Some(endpoint.to_string()),
            command: None,
            args: None,
            last_heartbeat: Some(chrono::Utc::now().to_rfc3339()),
        }
    }

    /// 检查是否有指定能力
    pub fn has_capability(&self, cap: &str) -> bool {
        self.capabilities.iter().any(|c| c == cap)
    }

    /// 刷新心跳
    pub fn touch(&mut self) {
        self.last_heartbeat = Some(chrono::Utc::now().to_rfc3339());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn card_local() {
        let card = AgentCard::local("researcher", vec!["research".into()]);
        assert_eq!(card.transport, TransportKind::Local);
        assert!(card.has_capability("research"));
        assert!(!card.has_capability("coding"));
    }

    #[test]
    fn card_acp() {
        let card = AgentCard::acp("claude", "claude", vec!["--acp".into(), "--stdio".into()], vec!["coding".into()]);
        assert_eq!(card.transport, TransportKind::Acp);
        assert_eq!(card.command.as_deref(), Some("claude"));
    }

    #[test]
    fn card_a2a() {
        let card = AgentCard::a2a("remote", "http://host:9090/a2a/remote", vec!["coding".into()]);
        assert_eq!(card.transport, TransportKind::A2a);
        assert_eq!(card.endpoint.as_deref(), Some("http://host:9090/a2a/remote"));
    }

    #[test]
    fn card_serialize_roundtrip() {
        let card = AgentCard::a2a("test", "http://localhost", vec!["a".into(), "b".into()]);
        let json = serde_json::to_string(&card).unwrap();
        let back: AgentCard = serde_json::from_str(&json).unwrap();
        assert_eq!(card.name, back.name);
        assert_eq!(card.transport, back.transport);
        assert_eq!(card.capabilities, back.capabilities);
    }
}
