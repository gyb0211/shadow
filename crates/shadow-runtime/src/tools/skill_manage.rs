//! 技能管理工具 -- LLM 可调用, 列出/查看/改进技能

use shadow_core::{Attributable, Tool, ToolResult, Role};
use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::skills::improver::SkillImprover;
use crate::skills::SkillsService;

// ── SkillListTool: 列出已安装技能 ──────────────────────────

/// 列出所有已安装的技能
pub struct SkillListTool {
    skills_dir: PathBuf,
}

impl SkillListTool {
    pub fn new(skills_dir: PathBuf) -> Self {
        Self { skills_dir }
    }
}

impl Attributable for SkillListTool {
    fn role(&self) -> Role { Role::Tool }
    fn alias(&self) -> &str { "skill_list" }
}

#[async_trait]
impl Tool for SkillListTool {
    fn name(&self) -> &str { "skill_list" }
    fn description(&self) -> &str { "列出所有已安装的技能及其工具" }
    fn parameters_schema(&self) -> Value { json!({"type": "object", "properties": {}}) }

    async fn execute(&self, _args: Value) -> Result<ToolResult> {
        let service = SkillsService::load(&self.skills_dir)
            .map_err(|e| anyhow::anyhow!("加载技能失败: {e}"))?;
        let skills = service.list();
        if skills.is_empty() {
            return Ok(ToolResult::ok("没有已安装的技能"));
        }
        let mut lines = Vec::new();
        for skill in skills {
            let tool_names: Vec<_> = skill.tools.iter().map(|t| t.name.as_str()).collect();
            lines.push(format!(
                "- {} ({}): {} [工具: {}]",
                skill.name,
                skill.description,
                skill.prompts.len(),
                tool_names.join(", ")
            ));
        }
        Ok(ToolResult::ok(lines.join("\n")))
    }
}

// ── SkillViewTool: 查看技能详情 ────────────────────────────

/// 查看指定技能的 SKILL.md 内容
pub struct SkillViewTool {
    skills_dir: PathBuf,
}

impl SkillViewTool {
    pub fn new(skills_dir: PathBuf) -> Self {
        Self { skills_dir }
    }
}

impl Attributable for SkillViewTool {
    fn role(&self) -> Role { Role::Tool }
    fn alias(&self) -> &str { "skill_view" }
}

#[async_trait]
impl Tool for SkillViewTool {
    fn name(&self) -> &str { "skill_view" }
    fn description(&self) -> &str { "查看指定技能的详情, 包括 SKILL.md 内容" }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "技能名称" }
            },
            "required": ["name"]
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let name = args.get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("缺少 name 参数"))?;
        let path = self.skills_dir.join(name).join("SKILL.md");
        if !path.exists() {
            return Ok(ToolResult::err(format!("技能 '{name}' 不存在")));
        }
        let content = tokio::fs::read_to_string(&path).await?;
        Ok(ToolResult::ok(content))
    }
}

// ── SkillManageTool: 改进/创建技能 ─────────────────────────

/// 改进或创建技能 (通过 SkillImprover 原子写入)
pub struct SkillManageTool {
    improver: Arc<Mutex<SkillImprover>>,
}

impl SkillManageTool {
    pub fn new(improver: Arc<Mutex<SkillImprover>>) -> Self {
        Self { improver }
    }
}

impl Attributable for SkillManageTool {
    fn role(&self) -> Role { Role::Tool }
    fn alias(&self) -> &str { "skill_manage" }
}

#[async_trait]
impl Tool for SkillManageTool {
    fn name(&self) -> &str { "skill_manage" }
    fn description(&self) -> &str { "改进或创建技能。action: patch=改进已有技能, create=创建新技能" }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["patch", "create"], "description": "操作类型" },
                "name": { "type": "string", "description": "技能名称" },
                "content": { "type": "string", "description": "完整的 SKILL.md 内容" },
                "reason": { "type": "string", "description": "改进原因" }
            },
            "required": ["action", "name", "content", "reason"]
        })
    }

    fn requires_approval(&self) -> bool { true }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("patch");
        let name = args.get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("缺少 name 参数"))?;
        let content = args.get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("缺少 content 参数"))?;
        let reason = args.get("reason")
            .and_then(|v| v.as_str())
            .unwrap_or("LLM 改进");

        let mut improver = self.improver.lock().await;
        match improver.improve_skill(name, content, reason).await {
            Ok(()) => Ok(ToolResult::ok(format!("技能 '{name}' 已{action}成功"))),
            Err(e) => Ok(ToolResult::err(format!("技能{action}失败: {e}"))),
        }
    }
}
