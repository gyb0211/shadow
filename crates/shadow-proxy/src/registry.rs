//! Agent 注册表 -- 管理已注册 agent 的 AgentCard

use std::collections::HashMap;
use parking_lot::RwLock;
use anyhow::Result;

use crate::card::{AgentCard, TransportKind};

/// Agent 注册表 -- 线程安全
pub struct AgentRegistry {
    agents: RwLock<HashMap<String, AgentCard>>,
}

impl AgentRegistry {
    pub fn new() -> Self {
        Self {
            agents: RwLock::new(HashMap::new()),
        }
    }

    /// 注册或更新 agent
    pub fn register(&self, card: AgentCard) -> Result<()> {
        let name = card.name.clone();
        let mut agents = self.agents.write();
        agents.insert(name, card);
        Ok(())
    }

    /// 注销 agent
    pub fn deregister(&self, name: &str) -> bool {
        let mut agents = self.agents.write();
        agents.remove(name).is_some()
    }

    /// 查找 agent
    pub fn get(&self, name: &str) -> Option<AgentCard> {
        let agents = self.agents.read();
        agents.get(name).cloned()
    }

    /// 列出所有 agent
    pub fn list(&self) -> Vec<AgentCard> {
        let agents = self.agents.read();
        agents.values().cloned().collect()
    }

    /// 按能力查找 agent
    pub fn find_by_capability(&self, capability: &str) -> Vec<AgentCard> {
        let agents = self.agents.read();
        agents
            .values()
            .filter(|card| card.has_capability(capability))
            .cloned()
            .collect()
    }

    /// 按传输类型筛选
    pub fn find_by_transport(&self, kind: &TransportKind) -> Vec<AgentCard> {
        let agents = self.agents.read();
        agents
            .values()
            .filter(|card| &card.transport == kind)
            .cloned()
            .collect()
    }

    /// 清理超时未心跳的 agent (秒)
    pub fn cleanup_stale(&self, max_age_secs: i64) -> usize {
        let now = chrono::Utc::now();
        let mut agents = self.agents.write();
        let stale_names: Vec<String> = agents
            .iter()
            .filter_map(|(name, card)| {
                if let Some(hb) = &card.last_heartbeat
                    && let Ok(ts) = chrono::DateTime::parse_from_rfc3339(hb) {
                        let elapsed = now.signed_duration_since(ts).num_seconds();
                        if elapsed > max_age_secs {
                            return Some(name.clone());
                        }
                    }
                None
            })
            .collect();
        let count = stale_names.len();
        for name in &stale_names {
            agents.remove(name);
        }
        count
    }

    /// 已注册 agent 数量
    pub fn count(&self) -> usize {
        self.agents.read().len()
    }
}

impl Default for AgentRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_and_get() {
        let reg = AgentRegistry::new();
        let card = AgentCard::local("test", vec!["coding".into()]);
        reg.register(card).unwrap();

        assert_eq!(reg.count(), 1);
        assert!(reg.get("test").is_some());
        assert!(reg.get("missing").is_none());
    }

    #[test]
    fn deregister() {
        let reg = AgentRegistry::new();
        reg.register(AgentCard::local("a", vec![])).unwrap();
        assert!(reg.deregister("a"));
        assert!(!reg.deregister("a"));
        assert_eq!(reg.count(), 0);
    }

    #[test]
    fn find_by_capability() {
        let reg = AgentRegistry::new();
        reg.register(AgentCard::local("coder", vec!["coding".into(), "review".into()])).unwrap();
        reg.register(AgentCard::local("writer", vec!["writing".into()])).unwrap();

        let coders = reg.find_by_capability("coding");
        assert_eq!(coders.len(), 1);
        assert_eq!(coders[0].name, "coder");
    }

    #[test]
    fn list_all() {
        let reg = AgentRegistry::new();
        reg.register(AgentCard::local("a", vec![])).unwrap();
        reg.register(AgentCard::local("b", vec![])).unwrap();
        assert_eq!(reg.list().len(), 2);
    }

    #[test]
    fn cleanup_stale() {
        let reg = AgentRegistry::new();
        let mut card = AgentCard::local("stale", vec![]);
        // 设置心跳为 2 小时前
        let old = chrono::Utc::now() - chrono::Duration::hours(2);
        card.last_heartbeat = Some(old.to_rfc3339());
        reg.register(card).unwrap();

        reg.register(AgentCard::local("fresh", vec![])).unwrap();

        let removed = reg.cleanup_stale(3600); // 1 hour threshold
        assert_eq!(removed, 1);
        assert_eq!(reg.count(), 1);
        assert!(reg.get("fresh").is_some());
    }
}
