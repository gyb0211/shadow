//! 空记忆后端 -- 不存储任何记忆

use shadow_core::{Attributable, Memory, MemoryCategory, MemoryEntry, MemoryKind, Role};
use anyhow::Result;
use async_trait::async_trait;

#[derive(Debug, Default, Clone)]
pub struct NoneMemory{
    alias: String,
}


impl Attributable for NoneMemory {
    fn role(&self) -> Role {
        Role::Memory(MemoryKind::None)
    }
    fn alias(&self) -> &str {
        &self.alias
    }
}

#[async_trait]
impl Memory for NoneMemory {
    fn name(&self) -> &str {
        "none"
    }

    async fn store(&self, key: &str, content: &str, category: MemoryCategory, session_id: Option<&str>) -> Result<()> {
        Ok(())
    }

    async fn recall(&self, query: &str, limit: usize, session_id: Option<&str>, since: Option<&str>, until: Option<&str>) -> Result<Vec<MemoryEntry>> {
        Ok(vec![])
    }

    async fn get(&self, key: &str) -> Result<Option<MemoryEntry>> {
        Ok(None)
    }

    async fn list(&self, category: Option<&MemoryCategory>, session_id: Option<&str>) -> Result<Vec<MemoryEntry>> {
        Ok(vec![])
    }

    async fn forget(&self, key: &str) -> Result<bool> {
        Ok(false)
    }

    async fn forget_for_agent(&self, key: &str) -> Result<bool> {
        Ok(false)
    }

    async fn count(&self) -> Result<usize> {
        Ok(0)
    }

    async fn health_check(&self) -> bool {
       true
    }

    async fn store_with_agent(&self, key: &str, content: &str, category: MemoryCategory, session_id: Option<&str>, _namespace: Option<&str>, _importance: Option<f64>, agent_id: Option<&str>) -> Result<()> {
     Ok(())
    }

    async fn recall_for_agent(&self, allowed_agent_ids: &[&str], query: &str, limit: usize, session_id: Option<&str>, since: Option<&str>, until: Option<&str>) -> Result<Vec<MemoryEntry>> {
        Ok(vec![])
    }
}
