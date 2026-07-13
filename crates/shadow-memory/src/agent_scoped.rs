use shadow_core::{Attributable, Memory, MemoryCategory, MemoryEntry, MemoryKind, Role};
use std::collections::HashSet;
use std::sync::Arc;
use async_trait::async_trait;

pub struct AgentScopedMemory {
    inner: Arc<dyn Memory>,
    agent_id: String,
    allowed_agent_ids: HashSet<String>,
}

impl AgentScopedMemory {
    pub fn new(
        inner: Arc<dyn Memory>,
        agent_id: impl Into<String>,
        allowed_sibling_agent_ids: impl IntoIterator<Item = String>,
    )->Self {
        let agent_id = agent_id.into();
        let mut allowed_agent_ids: HashSet<String> =
            allowed_sibling_agent_ids.into_iter().collect();
        allowed_agent_ids.insert(agent_id.clone());
        Self {
            inner,
            agent_id,
            allowed_agent_ids,
        }
    }
    
    fn allowed_slice(&self) -> Vec<&str> {
        self.allowed_agent_ids.iter().map(String::as_str).collect()
    }
}

impl Attributable for AgentScopedMemory {
    fn role(&self) -> Role {
        Role::Memory(MemoryKind::AgentScoped)
    }

    fn alias(&self) -> &str {
        self.inner.alias()
    }
}

#[async_trait]
impl Memory for AgentScopedMemory {
    fn name(&self) -> &str {
        todo!()
    }

    async fn store(&self, key: &str, content: &str, category: MemoryCategory, session_id: Option<&str>) -> anyhow::Result<()> {
        todo!()
    }

    async fn recall(&self, query: &str, limit: usize, session_id: Option<&str>, since: Option<&str>, until: Option<&str>) -> anyhow::Result<Vec<MemoryEntry>> {
        todo!()
    }

    async fn get(&self, key: &str) -> anyhow::Result<Option<MemoryEntry>> {
        todo!()
    }

    async fn list(&self, category: Option<&MemoryCategory>, session_id: Option<&str>) -> anyhow::Result<Vec<MemoryEntry>> {
        todo!()
    }

    async fn forget(&self, key: &str) -> anyhow::Result<bool> {
        todo!()
    }

    async fn forget_for_agent(&self, key: &str) -> anyhow::Result<bool> {
        todo!()
    }

    async fn count(&self) -> anyhow::Result<usize> {
        todo!()
    }

    async fn health_check(&self) -> bool {
        todo!()
    }

    async fn store_with_agent(&self, key: &str, content: &str, category: MemoryCategory, session_id: Option<&str>, _namespace: Option<&str>, _importance: Option<f64>, agent_id: Option<&str>) -> anyhow::Result<()> {
        todo!()
    }

    async fn recall_for_agent(&self, allowed_agent_ids: &[&str], query: &str, limit: usize, session_id: Option<&str>, since: Option<&str>, until: Option<&str>) -> anyhow::Result<Vec<MemoryEntry>> {
        todo!()
    }
}