use crate::markdown::MarkdownMemory;
use async_trait::async_trait;
use shadow_core::{
    Attributable, Memory, MemoryCategory, MemoryEntry, Role,
};
use shadow_core::kennel::attribution::MemoryKind;

pub struct MarkdownPeer {
    pub alias: String,
    pub memory: MarkdownMemory,
}

pub struct AgentScopedMarkdownMemory {
    own_alias: String,
    own: MarkdownMemory,
    peers: Vec<MarkdownPeer>,
}

impl AgentScopedMarkdownMemory {
    pub fn new(
        own_alias: impl Into<String>,
        own: MarkdownMemory,
        peers: Vec<MarkdownPeer>,
    ) -> Self {
        Self {
            own_alias: own_alias.into(),
            own,
            peers,
        }
    }

    fn attribute(alias: &str, mut entries: Vec<MemoryEntry>) -> Vec<MemoryEntry> {
        for entry in &mut entries {
            entry.key = format!("[{alias}]{}", entry.key);
            entry.agent_alias = Some(alias.to_string());
            entry.agent_id = Some(alias.to_string());
        }

        entries
    }

    fn stamp_attribution(alias: &str, mut entries: Vec<MemoryEntry>) -> Vec<MemoryEntry> {
        for entry in &mut entries {
            entry.agent_alias = Some(alias.to_string());
            entry.agent_id = Some(alias.to_string());
        }

        entries
    }
}

impl Attributable for AgentScopedMarkdownMemory {
    fn role(&self) -> Role {
        Role::Memory(MemoryKind::AgentScopedMarkdown)
    }

    fn alias(&self) -> &str {
        &self.own_alias
    }
}

#[async_trait]
impl Memory for AgentScopedMarkdownMemory {
    fn name(&self) -> &str {
        self.own.name()
    }

    async fn store(
        &self,
        key: &str,
        content: &str,
        category: MemoryCategory,
        session_id: Option<&str>,
    ) -> anyhow::Result<()> {
        self.own.store(key, content, category, session_id).await
    }

    async fn recall(
        &self,
        query: &str,
        limit: usize,
        session_id: Option<&str>,
        since: Option<&str>,
        until: Option<&str>,
    ) -> anyhow::Result<Vec<MemoryEntry>> {
        let mut merged = Self::attribute(
            &self.own_alias,
            self.own
                .recall(query, limit, session_id, since, until)
                .await?,
        );
        for peer in &self.peers {
            match peer
                .memory
                .recall(query, limit, session_id, since, until)
                .await
            {
                Ok(res) => {
                    merged.extend(Self::attribute(&peer.alias, res));
                }
                Err(err) => {
                    // todo record
                }
            }
        }
        merged.truncate(limit);
        Ok(merged)
    }

    async fn get(&self, key: &str) -> anyhow::Result<Option<MemoryEntry>> {
        let entry = self.own.get(key).await?;

        Ok(entry.map(|mut me| {
            me.agent_alias = Some(self.own_alias.clone());
            me.agent_id = Some(self.own_alias.clone());
            me
        }))
    }

    async fn list(
        &self,
        category: Option<&MemoryCategory>,
        session_id: Option<&str>,
    ) -> anyhow::Result<Vec<MemoryEntry>> {
        let entries = self.own.list(category, session_id).await?;
        Ok(Self::stamp_attribution(&self.own_alias, entries))
    }

    async fn forget(&self, key: &str) -> anyhow::Result<bool> {
        self.own.forget(key).await
    }

    async fn forget_for_agent(&self, key: &str, agent_id: &str) -> anyhow::Result<bool> {
        self.own.forget_for_agent(key, agent_id).await
    }

    async fn count(&self) -> anyhow::Result<usize> {
        self.own.count().await
    }

    async fn health_check(&self) -> bool {
        self.own.health_check().await
    }

    async fn store_with_metadata(
        &self,
        key: &str,
        content: &str,
        category: MemoryCategory,
        session_id: Option<&str>,
        namespace: Option<&str>,
        importance: Option<f64>,
    ) -> anyhow::Result<()> {
        self.own
            .store_with_metadata(key, content, category, session_id, namespace, importance)
            .await
    }

    async fn store_with_agent(
        &self,
        key: &str,
        content: &str,
        category: MemoryCategory,
        session_id: Option<&str>,
        namespace: Option<&str>,
        importance: Option<f64>,
        agent_id: Option<&str>,
    ) -> anyhow::Result<()> {
        self.own
            .store_with_metadata(key, content, category, session_id, namespace, importance)
            .await
    }

    async fn recall_for_agents(
        &self,
        allowed_agent_ids: &[&str],
        query: &str,
        limit: usize,
        session_id: Option<&str>,
        since: Option<&str>,
        until: Option<&str>,
    ) -> anyhow::Result<Vec<MemoryEntry>> {
        if allowed_agent_ids.is_empty() {
            return self.recall(query, limit, session_id, since, until).await;
        }

        let mut merged = Vec::new();

        if allowed_agent_ids.contains(&self.own_alias.as_str()) {
            merged.extend(Self::attribute(
                &self.own_alias,
                self.own
                    .recall(query, limit, session_id, since, until)
                    .await?,
            ));
        }
        for peer in &self.peers {
            let res = peer
                .memory
                .recall(query, limit, session_id, since, until)
                .await?;
            merged.extend(Self::attribute(&peer.alias, res));
        }
        merged.truncate(limit);
        Ok(merged)
    }
}
