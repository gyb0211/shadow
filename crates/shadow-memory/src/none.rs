//! 空记忆后端 -- 不存储任何记忆

use agent_core::{Attributable, Memory, MemoryEntry, Role};
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
    async fn store(&self, _entry: &MemoryEntry) -> Result<()> {
        Ok(())
    }
    async fn recall(&self, _query: &str, _limit: usize) -> Result<Vec<MemoryEntry>> {
        Ok(vec![])
    }
    async fn get(&self, _key: &str) -> Result<Option<MemoryEntry>> {
        Ok(None)
    }
    async fn list(&self) -> Result<Vec<MemoryEntry>> {
        Ok(vec![])
    }
    async fn forget(&self, _key: &str) -> Result<()> {
        Ok(())
    }
}
