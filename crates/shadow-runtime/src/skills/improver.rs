//! Skill 自进化 -- 原子写入 + 冷却 + 审计跟踪
//!
//! 参考 ZeroClaw skills/improver.rs + Hermes _spawn_background_review
//! 精简版: 内存+磁盘双冷却, 临时文件→验证→rename 原子写入, HTML 注释审计跟踪

use anyhow::{Context, Result, bail};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;

const FRONT_MATTER_DELIM: &str = "---";
const DEFAULT_COOLDOWN_SECS: u64 = 300; // 5 分钟

/// Skill 自进化管理器 -- 原子写入 + 双冷却 + 审计跟踪
pub struct SkillImprover {
    workspace_dir: PathBuf,
    cooldown_secs: u64,
    /// 内存冷却 (进程内, 快速检查)
    cooldowns: HashMap<String, Instant>,
}

impl SkillImprover {
    pub fn new(workspace_dir: PathBuf) -> Self {
        Self {
            workspace_dir,
            cooldown_secs: DEFAULT_COOLDOWN_SECS,
            cooldowns: HashMap::new(),
        }
    }

    /// 设置冷却时间 (秒)
    pub fn with_cooldown(mut self, secs: u64) -> Self {
        self.cooldown_secs = secs;
        self
    }

    /// 检查技能是否可以改进 (冷却是否过期)
    /// 内存冷却 + 磁盘冷却 (SKILL.md front-matter 的 updated_at)
    pub fn should_improve(&self, slug: &str) -> bool {
        // 1. 内存冷却
        if let Some(last) = self.cooldowns.get(slug) {
            let elapsed = Instant::now().saturating_duration_since(*last);
            if elapsed.as_secs() < self.cooldown_secs {
                return false;
            }
        }
        // 2. 磁盘冷却 (读 SKILL.md 的 updated_at)
        if self.is_on_disk_cooldown(slug) {
            return false;
        }
        true
    }

    /// 改进技能 -- 原子写入: 临时文件 → 验证 → rename
    pub async fn improve_skill(
        &mut self,
        slug: &str,
        improved_content: &str,
        reason: &str,
    ) -> Result<()> {
        // 1. 验证内容
        validate_skill_content(improved_content)?;

        // 2. 读取现有文件, 提取审计跟踪
        let skill_dir = self.skills_dir().join(slug);
        let md_path = skill_dir.join("SKILL.md");
        let existing = tokio::fs::read_to_string(&md_path)
            .await
            .with_context(|| format!("读取技能文件失败: {}", md_path.display()))?;
        let audit_trail = extract_audit_trail(&existing);

        // 3. 更新 front-matter (加 updated_at + improvement_reason)
        let now = chrono::Utc::now().to_rfc3339();
        let updated = append_improvement_metadata(improved_content, &now, reason);

        // 4. 拼接: 更新内容 + 旧审计跟踪 + 新审计条目
        let single_line_reason = reason.replace('\n', " ");
        let audit_entry = format!("\n<!-- Improvement: {now} | Reason: {single_line_reason} -->\n");
        let final_content = if audit_trail.is_empty() {
            format!("{updated}{audit_entry}")
        } else {
            format!("{updated}\n{audit_trail}{audit_entry}")
        };

        // 5. 原子写入: 临时文件 → 验证 → rename
        let temp_path = skill_dir.join(".SKILL.md.tmp");
        tokio::fs::write(&temp_path, &final_content)
            .await
            .context("写入临时文件失败")?;

        // 验证临时文件
        let written = tokio::fs::read_to_string(&temp_path).await?;
        validate_skill_content(&written)?;

        // rename (原子替换)
        tokio::fs::rename(&temp_path, &md_path)
            .await
            .with_context(|| format!("重命名失败: {} → {}", temp_path.display(), md_path.display()))?;

        // 6. 更新内存冷却
        self.cooldowns.insert(slug.to_string(), Instant::now());

        Ok(())
    }

    fn skills_dir(&self) -> PathBuf {
        self.workspace_dir.join("skills")
    }

    /// 磁盘冷却: 读 SKILL.md front-matter 的 updated_at 字段
    fn is_on_disk_cooldown(&self, slug: &str) -> bool {
        let md_path = self.skills_dir().join(slug).join("SKILL.md");
        let Ok(content) = std::fs::read_to_string(&md_path) else {
            return false;
        };
        let Some((front, _)) = split_front_matter(&content) else {
            return false;
        };
        let Some(value) = front_matter_value(&front, "updated_at") else {
            return false;
        };
        let Ok(ts) = chrono::DateTime::parse_from_rfc3339(value.trim()) else {
            return false;
        };
        let elapsed = chrono::Utc::now().signed_duration_since(ts);
        elapsed.num_seconds() < self.cooldown_secs as i64
    }
}

// ── 辅助函数 ──────────────────────────────────────────────

/// 验证技能内容: 必须有 front-matter + name 字段
pub fn validate_skill_content(content: &str) -> Result<()> {
    if content.trim().is_empty() {
        bail!("技能内容为空");
    }
    let Some((front, _)) = split_front_matter(content) else {
        bail!("技能内容缺少 YAML front-matter (需要 --- 分隔块)");
    };
    let name = front_matter_value(&front, "name").unwrap_or_default();
    if name.trim().is_empty() {
        bail!("技能 front-matter 缺少必填字段 name");
    }
    Ok(())
}

/// 分离 front-matter 和 body
/// 返回 (front_matter_text, body_text)
fn split_front_matter(content: &str) -> Option<(String, String)> {
    let normalized = content.replace("\r\n", "\n");
    let rest = normalized.strip_prefix("---\n")?;
    if let Some(idx) = rest.find("\n---\n") {
        Some((rest[..idx].to_string(), rest[idx + 5..].to_string()))
    } else {
        rest.strip_suffix("\n---")
            .map(|front| (front.to_string(), String::new()))
    }
}

/// 查找 front-matter 中的 key 对应的 value (扁平 key: value, 不处理嵌套)
fn front_matter_value(front: &str, key: &str) -> Option<String> {
    for line in front.lines() {
        if line.starts_with(' ') || line.starts_with('\t') {
            continue; // 跳过嵌套行
        }
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        if k.trim() == key {
            let v = v.trim();
            let unquoted = v.trim_matches('"').trim_matches('\'');
            return Some(unquoted.to_string());
        }
    }
    None
}

/// 更新 front-matter: 添加/替换 updated_at + improvement_reason
fn append_improvement_metadata(content: &str, timestamp: &str, reason: &str) -> String {
    let normalized = content.replace("\r\n", "\n");
    let Some((front, body)) = split_front_matter(&normalized) else {
        // 无 front-matter, 生成一个
        let yaml = format!(
            "name: \"unknown\"\nupdated_at: \"{timestamp}\"\nimprovement_reason: \"{}\"\n",
            yaml_escape(reason)
        );
        return format!("{FRONT_MATTER_DELIM}\n{yaml}{FRONT_MATTER_DELIM}\n{normalized}");
    };

    // 移除已有的 updated_at / improvement_reason
    let stripped: Vec<&str> = front
        .lines()
        .filter(|line| {
            if line.starts_with(' ') || line.starts_with('\t') {
                return true; // 保留嵌套行
            }
            let trimmed = line.trim_start();
            !trimmed.starts_with("updated_at:") && !trimmed.starts_with("improvement_reason:")
        })
        .collect();

    let mut new_front = stripped.join("\n");
    if !new_front.ends_with('\n') {
        new_front.push('\n');
    }
    new_front.push_str(&format!(
        "updated_at: \"{timestamp}\"\nimprovement_reason: \"{}\"\n",
        yaml_escape(reason)
    ));

    format!("{FRONT_MATTER_DELIM}\n{new_front}{FRONT_MATTER_DELIM}\n{body}")
}

/// 提取审计跟踪 (<!-- Improvement: ... --> 行)
fn extract_audit_trail(content: &str) -> String {
    content
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            trimmed.starts_with("<!-- Improvement:") && trimmed.ends_with("-->")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// YAML 转义 (双引号字符串)
fn yaml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push(' '),
            '\r' => {}
            _ => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn make_skill_file(dir: &std::path::Path, slug: &str) {
        let skill_dir = dir.join("skills").join(slug);
        std::fs::create_dir_all(&skill_dir).unwrap();
        let content = "---\nname: test-skill\ndescription: 测试技能\n---\n# Test\n这是测试内容\n";
        std::fs::write(skill_dir.join("SKILL.md"), content).unwrap();
    }

    #[test]
    fn validate_valid_content() {
        let content = "---\nname: test\ndescription: 测试\n---\n# Body\n";
        assert!(validate_skill_content(content).is_ok());
    }

    #[test]
    fn validate_empty_content() {
        assert!(validate_skill_content("").is_err());
    }

    #[test]
    fn validate_no_frontmatter() {
        assert!(validate_skill_content("# Hello\n").is_err());
    }

    #[test]
    fn validate_no_name() {
        let content = "---\ndescription: 测试\n---\n# Body\n";
        assert!(validate_skill_content(content).is_err());
    }

    #[tokio::test]
    async fn improve_skill_adds_audit_trail() {
        let dir = tempdir().unwrap();
        make_skill_file(dir.path(), "test-skill");
        let mut improver = SkillImprover::new(dir.path().to_path_buf());

        let improved = "---\nname: test-skill\ndescription: 改进后的测试\n---\n# Test\n改进后的内容\n";
        improver
            .improve_skill("test-skill", improved, "添加了新步骤")
            .await
            .unwrap();

        let content = std::fs::read_to_string(
            dir.path().join("skills/test-skill/SKILL.md"),
        )
        .unwrap();
        assert!(content.contains("updated_at:"));
        assert!(content.contains("improvement_reason:"));
        assert!(content.contains("<!-- Improvement:"));
        assert!(content.contains("添加了新步骤"));
        assert!(content.contains("改进后的内容"));
    }

    #[tokio::test]
    async fn improve_skill_accumulates_audit_trail() {
        let dir = tempdir().unwrap();
        make_skill_file(dir.path(), "test-skill");
        let mut improver = SkillImprover::new(dir.path().to_path_buf()).with_cooldown(0);

        let v1 = "---\nname: test-skill\ndescription: v1\n---\n# v1\n";
        improver.improve_skill("test-skill", v1, "第一次改进").await.unwrap();

        let v2 = "---\nname: test-skill\ndescription: v2\n---\n# v2\n";
        improver.improve_skill("test-skill", v2, "第二次改进").await.unwrap();

        let content = std::fs::read_to_string(
            dir.path().join("skills/test-skill/SKILL.md"),
        )
        .unwrap();
        assert_eq!(content.matches("<!-- Improvement:").count(), 2);
        assert!(content.contains("第一次改进"));
        assert!(content.contains("第二次改进"));
    }

    #[test]
    fn should_improve_no_cooldown() {
        let dir = tempdir().unwrap();
        make_skill_file(dir.path(), "test-skill");
        let improver = SkillImprover::new(dir.path().to_path_buf());
        assert!(improver.should_improve("test-skill"));
    }

    #[test]
    fn should_improve_after_cooldown() {
        let dir = tempdir().unwrap();
        make_skill_file(dir.path(), "test-skill");
        let improver = SkillImprover::new(dir.path().to_path_buf()).with_cooldown(0);
        // cooldown=0, 应该总是可以改进
        assert!(improver.should_improve("test-skill"));
    }

    #[test]
    fn extract_audit_trail_from_empty() {
        let content = "---\nname: test\n---\n# Body\n";
        assert!(extract_audit_trail(content).is_empty());
    }

    #[test]
    fn extract_audit_trail_multiple() {
        let content = "---\nname: test\n---\n# Body\n<!-- Improvement: 2026-01-01 | Reason: a -->\n<!-- Improvement: 2026-01-02 | Reason: b -->\n";
        let trail = extract_audit_trail(content);
        assert!(trail.contains("Reason: a"));
        assert!(trail.contains("Reason: b"));
    }
}
