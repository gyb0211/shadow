//! Markdown 记忆后端 -- 每条记忆存为一个 .md 文件

use agent_core::{Attributable, Memory, MemoryEntry, Role};
use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use std::path::{Path, PathBuf};

pub struct MarkdownMemory {
    dir: PathBuf,
}

impl MarkdownMemory {
    pub fn new(workspace_dir: &Path) -> Self {
        let dir = workspace_dir.join("memory");
        let _ = std::fs::create_dir_all(&dir);
        Self { dir }
    }

    fn entry_path(&self, key: &str) -> PathBuf {
        // 安全文件名: 只保留字母数字和 -
        let safe: String = key
            .chars()
            .map(|c| if c.is_alphanumeric() || c == '-' { c } else { '-' })
            .collect();
        self.dir.join(format!("{safe}.md"))
    }
}

impl Attributable for MarkdownMemory {
    fn role(&self) -> Role {
        Role::Memory
    }
    fn alias(&self) -> &str {
        "markdown"
    }
}

#[async_trait]
impl Memory for MarkdownMemory {
    async fn store(&self, entry: &MemoryEntry) -> Result<()> {
        let path = self.entry_path(&entry.key);
        let content = format!(
            "---\nid: {}\nkey: {}\ncategory: {}\ntimestamp: {}\n---\n{}",
            entry.id, entry.key, entry.category, entry.timestamp, entry.content
        );
        tokio::fs::write(&path, content).await?;
        Ok(())
    }

    async fn recall(&self, query: &str, limit: usize) -> Result<Vec<MemoryEntry>> {
        let query_lower = query.to_lowercase();
        let mut entries = self.list().await?;
        // 简单关键词匹配
        entries.retain(|e| {
            e.content.to_lowercase().contains(&query_lower)
                || e.key.to_lowercase().contains(&query_lower)
        });
        entries.truncate(limit);
        Ok(entries)
    }

    async fn get(&self, key: &str) -> Result<Option<MemoryEntry>> {
        let path = self.entry_path(key);
        if !path.exists() {
            return Ok(None);
        }
        let content = tokio::fs::read_to_string(&path).await?;
        // 简单解析: 跳过 frontmatter, 取 body
        let body = content
            .split("---")
            .nth(2)
            .unwrap_or("")
            .trim();
        Ok(Some(MemoryEntry {
            id: uuid::Uuid::new_v4().to_string(),
            key: key.to_string(),
            content: body.to_string(),
            category: "general".to_string(),
            timestamp: Utc::now(),
            session_id: None,
            agent_alias: None,
        }))
    }

    async fn list(&self) -> Result<Vec<MemoryEntry>> {
        let mut entries = Vec::new();
        if !self.dir.exists() {
            return Ok(entries);
        }
        let mut rd = tokio::fs::read_dir(&self.dir).await?;
        while let Some(entry) = rd.next_entry().await? {
            let path = entry.path();
            if path.extension().is_none_or(|ext| ext != "md") {
                continue;
            }
            let key = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string();
            let content = tokio::fs::read_to_string(&path).await?;
            let body = content.split("---").nth(2).unwrap_or("").trim().to_string();
            entries.push(MemoryEntry {
                id: uuid::Uuid::new_v4().to_string(),
                key,
                content: body,
                category: "general".to_string(),
                timestamp: Utc::now(),
                session_id: None,
                agent_alias: None,
            });
        }
        Ok(entries)
    }

    async fn forget(&self, key: &str) -> Result<()> {
        let path = self.entry_path(key);
        if path.exists() {
            tokio::fs::remove_file(&path).await?;
        }
        Ok(())
    }
}
