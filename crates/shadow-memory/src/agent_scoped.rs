//! Runtime memory wrapper bound to one agent.
//!
//! Each agent holds its own per-agent backend instance (selected at
//! agent creation via `[agents.<alias>.memory.backend]`, immutable
//! thereafter). The wrapper sits directly on top of that instance and:
//!
//! - Stamps the bound agent's UUID on every store via the inner
//!   backend's `store_with_agent` trait method (real implementations
//!   on every backend; the agent_id is never silently dropped at the
//!   trait boundary).
//! - Filters every recall through the inner backend's
//!   `recall_for_agents` with the resolved allowlist (own UUID + the
//!   `read_memory_from` allowlist from
//!   `[agents.<alias>.workspace.read_memory_from]`).
//! - Intersects caller-supplied per-call allowlists with the bound
//!   allowlist so a caller can never widen scope past what the agent's
//!   config permits.
//!
//! Cross-backend allowlist entries are rejected at config load. The
//! wrapper only ever sees same-backend sibling UUIDs in its
//! `allowed_agent_ids` set.

use shadow_core::{
    Attributable, ExportFilter, Memory, MemoryCategory, MemoryEntry, ProceduralMessage, Role,
};
use shadow_core::kennel::attribution::MemoryKind;
use anyhow::Result;
use async_trait::async_trait;
use std::collections::HashSet;
use std::sync::Arc;

/// A `Memory` impl that scopes every read and write to a bound agent's
/// UUID + a resolved cross-agent allowlist.
///
/// Construct via [`AgentScopedMemory::new`] at agent-loop entry. The
/// runtime holds one per agent. Non-generic over the inner backend
/// (holds `Arc<dyn Memory>`) so the per-agent factory can hand back a
/// single concrete type regardless of the agent's chosen backend kind.
pub struct AgentScopedMemory {
    /// The wrapped backend.
    inner: Arc<dyn Memory>,
    /// The bound agent's UUID.
    agent_id: String,
    /// Set of agent UUIDs this wrapper recalls from.
    allowed_agent_ids: HashSet<String>,
}

impl AgentScopedMemory {
    pub fn new(
        inner: Arc<dyn Memory>,
        agent_id: impl Into<String>,
        allowed_sibling_agent_ids: impl IntoIterator<Item = String>,
    ) -> Self {
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
        &self.agent_id
    }
}

#[async_trait]
impl Memory for AgentScopedMemory {
    fn name(&self) -> &str {
        self.inner.name()
    }

    async fn health_check(&self) -> bool {
        self.inner.health_check().await
    }

    async fn store(
        &self,
        key: &str,
        content: &str,
        category: MemoryCategory,
        session_id: Option<&str>,
    ) -> Result<()> {
        self.inner
            .store_with_agent(
                key,
                content,
                category,
                session_id,
                None,
                None,
                Some(&self.agent_id),
            )
            .await
    }

    async fn store_with_metadata(
        &self,
        key: &str,
        content: &str,
        category: MemoryCategory,
        session_id: Option<&str>,
        namespace: Option<&str>,
        importance: Option<f64>,
    ) -> Result<()> {
        self.inner
            .store_with_agent(
                key,
                content,
                category,
                session_id,
                namespace,
                importance,
                Some(&self.agent_id),
            )
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
    ) -> Result<()> {
        if let Some(requested) = agent_id
            && requested != self.agent_id
        {
            anyhow::bail!(
                "AgentScopedMemory refuses store_with_agent for foreign agent_id; use a wrapper bound to the target agent"
            );
        }
        self.inner
            .store_with_agent(
                key,
                content,
                category,
                session_id,
                namespace,
                importance,
                Some(&self.agent_id),
            )
            .await
    }

    async fn recall(
        &self,
        query: &str,
        limit: usize,
        session_id: Option<&str>,
        since: Option<&str>,
        until: Option<&str>,
    ) -> Result<Vec<MemoryEntry>> {
        let allowed = self.allowed_slice();
        self.inner
            .recall_for_agents(&allowed, query, limit, session_id, since, until)
            .await
    }

    async fn recall_for_agents(
        &self,
        caller_allowed: &[&str],
        query: &str,
        limit: usize,
        session_id: Option<&str>,
        since: Option<&str>,
        until: Option<&str>,
    ) -> Result<Vec<MemoryEntry>> {
        if caller_allowed.is_empty() {
            let bound: Vec<&str> = self.allowed_agent_ids.iter().map(String::as_str).collect();
            return self
                .inner
                .recall_for_agents(&bound, query, limit, session_id, since, until)
                .await;
        }

        let intersected: Vec<&str> = caller_allowed
            .iter()
            .copied()
            .filter(|id| self.allowed_agent_ids.contains(*id))
            .collect();
        if intersected.is_empty() {
            return Ok(Vec::new());
        }
        self.inner
            .recall_for_agents(&intersected, query, limit, session_id, since, until)
            .await
    }

    async fn get(&self, key: &str) -> Result<Option<MemoryEntry>> {
        if let Some(own) = self.inner.get_for_agent(key, &self.agent_id).await? {
            return Ok(Some(own));
        }
        for sibling in &self.allowed_agent_ids {
            if sibling == &self.agent_id {
                continue;
            }
            if let Some(hit) = self.inner.get_for_agent(key, sibling).await? {
                return Ok(Some(hit));
            }
        }
        Ok(None)
    }

    async fn get_for_agent(&self, key: &str, agent_id: &str) -> Result<Option<MemoryEntry>> {
        if agent_id != self.agent_id && !self.allowed_agent_ids.iter().any(|a| a == agent_id) {
            return Ok(None);
        }
        self.inner.get_for_agent(key, agent_id).await
    }

    async fn list(
        &self,
        category: Option<&MemoryCategory>,
        session_id: Option<&str>,
    ) -> Result<Vec<MemoryEntry>> {
        let entries = self.inner.list(category, session_id).await?;
        Ok(entries
            .into_iter()
            .filter(|e| {
                e.agent_id
                    .as_deref()
                    .is_some_and(|aid| self.allowed_agent_ids.contains(aid))
            })
            .collect())
    }

    async fn forget(&self, key: &str) -> Result<bool> {
        if self.inner.forget_for_agent(key, &self.agent_id).await? {
            return Ok(true);
        }
        match self.inner.get(key).await? {
            None => Ok(false),
            Some(entry) => match entry.agent_id.as_deref() {
                Some(_other) => {
                    anyhow::bail!(
                        "AgentScopedMemory refuses to forget cross-agent row: key attributed to agent other than the bound agent"
                    );
                }
                None => {
                    anyhow::bail!(
                        "AgentScopedMemory refuses to forget unattributed row: legacy or backend without per-agent tracking; resolve via an admin Memory handle"
                    );
                }
            },
        }
    }

    async fn forget_for_agent(&self, key: &str, agent_id: &str) -> Result<bool> {
        if agent_id != self.agent_id {
            anyhow::bail!(
                "AgentScopedMemory refuses cross-agent forget_for_agent: bound agent and target agent differ"
            );
        }
        self.inner.forget_for_agent(key, agent_id).await
    }

    async fn purge_namespace(&self, _namespace: &str) -> Result<usize> {
        anyhow::bail!(
            "AgentScopedMemory refuses purge_namespace: cross-agent bulk delete must run through an admin Memory handle"
        );
    }

    async fn purge_session(&self, session_id: &str) -> Result<usize> {
        self.inner
            .purge_session_for_agent(session_id, &self.agent_id)
            .await
    }

    async fn purge_session_for_agent(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<usize> {
        if agent_id != self.agent_id && !self.allowed_agent_ids.iter().any(|a| a == agent_id) {
            return Ok(0);
        }
        self.inner
            .purge_session_for_agent(session_id, agent_id)
            .await
    }

    async fn purge_agent(&self, _agent_alias: &str) -> Result<usize> {
        anyhow::bail!(
            "AgentScopedMemory refuses purge_agent: bulk agent deletion must run through an admin Memory handle"
        );
    }

    async fn export_agent(&self, _agent_alias: &str) -> Result<Vec<MemoryEntry>> {
        Ok(Vec::new())
    }

    async fn rename_agent(&self, _from: &str, _to: &str) -> Result<usize> {
        anyhow::bail!(
            "AgentScopedMemory refuses rename_agent: bulk rename must run through an admin Memory handle"
        );
    }

    async fn count_agent(&self, _agent_alias: &str) -> Result<usize> {
        Ok(0)
    }

    async fn count(&self) -> Result<usize> {
        let entries = self.inner.list(None, None).await?;
        Ok(entries
            .into_iter()
            .filter(|e| {
                e.agent_id
                    .as_deref()
                    .is_some_and(|aid| self.allowed_agent_ids.contains(aid))
            })
            .count())
    }

    async fn supersede(&self, _superseded_ids: &[String], _new_id: &str) -> Result<()> {
        Ok(())
    }

    async fn store_procedural(
        &self,
        messages: &[ProceduralMessage],
        _session_id: &str,
    ) -> Result<()> {
        self.inner.store_procedural(messages, _session_id).await
    }

    async fn count_in_scope(
        &self,
        _namespace: Option<&str>,
        _category: Option<&MemoryCategory>,
    ) -> Result<u64> {
        Ok(0)
    }

    async fn stats(&self) -> Result<shadow_core::MemoryStats> {
        Ok(shadow_core::MemoryStats::default())
    }

    async fn reindex(&self) -> Result<usize> {
        self.inner.reindex().await
    }

    async fn refresh_embedder(
        &self,
        _model_provider: &str,
        _api_key: Option<&str>,
        _model: &str,
        _dimensions: usize,
    ) {
    }

    async fn recall_namespaced(
        &self,
        namespace: &str,
        query: &str,
        limit: usize,
        session_id: Option<&str>,
        since: Option<&str>,
        until: Option<&str>,
    ) -> Result<Vec<MemoryEntry>> {
        let entries = self
            .recall(query, limit * 2, session_id, since, until)
            .await?;
        Ok(entries
            .into_iter()
            .filter(|e| e.namespace == namespace)
            .take(limit)
            .collect())
    }

    async fn export(&self, filter: &ExportFilter) -> Result<Vec<MemoryEntry>> {
        let entries = self
            .list(filter.category.as_ref(), filter.session_id.as_deref())
            .await?;
        Ok(entries
            .into_iter()
            .filter(|e| {
                if let Some(ref ns) = filter.namespace
                    && e.namespace != *ns
                {
                    return false;
                }
                if let Some(ref since) = filter.since
                    && e.timestamp.as_str() < since.as_str()
                {
                    return false;
                }
                if let Some(ref until) = filter.until
                    && e.timestamp.as_str() > until.as_str()
                {
                    return false;
                }
                true
            })
            .collect())
    }

    async fn ensure_agent_uuid(&self, alias: &str) -> Result<String> {
        self.inner.ensure_agent_uuid(alias).await
    }
}
