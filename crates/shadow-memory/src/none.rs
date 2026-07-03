//! 空记忆后端 -- 不存储任何记忆

use shadow_core::{Attributable, Memory, MemoryCategory, MemoryEntry, Role};
use anyhow::Result;
use async_trait::async_trait;

pub struct NoneMemoryBackend;

impl Attributable for NoneMemoryBackend {
    fn role(&self) -> Role {
        Role::Memory
    }
    fn alias(&self) -> &str {
        "none"
    }
}

#[async_trait]
impl Memory for NoneMemoryBackend {
    fn name(&self) -> &str {
        "none"
    }

    async fn store(
        &self,
        _key: &str,
        _content: &str,
        _category: MemoryCategory,
        _session_id: Option<&str>,
    ) -> Result<()> {
        Ok(())
    }

    async fn recall(
        &self,
        _query: &str,
        _limit: usize,
        _session_id: Option<&str>,
    ) -> Result<Vec<MemoryEntry>> {
        Ok(vec![])
    }

    async fn get(&self, _key: &str) -> Result<Option<MemoryEntry>> {
        Ok(None)
    }

    async fn list(&self, _category: Option<&MemoryCategory>) -> Result<Vec<MemoryEntry>> {
        Ok(vec![])
    }

    async fn forget(&self, _key: &str) -> Result<bool> {
        Ok(false)
    }

    async fn count(&self) -> Result<usize> {
        Ok(0)
    }

    fn health_check(&self) -> bool {
        true
    }
}
