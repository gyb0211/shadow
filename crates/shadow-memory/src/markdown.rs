// //! Markdown 记忆后端 -- 每条记忆存为一个 .md 文件
// //!
// //! 文件格式:
// //! ```text
// //! ---
// //! id: <uuid>
// //! key: <key>
// //! category: <core|daily|conversation|custom>
// //! timestamp: <RFC 3339>
// //! session_id: <optionåal>
// //! ---
// //! <content>
// //! ```
//
// use shadow_core::{Attributable, Memory, MemoryCategory, MemoryEntry, Role};
// use anyhow::Result;
// use async_trait::async_trait;
// use chrono::Utc;
// use std::path::{Path, PathBuf};
//
// pub struct MarkdownMemory {
//     dir: PathBuf,
// }
//
// impl MarkdownMemory {
//     pub fn new(workspace_dir: &Path) -> Self {
//         let dir = workspace_dir.join("memory");
//         let _ = std::fs::create_dir_all(&dir);
//         Self { dir }
//     }
//
//     /// 安全文件名: 只保留字母数字和 -
//     fn entry_path(&self, key: &str) -> PathBuf {
//         let safe: String = key
//             .chars()
//             .map(|c| if c.is_alphanumeric() || c == '-' { c } else { '-' })
//             .collect();
//         self.dir.join(format!("{safe}.md"))
//     }
//
//     /// 解析 frontmatter + body
//     fn parse_file(content: &str, key: &str) -> MemoryEntry {
//         let mut id = String::new();
//         let mut category = MemoryCategory::Custom("general".to_string());
//         let mut timestamp = Utc::now().to_rfc3339();
//         let mut session_id = None;
//
//         // 解析 frontmatter (--- 之间的内容)
//         if content.starts_with("---") {
//             let parts: Vec<&str> = content.splitn(3, "---").collect();
//             if parts.len() >= 3 {
//                 let frontmatter = parts[1].trim();
//                 for line in frontmatter.lines() {
//                     let line = line.trim();
//                     if let Some((k, v)) = line.split_once(": ") {
//                         let v = v.trim();
//                         match k.trim() {
//                             "id" => id = v.to_string(),
//                             "category" => category = MemoryCategory::from_name(v),
//                             "timestamp" => timestamp = v.to_string(),
//                             "session_id" if !v.is_empty() => {
//                                 session_id = Some(v.to_string());
//                             }
//                             _ => {}
//                         }
//                     }
//                 }
//             }
//         }
//
//         // body: frontmatter 之后的内容
//         let body = if content.starts_with("---") {
//             content
//                 .splitn(3, "---")
//                 .nth(2)
//                 .unwrap_or("")
//                 .trim()
//                 .to_string()
//         } else {
//             content.trim().to_string()
//         };
//
//         MemoryEntry {
//             id: if id.is_empty() {
//                 uuid::Uuid::new_v4().to_string()
//             } else {
//                 id
//             },
//             agent_id: None,
//             key: key.to_string(),
//             content: body,
//             category,
//             timestamp,
//             session_id,
//             score: None,
//             agent_alias: None,
//             namespace: "".to_string(),
//             importance: None,
//             superseded_by: None,
//             kind: None,
//             pinned: false,
//             tenant_id: None,
//         }
//     }
// }
//
// impl Attributable for MarkdownMemory {
//     fn role(&self) -> Role {
//         Role::Memory
//     }
//     fn alias(&self) -> &str {
//         "markdown"
//     }
// }
//
// #[async_trait]
// impl Memory for MarkdownMemory {
//     fn name(&self) -> &str {
//         "markdown"
//     }
//
//     async fn store(
//         &self,
//         key: &str,
//         content: &str,
//         category: MemoryCategory,
//         session_id: Option<&str>,
//     ) -> Result<()> {
//         let path = self.entry_path(key);
//         let id = uuid::Uuid::new_v4().to_string();
//         let timestamp = Utc::now().to_rfc3339();
//
//         let mut file_content = format!(
//             "---\nid: {id}\nkey: {key}\ncategory: {}\ntimestamp: {timestamp}\n",
//             category.as_str()
//         );
//         if let Some(sid) = session_id {
//             file_content.push_str(&format!("session_id: {sid}\n"));
//         }
//         file_content.push_str(&format!("---\n{content}"));
//
//         tokio::fs::write(&path, file_content).await?;
//         Ok(())
//     }
//
//     async fn recall(
//         &self,
//         query: &str,
//         limit: usize,
//         session_id: Option<&str>,
//     ) -> Result<Vec<MemoryEntry>> {
//         let query_lower = query.to_lowercase();
//         let mut entries = self.list(None).await?;
//
//         // 关键词过滤
//         if !query_lower.is_empty() {
//             entries.retain(|e| {
//                 e.content.to_lowercase().contains(&query_lower)
//                     || e.key.to_lowercase().contains(&query_lower)
//             });
//         }
//
//         // session 过滤
//         if let Some(sid) = session_id {
//             entries.retain(|e| e.session_id.as_deref() == Some(sid));
//         }
//
//         entries.truncate(limit);
//         Ok(entries)
//     }
//
//     async fn get(&self, key: &str) -> Result<Option<MemoryEntry>> {
//         let path = self.entry_path(key);
//         if !path.exists() {
//             return Ok(None);
//         }
//         let content = tokio::fs::read_to_string(&path).await?;
//         Ok(Some(Self::parse_file(&content, key)))
//     }
//
//     async fn list(&self, category: Option<&MemoryCategory>) -> Result<Vec<MemoryEntry>> {
//         let mut entries = Vec::new();
//         if !self.dir.exists() {
//             return Ok(entries);
//         }
//         let mut rd = tokio::fs::read_dir(&self.dir).await?;
//         while let Some(entry) = rd.next_entry().await? {
//             let path = entry.path();
//             if path.extension().is_none_or(|ext| ext != "md") {
//                 continue;
//             }
//             let key = path
//                 .file_stem()
//                 .and_then(|s| s.to_str())
//                 .unwrap_or("unknown")
//                 .to_string();
//             let content = tokio::fs::read_to_string(&path).await?;
//             let mem_entry = Self::parse_file(&content, &key);
//
//             // 按 category 过滤
//             if let Some(cat) = category
//                 && &mem_entry.category != cat {
//                     continue;
//                 }
//
//             entries.push(mem_entry);
//         }
//         Ok(entries)
//     }
//
//     async fn forget(&self, key: &str) -> Result<bool> {
//         let path = self.entry_path(key);
//         if path.exists() {
//             tokio::fs::remove_file(&path).await?;
//             Ok(true)
//         } else {
//             Ok(false)
//         }
//     }
//
//     async fn count(&self) -> Result<usize> {
//         if !self.dir.exists() {
//             return Ok(0);
//         }
//         let mut count = 0;
//         let mut rd = tokio::fs::read_dir(&self.dir).await?;
//         while let Some(entry) = rd.next_entry().await? {
//             if entry.path().extension().is_none_or(|ext| ext == "md") {
//                 count += 1;
//             }
//         }
//         Ok(count)
//     }
//
//     fn health_check(&self) -> bool {
//         self.dir.exists()
//     }
// }
//
