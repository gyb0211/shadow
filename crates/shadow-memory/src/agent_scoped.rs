use shadow_core::Memory;
use std::collections::HashSet;
use std::sync::Arc;

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
