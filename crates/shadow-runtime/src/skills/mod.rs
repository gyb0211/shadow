//! Skills 系统 -- SKILL.md 解析 + 目录加载 + 技能工具注册
//!
//! 借鉴 ZeroClaw 的 Skills 设计但大幅精简:
//! - ZeroClaw: 四源合并 + 审计 + 缓存 + 自改进 + SkillForge (10973行)
//! - Shadow: SKILL.md 解析 + 目录加载 + 技能工具注册 (~350行)
//!
//! 技能目录结构:
//! ```text
//! ~/.shadow/skills/
//! ├── git-helper/
//! │   └── SKILL.md
//! └── docker-helper/
//!     └── SKILL.md
//! ```
//!
//! SKILL.md 格式:
//! ```markdown
//! ---
//! name: git-helper
//! description: Git 操作辅助技能
//! tools:
//!   - name: status
//!     description: 查看 git 状态
//!     kind: shell
//!     command: git status
//!     args:
//!       - path
//! prompts:
//!   - "你是一个 git 专家"
//! ---
//!
//! # Git Helper
//!
//! 这是技能的说明文档, 会作为附加提示使用...
//! ```


// ── 数据结构 ──────────────────────────────────────────────────────────

use std::path::Path;
use serde::{Deserialize, Serialize};
use anyhow::{Context, Result};
use shadow_core::Tool;

/// 技能工具定义 -- 从 SKILL.md frontmatter 解析
///
/// 每个工具代表一个可被 agent 调用的操作
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillTool {
    /// 工具名称 (在技能内唯一)
    pub name: String,
    /// 工具描述 (给 LLM 看)
    #[serde(default)]
    pub description: String,
    /// 工具类型: "shell" | "http" | "builtin"
    #[serde(default)]
    pub kind: String,
    /// 命令模板 (shell 类型为 shell 命令, 可含 {arg_name} 占位符)
    #[serde(default)]
    pub command: String,
    /// 参数名列表 (模型可提供这些参数的值)
    #[serde(default)]
    pub args: Vec<String>,
}

/// 技能 -- 一个 SKILL.md 文件对应一个 Skill
#[derive(Debug, Clone, Serialize)]
pub struct Skill {
    /// 技能名称
    pub name: String,
    /// 技能描述
    pub description: String,
    /// 技能包含的工具列表
    pub tools: Vec<SkillTool>,
    /// 提示文本列表 (frontmatter 中的 prompts + body 正文)
    pub prompts: Vec<String>,
}

/// Skills 服务 -- 加载、列表、查找技能
///
/// 负责从文件系统加载技能, 并将技能工具注册为 Tool trait 对象
pub struct SkillsService {
    skills: Vec<Skill>,
}

impl SkillsService {
    /// 从工作目录加载技能
    ///
    /// 扫描 ~/.shadow/skills/ 和 {workspace_dir}/.shadow/skills/
    pub fn load(workspace_dir: &Path) -> Result<Self> {
        let skills = load_skills(workspace_dir)?;
        Ok(Self { skills })
    }

    /// 从已有技能列表创建 (用于测试或自定义加载)
    pub fn from_skills(skills: Vec<Skill>) -> Self {
        Self { skills }
    }

    /// 获取所有技能
    pub fn list(&self) -> &[Skill] {
        &self.skills
    }

    /// 按名称查找技能
    pub fn find(&self, name: &str) -> Option<&Skill> {
        self.skills.iter().find(|s| s.name == name)
    }

    /// 将所有 shell 类型的技能工具注册为 Tool trait 对象
    ///
    /// 返回的 Vec 可直接传给 AgentBuilder::tools()
    pub fn all_tools(&self) -> Vec<Box<dyn Tool>> {
        let mut tools = Vec::new();
        for skill in &self.skills {
            for tool_def in &skill.tools {
                match tool_def.kind.as_str() {
                    // "shell" => {
                    //     tools.push(Box::new(SkillShellTool::new(&skill.name, tool_def.clone()))
                    //         as Box<dyn Tool>);
                    // }
                    // "http" => {
                    //     tools.push(Box::new(SkillHttpTool::new(&skill.name, tool_def.clone()))
                    //         as Box<dyn Tool>);
                    // }
                    // "builtin" => {
                    //     // TODO: 后续实现内置类型工具
                    //     tracing::debug!("跳过未实现的技能工具类型: builtin ({})", tool_def.name);
                    // }
                    _ => {
                        tracing::warn!("未知技能工具类型: {}", tool_def.kind);
                    }
                }
            }
        }
        tools
    }
}

// ── SKILL.md 解析 ────────────────────────────────────────────────────

/// frontmatter 反序列化用的中间结构
///
/// 所有字段都带 #[serde(default)] -- 空值在 to_json() 中被跳过,
/// serde 会用默认值 (空字符串/空 Vec) 填充
#[derive(Debug, Deserialize)]
struct SkillFrontmatter {
    #[serde(default)]
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    tools: Vec<SkillTool>,
    #[serde(default)]
    prompts: Vec<String>,
}

/// 解析 SKILL.md 内容
///
/// 将 SKILL.md 分为 frontmatter (YAML) 和 body (Markdown 正文):
/// - frontmatter 解析为技能元数据 (name, description, tools, prompts)
/// - body 作为附加提示文本追加到 prompts
///
/// # 错误
/// - 缺少 frontmatter (不以 --- 开头)
/// - frontmatter 未闭合 (缺少结束 ---)
/// - YAML 解析失败
/// - name 字段为空
pub fn parse_skill_md(content: &str) -> Result<Skill> {
    // 分离 frontmatter 和 body
    let (frontmatter, body) = split_frontmatter(content)?;

    // 解析 frontmatter YAML
    let yaml_value = parse_yaml(&frontmatter)?;
    let fm: SkillFrontmatter =
        serde_json::from_value(yaml_value).context("解析 frontmatter 失败")?;

    if fm.name.is_empty() {
        return Err(anyhow::anyhow!("技能名称 (name) 不能为空"));
    }

    // 收集 prompts -- frontmatter 中的 + body 正文
    let mut prompts = fm.prompts;
    if !body.is_empty() {
        prompts.push(body);
    }

    Ok(Skill {
        name: fm.name,
        description: fm.description,
        tools: fm.tools,
        prompts,
    })
}

/// 分离 frontmatter 和 body
///
/// SKILL.md 格式:
/// ```text
/// ---
/// name: ...
/// ---
/// body text...
/// ```
///
/// 返回 (frontmatter_yaml, body_text)
fn split_frontmatter(content: &str) -> Result<(String, String)> {
    let content = content.trim();
    if !content.starts_with("---") {
        return Err(anyhow::anyhow!("缺少 frontmatter: SKILL.md 应以 --- 开头"));
    }

    let lines: Vec<&str> = content.lines().collect();

    // 第一行是 ---, 查找结束的 ---
    let mut end_line = None;
    for (i, line) in lines.iter().enumerate().skip(1) {
        if line.trim() == "---" {
            end_line = Some(i);
            break;
        }
    }

    match end_line {
        Some(i) => {
            let frontmatter = lines[1..i].join("\n");
            let body = lines
                .get(i + 1..)
                .map(|s| s.join("\n").trim().to_string())
                .unwrap_or_default();
            Ok((frontmatter, body))
        }
        None => Err(anyhow::anyhow!("frontmatter 未闭合: 缺少结束标记 ---")),
    }
}

// ── 最小 YAML 解析器 ─────────────────────────────────────────────────
//
// 仅支持 frontmatter 所需的 YAML 子集:
// - 映射: key: value
// - 序列: - item
// - 映射序列: - key: value (序列中的对象)
// - 嵌套结构 (通过缩进)
// - 字符串值 (可带引号)
//
// 不支持: 多行字符串、锚点、别名、流式语法等高级特性

/// YAML 行 -- 预处理后的行, 包含缩进级别和内容
struct YamlLine {
    indent: usize,
    content: String,
}

/// YAML 值 -- 内部表示
#[derive(Debug, Clone)]
enum YamlValue {
    /// 标量值 (字符串)
    Scalar(String),
    /// 序列 (列表)
    Seq(Vec<YamlValue>),
    /// 映射 (键值对集合)
    Map(Vec<(String, YamlValue)>),
}

impl YamlValue {
    /// 转换为 serde_json::Value, 便于用 serde 反序列化
    fn to_json(&self) -> serde_json::Value {
        match self {
            YamlValue::Scalar(s) => {
                let s = unquote(s.trim());
                serde_json::Value::String(s)
            }
            YamlValue::Seq(items) => {
                serde_json::Value::Array(items.iter().map(|v| v.to_json()).collect())
            }
            YamlValue::Map(pairs) => {
                let mut map = serde_json::Map::new();
                for (k, v) in pairs {
                    match v {
                        // 空标量 -- 跳过, 让 serde 的 #[serde(default)] 生效
                        YamlValue::Scalar(s) if s.trim().is_empty() => {}
                        _ => {
                            map.insert(k.clone(), v.to_json());
                        }
                    }
                }
                serde_json::Value::Object(map)
            }
        }
    }
}

/// 去除字符串两端的引号 (单引号或双引号)
fn unquote(s: &str) -> String {
    if s.len() >= 2 {
        let bytes = s.as_bytes();
        if (bytes[0] == b'"' && bytes[s.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[s.len() - 1] == b'\'')
        {
            return s[1..s.len() - 1].to_string();
        }
    }
    s.to_string()
}

/// 查找键值分隔符冒号的位置
///
/// YAML 中键值分隔符是 `: ` (冒号后跟空格) 或 `:` 在行尾
/// 例如 `name: status` 返回 4
/// 但 `http://example.com` 中的冒号不是分隔符 (后面不是空格)
fn find_colon(s: &str) -> Option<usize> {
    for (i, c) in s.char_indices() {
        if c == ':' {
            let rest = &s[i + 1..];
            if rest.is_empty() || rest.starts_with(' ') {
                return Some(i);
            }
        }
    }
    None
}

/// 预处理 YAML 文本 -- 按行分割, 计算缩进, 过滤空行和注释
fn preprocess_yaml(content: &str) -> Vec<YamlLine> {
    content
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            // 跳过空行和注释
            if trimmed.is_empty() || trimmed.starts_with('#') {
                return None;
            }
            let indent = line.len() - line.trim_start().len();
            Some(YamlLine {
                indent,
                content: trimmed.to_string(),
            })
        })
        .collect()
}

/// 解析 YAML 块 -- 递归下降解析器
///
/// 从当前位置开始, 解析缩进大于 parent_indent 的连续行
fn parse_block(lines: &[YamlLine], pos: &mut usize, parent_indent: usize) -> YamlValue {
    if *pos >= lines.len() {
        return YamlValue::Scalar(String::new());
    }

    let block_indent = lines[*pos].indent;
    if block_indent <= parent_indent {
        return YamlValue::Scalar(String::new());
    }

    // 判断是映射还是序列
    if lines[*pos].content.starts_with('-') {
        parse_sequence(lines, pos, block_indent)
    } else {
        parse_mapping(lines, pos, block_indent)
    }
}

/// 解析映射 -- 连续的 key: value 行 (同一缩进级别)
fn parse_mapping(lines: &[YamlLine], pos: &mut usize, indent: usize) -> YamlValue {
    let mut pairs = Vec::new();

    while *pos < lines.len() {
        let line = &lines[*pos];

        // 缩进不匹配或遇到序列项 -> 结束映射
        if line.indent != indent || line.content.starts_with('-') {
            break;
        }

        // 查找键值分隔符
        let colon_pos = match find_colon(&line.content) {
            Some(p) => p,
            None => break, // 不是有效的键值对
        };

        let key = line.content[..colon_pos].trim().to_string();
        let value_str = line.content[colon_pos + 1..].trim();
        *pos += 1;

        if value_str.is_empty() {
            // 值为空 -- 解析嵌套块 (缩进更深的后续行)
            let child = parse_block(lines, pos, indent);
            pairs.push((key, child));
        } else {
            pairs.push((key, YamlValue::Scalar(value_str.to_string())));
        }
    }

    YamlValue::Map(pairs)
}

/// 解析序列 -- 连续的 `- item` 行 (同一缩进级别)
fn parse_sequence(lines: &[YamlLine], pos: &mut usize, indent: usize) -> YamlValue {
    let mut items = Vec::new();

    while *pos < lines.len() {
        let line = &lines[*pos];

        if line.indent != indent || !line.content.starts_with('-') {
            break;
        }

        // 获取 `-` 之后的内容
        let after_dash = line.content[1..].trim_start();

        if after_dash.is_empty() {
            // 只有 `-`, 嵌套块在下一行
            *pos += 1;
            let child = parse_block(lines, pos, indent);
            items.push(child);
        } else if let Some(colon_pos) = find_colon(after_dash) {
            // `- key: value` -- 映射序列项 (序列中的对象)
            let key = after_dash[..colon_pos].trim().to_string();
            let value_str = after_dash[colon_pos + 1..].trim();

            // 计算映射键的起始列 (用于后续行的对齐)
            // key 在原始行中的列 = indent + (line.content 中 key 的位置)
            // line.content 中 key 的位置 = line.content.len() - after_dash.len()
            let key_col = indent + line.content.len() - after_dash.len();

            *pos += 1;

            let mut map_pairs = Vec::new();
            if value_str.is_empty() {
                let child = parse_block(lines, pos, key_col);
                map_pairs.push((key, child));
            } else {
                map_pairs.push((key, YamlValue::Scalar(value_str.to_string())));
            }

            // 继续解析同一映射对象的后续键值对 (同一 key_col 缩进)
            while *pos < lines.len() {
                let next_line = &lines[*pos];
                if next_line.indent != key_col || next_line.content.starts_with('-') {
                    break;
                }
                let next_colon = match find_colon(&next_line.content) {
                    Some(p) => p,
                    None => break,
                };
                let next_key = next_line.content[..next_colon].trim().to_string();
                let next_value = next_line.content[next_colon + 1..].trim();
                *pos += 1;
                if next_value.is_empty() {
                    let child = parse_block(lines, pos, key_col);
                    map_pairs.push((next_key, child));
                } else {
                    map_pairs.push((next_key, YamlValue::Scalar(next_value.to_string())));
                }
            }

            items.push(YamlValue::Map(map_pairs));
        } else {
            // `- value` -- 标量序列项
            items.push(YamlValue::Scalar(after_dash.to_string()));
            *pos += 1;
        }
    }

    YamlValue::Seq(items)
}

/// 解析 YAML 文本为 serde_json::Value
fn parse_yaml(content: &str) -> Result<serde_json::Value> {
    let lines = preprocess_yaml(content);
    if lines.is_empty() {
        return Ok(serde_json::Value::Null);
    }
    let mut pos = 0;
    // 顶层解析: 直接调用 parse_mapping 或 parse_sequence, 不经过 parse_block
    // (parse_block 的 parent_indent 检查会拒绝缩进 == parent 的行)
    let block_indent = lines[0].indent;
    let value = if lines[0].content.starts_with('-') {
        parse_sequence(&lines, &mut pos, block_indent)
    } else {
        parse_mapping(&lines, &mut pos, block_indent)
    };
    Ok(value.to_json())
}

// ── 目录加载 ─────────────────────────────────────────────────────────

/// 从指定目录加载技能
///
/// 扫描 {skills_dir}/*/SKILL.md, 解析每个文件为 Skill
///
/// 如果目录不存在, 返回空列表
/// 单个技能解析失败会记录警告但不会中断其他技能的加载
pub fn load_skills_from_dir(skills_dir: &Path) -> Result<Vec<Skill>> {
    if !skills_dir.exists() {
        return Ok(vec![]);
    }

    let mut skills = Vec::new();

    for entry in std::fs::read_dir(skills_dir)
        .with_context(|| format!("读取技能目录失败: {}", skills_dir.display()))?
    {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!("读取目录条目失败: {e}");
                continue;
            }
        };

        // 只处理子目录
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }

        let skill_md = entry.path().join("SKILL.md");
        if !skill_md.exists() {
            continue;
        }

        let content = std::fs::read_to_string(&skill_md)
            .with_context(|| format!("读取技能文件失败: {}", skill_md.display()))?;

        match parse_skill_md(&content) {
            Ok(skill) => skills.push(skill),
            Err(e) => {
                // 记录错误但继续加载其他技能
                tracing::warn!("解析技能失败 {}: {e}", skill_md.display());
            }
        }
    }

    Ok(skills)
}

/// 加载技能 -- 扫描 ~/.shadow/skills/*/SKILL.md
///
/// 同时也扫描 {workspace_dir}/.shadow/skills/ 作为备选位置
///
/// # 参数
/// - `workspace_dir`: 工作目录, 用于查找 {workspace_dir}/.shadow/skills/
pub fn load_skills(workspace_dir: &Path) -> Result<Vec<Skill>> {
    let mut skills = Vec::new();

    // // 1. 扫描 ~/.shadow/skills/ (经 shadow_config::config_dir() 解析, 支持 SHADOW_CONFIG_DIR override)
    // let home_dir = shadow_config::config_dir().join("skills");
    // if home_dir.exists() {
    //     skills.extend(load_skills_from_dir(&home_dir)?);
    // }
    //
    // // 2. 扫描 {workspace_dir}/.shadow/skills/
    // let ws_dir = workspace_dir.join(".shadow").join("skills");
    // if ws_dir.exists() {
    //     skills.extend(load_skills_from_dir(&ws_dir)?);
    // }

    Ok(skills)
}

// ── 单元测试 ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ---- parse_skill_md 测试 ----

    #[test]
    fn test_parse_skill_md_basic() {
        let content = r#"---
name: git-helper
description: Git 操作辅助技能
---
# Git Helper
这是一个 git 辅助技能
"#;
        let skill = parse_skill_md(content).unwrap();
        assert_eq!(skill.name, "git-helper");
        assert_eq!(skill.description, "Git 操作辅助技能");
        assert!(skill.tools.is_empty());
        // body 应作为 prompt
        assert_eq!(skill.prompts.len(), 1);
        assert!(skill.prompts[0].contains("Git Helper"));
    }

    #[test]
    fn test_parse_skill_md_with_tools() {
        let content = r#"---
name: git-helper
description: Git 操作辅助技能
tools:
  - name: status
    description: 查看 git 状态
    kind: shell
    command: git status
  - name: log
    description: 查看 git 日志
    kind: shell
    command: git log --oneline -10
prompts:
  - "你是一个 git 专家"
---
正文
"#;
        let skill = parse_skill_md(content).unwrap();
        assert_eq!(skill.name, "git-helper");
        assert_eq!(skill.tools.len(), 2);

        let tool0 = &skill.tools[0];
        assert_eq!(tool0.name, "status");
        assert_eq!(tool0.kind, "shell");
        assert_eq!(tool0.command, "git status");

        let tool1 = &skill.tools[1];
        assert_eq!(tool1.name, "log");
        assert_eq!(tool1.command, "git log --oneline -10");

        // prompts: frontmatter 中的 1 条 + body 1 条
        assert_eq!(skill.prompts.len(), 2);
        assert_eq!(skill.prompts[0], "你是一个 git 专家");
    }

    #[test]
    fn test_parse_skill_md_with_args() {
        let content = r#"---
name: docker-helper
description: Docker 辅助技能
tools:
  - name: run
    description: 运行容器
    kind: shell
    command: docker run {image} {cmd}
    args:
      - image
      - cmd
---
"#;
        let skill = parse_skill_md(content).unwrap();
        assert_eq!(skill.tools.len(), 1);
        let tool = &skill.tools[0];
        assert_eq!(tool.name, "run");
        assert_eq!(tool.command, "docker run {image} {cmd}");
        assert_eq!(tool.args.len(), 2);
        assert_eq!(tool.args[0], "image");
        assert_eq!(tool.args[1], "cmd");
    }

    #[test]
    fn test_parse_skill_md_no_frontmatter() {
        let content = "just some text without frontmatter";
        let result = parse_skill_md(content);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("frontmatter"));
    }

    #[test]
    fn test_parse_skill_md_unclosed_frontmatter() {
        let content = "---\nname: test\nthis has no closing marker";
        let result = parse_skill_md(content);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("未闭合"));
    }

    #[test]
    fn test_parse_skill_md_empty_name() {
        let content = "---\nname: \ndescription: test\n---\nbody";
        let result = parse_skill_md(content);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("名称"));
    }

    #[test]
    fn test_parse_skill_md_no_tools_no_prompts() {
        let content = "---\nname: simple\ndescription: 简单技能\n---\n";
        let skill = parse_skill_md(content).unwrap();
        assert_eq!(skill.name, "simple");
        assert!(skill.tools.is_empty());
        assert!(skill.prompts.is_empty());
    }

    #[test]
    fn test_parse_skill_md_quoted_values() {
        let content = "---\nname: test-skill\ndescription: \"带引号的描述\"\nprompts:\n  - '单引号提示'\n  - \"双引号提示\"\n---\n";
        let skill = parse_skill_md(content).unwrap();
        assert_eq!(skill.description, "带引号的描述");
        assert_eq!(skill.prompts.len(), 2);
        assert_eq!(skill.prompts[0], "单引号提示");
        assert_eq!(skill.prompts[1], "双引号提示");
    }

    #[test]
    fn test_parse_skill_md_command_with_colon() {
        // 命令中包含冒号 (如 URL), 不应被误认为键值分隔符
        let content = "---\nname: api-test\ntools:\n  - name: curl\n    kind: shell\n    command: curl http://localhost:8080/api\n---\n";
        let skill = parse_skill_md(content).unwrap();
        assert_eq!(skill.tools[0].command, "curl http://localhost:8080/api");
    }

    // ---- load_skills_from_dir 测试 ----

    #[test]
    fn test_load_skills_from_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path().join(".shadow").join("skills");

        // 创建技能 1: git-helper
        let skill1_dir = skills_dir.join("git-helper");
        std::fs::create_dir_all(&skill1_dir).unwrap();
        std::fs::write(
            skill1_dir.join("SKILL.md"),
            "---\nname: git-helper\ndescription: Git 辅助\ntools:\n  - name: status\n    kind: shell\n    command: git status\n---\n",
        )
        .unwrap();

        // 创建技能 2: docker-helper
        let skill2_dir = skills_dir.join("docker-helper");
        std::fs::create_dir_all(&skill2_dir).unwrap();
        std::fs::write(
            skill2_dir.join("SKILL.md"),
            "---\nname: docker-helper\ndescription: Docker 辅助\n---\n",
        )
        .unwrap();

        // 创建一个非技能目录 (没有 SKILL.md)
        std::fs::create_dir_all(skills_dir.join("empty-skill")).unwrap();

        // 创建一个无效的 SKILL.md (解析失败)
        let bad_dir = skills_dir.join("bad-skill");
        std::fs::create_dir_all(&bad_dir).unwrap();
        std::fs::write(bad_dir.join("SKILL.md"), "no frontmatter here").unwrap();

        let skills = load_skills_from_dir(&skills_dir).unwrap();
        assert_eq!(skills.len(), 2); // bad-skill 应被跳过, empty-skill 应被跳过

        let names: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"git-helper"));
        assert!(names.contains(&"docker-helper"));
    }

    #[test]
    fn test_load_skills_from_dir_not_exists() {
        let skills = load_skills_from_dir(Path::new("/nonexistent/path/skills")).unwrap();
        assert!(skills.is_empty());
    }

    #[test]
    fn test_load_skills_from_dir_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let skills = load_skills_from_dir(tmp.path()).unwrap();
        assert!(skills.is_empty());
    }

    // ---- SkillsService 测试 ----

    #[test]
    fn test_skills_service_find() {
        let skills = vec![
            Skill {
                name: "skill-a".to_string(),
                description: "技能 A".to_string(),
                tools: vec![],
                prompts: vec![],
            },
            Skill {
                name: "skill-b".to_string(),
                description: "技能 B".to_string(),
                tools: vec![],
                prompts: vec![],
            },
        ];
        let service = SkillsService::from_skills(skills);

        assert_eq!(service.list().len(), 2);
        assert!(service.find("skill-a").is_some());
        assert!(service.find("skill-b").is_some());
        assert!(service.find("skill-c").is_none());
    }

    #[test]
    fn test_skills_service_all_tools() {
        let skills = vec![Skill {
            name: "git".to_string(),
            description: "Git 技能".to_string(),
            tools: vec![
                SkillTool {
                    name: "status".to_string(),
                    description: "查看状态".to_string(),
                    kind: "shell".to_string(),
                    command: "git status".to_string(),
                    args: vec![],
                },
                SkillTool {
                    name: "log".to_string(),
                    description: "查看日志".to_string(),
                    kind: "shell".to_string(),
                    command: "git log".to_string(),
                    args: vec![],
                },
                SkillTool {
                    name: "fetch".to_string(),
                    description: "HTTP 工具".to_string(),
                    kind: "http".to_string(),
                    command: "https://example.com/api".to_string(),
                    args: vec![],
                },
            ],
            prompts: vec![],
        }];
        let service = SkillsService::from_skills(skills);

        // shell 和 http 类型的工具都会被注册
        let tools = service.all_tools();
        assert_eq!(tools.len(), 3);
        let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        assert!(names.contains(&"git__status"));
        assert!(names.contains(&"git__log"));
        assert!(names.contains(&"git__fetch"));
    }

    // ---- YAML 解析器单元测试 ----

    #[test]
    fn test_yaml_parse_simple_mapping() {
        let yaml = "name: test\ndescription: hello";
        let value = parse_yaml(yaml).unwrap();
        assert_eq!(value["name"], "test");
        assert_eq!(value["description"], "hello");
    }

    #[test]
    fn test_yaml_parse_sequence() {
        let yaml = "items:\n  - one\n  - two\n  - three";
        let value = parse_yaml(yaml).unwrap();
        assert_eq!(value["items"][0], "one");
        assert_eq!(value["items"][1], "two");
        assert_eq!(value["items"][2], "three");
    }

    #[test]
    fn test_yaml_parse_sequence_of_objects() {
        let yaml = "tools:\n  - name: a\n    kind: shell\n  - name: b\n    kind: shell";
        let value = parse_yaml(yaml).unwrap();
        assert_eq!(value["tools"][0]["name"], "a");
        assert_eq!(value["tools"][0]["kind"], "shell");
        assert_eq!(value["tools"][1]["name"], "b");
    }

    #[test]
    fn test_yaml_parse_nested_args() {
        let yaml = "tools:\n  - name: run\n    args:\n      - image\n      - cmd";
        let value = parse_yaml(yaml).unwrap();
        assert_eq!(value["tools"][0]["args"][0], "image");
        assert_eq!(value["tools"][0]["args"][1], "cmd");
    }

    #[test]
    fn test_yaml_parse_empty() {
        let value = parse_yaml("").unwrap();
        assert!(value.is_null());
    }

    #[test]
    fn test_yaml_parse_quoted_strings() {
        let yaml = "name: \"quoted\"\ndesc: 'single'";
        let value = parse_yaml(yaml).unwrap();
        assert_eq!(value["name"], "quoted");
        assert_eq!(value["desc"], "single");
    }
}
