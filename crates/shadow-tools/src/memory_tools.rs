//! 记忆存储/检索工具 -- agent 长期记忆
//!
//! memory_store: 存储记忆
//! memory_recall: 检索记忆

use anyhow::Result;
use async_trait::async_trait;
use serde_json::json;
use shadow_core::{Tool, ToolResult, Memory, MemoryCategory, MemoryEntry};
use std::sync::Arc;

// ── 记忆存储工具 ──────────────────────────────────────────────

/// 记忆存储工具
pub struct MemoryStoreTool {
    memory: Arc<dyn Memory>,
}

impl MemoryStoreTool {
    pub fn new(memory: Arc<dyn Memory>) -> Self {
        Self { memory }
    }
}

shadow_core::tool_attribution!(MemoryStoreTool, shadow_core::ToolKind::Memory);

#[async_trait]
impl Tool for MemoryStoreTool {
    fn name(&self) -> &str {
        "memory_store"
    }

    fn description(&self) -> &str {
        "Store a fact, preference, or note in long-term memory. \
         Use category 'core' for permanent facts, 'daily' for session notes, \
         'conversation' for chat context, or a custom category name."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "key": {
                    "type": "string",
                    "description": "Unique key for this memory (e.g. 'user_lang', 'project_stack')"
                },
                "content": {
                    "type": "string",
                    "description": "The information to remember"
                },
                "category": {
                    "type": "string",
                    "description": "Memory category: 'core' (permanent), 'daily' (session), 'conversation' (chat), or custom. Default: 'core'.",
                    "default": "core"
                }
            },
            "required": ["key", "content"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> Result<ToolResult> {
        let key = match args.get("key").and_then(|v| v.as_str()) {
            Some(k) if !k.is_empty() => k,
            _ => return Ok(ToolResult::err("Missing required parameter: key")),
        };

        let content = match args.get("content").and_then(|v| v.as_str()) {
            Some(c) if !c.is_empty() => c,
            _ => return Ok(ToolResult::err("Missing required parameter: content")),
        };

        let category = args.get("category").and_then(|v| v.as_str()).unwrap_or("core");
        let cat = MemoryCategory::from_name(category);

        match self.memory.store(key, content, cat, None).await {
            Ok(()) => Ok(ToolResult::ok(format!("Stored memory: {key}"))),
            Err(e) => Ok(ToolResult::err(format!("Failed to store: {e}"))),
        }
    }
}

// ── 记忆检索工具 ──────────────────────────────────────────────

/// 记忆检索工具
pub struct MemoryRecallTool {
    memory: Arc<dyn Memory>,
}

impl MemoryRecallTool {
    pub fn new(memory: Arc<dyn Memory>) -> Self {
        Self { memory }
    }
}

shadow_core::tool_attribution!(MemoryRecallTool, shadow_core::ToolKind::Memory);

#[async_trait]
impl Tool for MemoryRecallTool {
    fn name(&self) -> &str {
        "memory_recall"
    }

    fn description(&self) -> &str {
        "Search long-term memory for relevant facts, preferences, or context. \
         Returns scored results ranked by relevance."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Keywords to search for in memory. Omit to return recent memories."
                },
                "limit": {
                    "type": "integer",
                    "description": "Max results to return. Default: 5",
                    "default": 5
                }
            }
        })
    }

    async fn execute(&self, args: serde_json::Value) -> Result<ToolResult> {
        let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .unwrap_or(5);

        match self.memory.recall(query, limit, None, None, None).await {
            Ok(results) => {
                if results.is_empty() {
                    Ok(ToolResult::ok("No memories found"))
                } else {
                    let output: Vec<serde_json::Value> = results
                        .iter()
                        .map(|r| {
                            json!({
                                "key": r.key,
                                "content": r.content,
                                "timestamp": r.timestamp,
                                "score": r.score,
                            })
                        })
                        .collect();
                    Ok(ToolResult::ok(serde_json::to_string_pretty(&output).unwrap_or_default()))
                }
            }
            Err(e) => Ok(ToolResult::err(format!("Failed to recall: {e}"))),
        }
    }
}