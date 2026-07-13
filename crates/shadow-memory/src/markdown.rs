use async_trait::async_trait;
use chrono::{DateTime, FixedOffset, Local, NaiveDate};
use shadow_core::kennel::memory::is_recent_recall_query;
use shadow_core::{Attributable, Memory, MemoryCategory, MemoryEntry, Role};
use shadow_core::kennel::attribution::MemoryKind;
use std::path::{Path, PathBuf};

pub struct MarkdownMemory {
    alias: String,
    workspace_dir: PathBuf,
}

impl MarkdownMemory {
    pub fn new(alias: &str, workspace_dir: &Path) -> Self {
        Self {
            alias: alias.to_string(),
            workspace_dir: workspace_dir.to_path_buf(),
        }
    }

    fn memory_dir(&self) -> PathBuf {
        self.workspace_dir.join("memory")
    }

    fn core_path(&self) -> PathBuf {
        self.workspace_dir.join("MEMORY.md")
    }

    fn daily_path(&self) -> PathBuf {
        let date = Local::now().format("%Y-%m-%d").to_string();

        self.memory_dir().join(format!("{date}.md"))
    }

    async fn ensure_dirs(&self) -> anyhow::Result<()> {
        tokio::fs::create_dir_all(self.memory_dir()).await?;
        Ok(())
    }

    async fn append_to_file(&self, path: &Path, content: &str) -> anyhow::Result<()> {
        self.ensure_dirs().await?;

        let existing = if path.exists() {
            tokio::fs::read_to_string(path).await.unwrap_or_default()
        } else {
            String::new()
        };

        let updated = if existing.is_empty() {
            let header = if path == self.core_path() {
                "# Long-Term Memory\n\n"
            } else {
                let date = Local::now().format("%Y-%m-%d").to_string();
                &format!("# Daily Log - {date}\n\n")
            };
            format!("{header}\n{content}\n")
        } else {
            format!("{existing}\n{content}\n")
        };

        tokio::fs::write(path, updated).await?;
        Ok(())
    }

    fn parse_entries_from_file(
        path: &Path,
        content: &str,
        category: &MemoryCategory,
    ) -> Vec<MemoryEntry> {
        let filename = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown");

        content
            .lines()
            .filter(|line| {
                let trimmed = line.trim();
                !trimmed.is_empty() && !trimmed.starts_with('#')
            })
            .enumerate()
            .map(|(i, line)| {
                let trimmed = line.trim();
                let clean = trimmed.strip_prefix("- ").unwrap_or(trimmed);
                MemoryEntry {
                    id: format!("{filename}:{i}"),
                    key: format!("{filename}:{i}"),
                    content: clean.to_string(),
                    category: category.clone(),
                    timestamp: filename.to_string(),
                    session_id: None,
                    score: None,
                    namespace: "default".into(),
                    importance: None,
                    superseded_by: None,
                    kind: None,
                    pinned: false,
                    tenant_id: None,
                    agent_alias: None,
                    agent_id: None,
                }
            })
            .collect()
    }

    async fn read_all_entries(&self) -> anyhow::Result<Vec<MemoryEntry>> {
        let mut entries = Vec::new();

        // Read MEMORY.md (core)
        let core_path = self.core_path();
        if core_path.exists() {
            let content = tokio::fs::read_to_string(&core_path).await?;
            entries.extend(Self::parse_entries_from_file(
                &core_path,
                &content,
                &MemoryCategory::Core,
            ));
        }

        // Read daily logs
        let mem_dir = self.memory_dir();
        if mem_dir.exists() {
            let mut dir = tokio::fs::read_dir(&mem_dir).await?;
            while let Some(entry) = dir.next_entry().await? {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("md") {
                    let content = tokio::fs::read_to_string(&path).await?;
                    entries.extend(Self::parse_entries_from_file(
                        &path,
                        &content,
                        &MemoryCategory::Daily,
                    ));
                }
            }
        }

        entries.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
        Ok(entries)
    }
}

#[async_trait]
impl Memory for MarkdownMemory {
    fn name(&self) -> &str {
        "markdown"
    }

    async fn store(
        &self,
        key: &str,
        content: &str,
        category: MemoryCategory,
        session_id: Option<&str>,
    ) -> anyhow::Result<()> {
        let entry = format!("- **{key}**: {content}");
        let path = match category {
            MemoryCategory::Core => self.core_path(),
            _ => self.daily_path(),
        };
        self.append_to_file(&path, &entry).await
    }

    async fn recall(
        &self,
        query: &str,
        limit: usize,
        session_id: Option<&str>,
        since: Option<&str>,
        until: Option<&str>,
    ) -> anyhow::Result<Vec<MemoryEntry>> {
        let since_dt = since
            .map(chrono::DateTime::parse_from_rfc3339)
            .transpose()
            .map_err(|e| {
                // ::zeroclaw_log::record!(
                //     WARN,
                //     ::zeroclaw_log::Event::new(module_path!(), ::zeroclaw_log::Action::Reject)
                //         .with_outcome(::zeroclaw_log::EventOutcome::Failure)
                //         .with_attrs(
                //             ::serde_json::json!({"field": "since", "error": format!("{}", e)})
                //         ),
                //     "recall window bound rejected"
                // );
                anyhow::Error::msg(format!("invalid 'since' date (expected RFC 3339): {e}"))
            })?;
        let until_dt = until
            .map(chrono::DateTime::parse_from_rfc3339)
            .transpose()
            .map_err(|e| {
                // ::zeroclaw_log::record!(
                //     WARN,
                //     ::zeroclaw_log::Event::new(module_path!(), ::zeroclaw_log::Action::Reject)
                //         .with_outcome(::zeroclaw_log::EventOutcome::Failure)
                //         .with_attrs(
                //             ::serde_json::json!({"field": "until", "error": format!("{}", e)})
                //         ),
                //     "recall window bound rejected"
                // );
                anyhow::Error::msg(format!("invalid 'until' date (expected RFC 3339): {e}"))
            })?;
        if let (Some(s), Some(u)) = (&since_dt, &until_dt)
            && s >= u
        {
            anyhow::bail!("'since' must be before 'until'");
        }

        let all = self.read_all_entries().await?;
        let keywords: Vec<String> = if is_recent_recall_query(query) {
            Vec::new()
        } else {
            query
                .to_lowercase()
                .split_whitespace()
                .map(str::to_string)
                .collect()
        };

        let mut scored: Vec<MemoryEntry> = all
            .into_iter()
            .filter_map(|mut entry| {
                if !entry_in_window(&entry.timestamp, since_dt.as_ref(), until_dt.as_ref()) {
                    return None;
                }
                if keywords.is_empty() {
                    entry.score = Some(1.0);
                    return Some(entry);
                }
                let content_lower = entry.content.to_lowercase();
                let matched = keywords
                    .iter()
                    .filter(|kw| content_lower.contains(kw.as_str()))
                    .count();
                if matched > 0 {
                    #[allow(clippy::cast_precision_loss)]
                    let score = matched as f64 / keywords.len() as f64;
                    entry.score = Some(score);
                    Some(entry)
                } else {
                    None
                }
            })
            .collect();

        scored.sort_by(|a, b| {
            if keywords.is_empty() {
                b.timestamp.as_str().cmp(a.timestamp.as_str())
            } else {
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            }
        });
        scored.truncate(limit);
        Ok(scored)
    }

    async fn get(&self, key: &str) -> anyhow::Result<Option<MemoryEntry>> {
        let all = self.read_all_entries().await?;
        Ok(all
            .into_iter()
            .find(|e| e.key == key || e.content.contains(key)))
    }

    async fn list(
        &self,
        category: Option<&MemoryCategory>,
        session_id: Option<&str>,
    ) -> anyhow::Result<Vec<MemoryEntry>> {
        let all = self.read_all_entries().await?;
        match category {
            Some(cat) => Ok(all.into_iter().filter(|e| &e.category == cat).collect()),
            None => Ok(all),
        }
    }

    async fn forget(&self, key: &str) -> anyhow::Result<bool> {
        // Markdown memory is append-only by design (audit trail)
        // Return false to indicate the entry wasn't removed
        Ok(false)
    }

    async fn forget_for_agent(&self, key: &str, agent_id: &str) -> anyhow::Result<bool> {
        Ok(false)
    }

    async fn count(&self) -> anyhow::Result<usize> {
        let all = self.read_all_entries().await?;
        Ok(all.len())
    }

    async fn health_check(&self) -> bool {
        self.workspace_dir.exists()
    }

    async fn store_with_agent(
        &self,
        key: &str,
        content: &str,
        category: MemoryCategory,
        session_id: Option<&str>,
        _namespace: Option<&str>,
        _importance: Option<f64>,
        agent_id: Option<&str>,
    ) -> anyhow::Result<()> {
        self.store(key, content, category, session_id).await
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
        self.recall(query, limit, session_id, since, until).await
    }
}

impl Attributable for MarkdownMemory {
    fn role(&self) -> Role {
        Role::Memory(MemoryKind::Markdown)
    }

    fn alias(&self) -> &str {
        &self.alias
    }
}

fn entry_in_window(
    timestamp: &str,
    since: Option<&DateTime<FixedOffset>>,
    until: Option<&DateTime<FixedOffset>>,
) -> bool {
    if let Ok(ts) = DateTime::parse_from_rfc3339(timestamp) {
        if let Some(s) = since
            && ts < *s
        {
            return false;
        }

        if let Some(u) = until
            && ts > *u
        {
            return false;
        }
        return true;
    }

    if let Ok(date) = NaiveDate::parse_from_str(timestamp, "%Y-%m-%d") {
        if let Some(s) = since
            && date < s.date_naive()
        {
            return false;
        }

        if let Some(u) = until
            && date > u.date_naive()
        {
            return false;
        }
        return true;
    }
    true
}
