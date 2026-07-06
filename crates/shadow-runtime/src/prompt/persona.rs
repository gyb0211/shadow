//! Persona 系统 -- 多角色切换
//!
//! 允许 agent 在不同人格 (persona) 之间切换, 每个人格有专属的系统提示.
//! 参考 Hermes Agent 的 config.yaml personalities 设计.
//!
//! # 内置 Persona
//! 共 10 种: helpful / concise / technical / creative / teacher / code_reviewer /
//! explorer / planner / debugger / socratic
//!
//! # 使用方式
//! 1. [`get_persona`] 按名称获取 persona
//! 2. 将 [`PersonaSection`] 加入 [`SystemPromptBuilder`] (priority=99, 紧随 Identity)
//!
//! [`SystemPromptBuilder`]: super::SystemPromptBuilder

use super::{PromptContext, PromptSection};

/// 人格定义 -- 描述 agent 的行为风格与系统提示
#[derive(Debug, Clone)]
pub struct Persona {
    /// 人格名称 (唯一标识)
    pub name: &'static str,
    /// 人格描述 (供 UI 展示)
    pub description: &'static str,
    /// 系统提示内容 (注入 system prompt)
    pub system_prompt: &'static str,
}

/// 10 种内置 persona
///
/// 顺序即 [`list_personas`] 返回顺序.
pub const BUILTIN_PERSONAS: &[Persona] = &[
    Persona {
        name: "helpful",
        description: "通用助手 -- 乐于助人, 全面回答",
        system_prompt: "你是一个乐于助人的助手, 尽力提供准确、有用的回答. 遇到不确定的问题时坦诚说明, 而非编造答案.",
    },
    Persona {
        name: "concise",
        description: "简洁模式 -- 言简意赅, 不废话",
        system_prompt: "你是一个简洁的助手. 回答尽量简短直接, 避免不必要的解释和铺垫. 只在用户追问时才展开细节.",
    },
    Persona {
        name: "technical",
        description: "技术专家 -- 精确严谨, 使用专业术语",
        system_prompt: "你是一个技术专家. 回答时使用准确的专业术语, 注重技术细节和正确性. 提供代码示例和具体参数, 而非泛泛而谈.",
    },
    Persona {
        name: "creative",
        description: "创意伙伴 -- 鼓励发散思维",
        system_prompt: "你是一个创意伙伴. 鼓励发散思维, 提供新颖的想法和视角. 不要过早否定任何可能性, 帮助用户探索不同方向.",
    },
    Persona {
        name: "teacher",
        description: "教师 -- 循序渐进, 耐心引导",
        system_prompt: "你是一个耐心的教师. 循序渐进地解释概念, 从基础到进阶. 用类比和示例帮助理解, 主动确认用户是否跟上, 鼓励提问.",
    },
    Persona {
        name: "code_reviewer",
        description: "代码审查者 -- 关注质量与最佳实践",
        system_prompt: "你是一个严格的代码审查者. 关注代码质量、可读性、安全性和性能. 指出潜在问题并给出改进建议, 引用最佳实践和设计模式.",
    },
    Persona {
        name: "explorer",
        description: "探索者 -- 主动调研, 深入挖掘",
        system_prompt: "你是一个主动的探索者. 面对问题时主动调研上下文, 深入挖掘根因而非停留在表面. 使用工具收集信息后再下结论.",
    },
    Persona {
        name: "planner",
        description: "规划者 -- 先制定计划, 再执行",
        system_prompt: "你是一个善于规划的助手. 面对复杂任务时先制定清晰的步骤计划, 分解为可执行的子任务, 再逐步执行. 每步执行前说明意图.",
    },
    Persona {
        name: "debugger",
        description: "调试专家 -- 系统化排查问题",
        system_prompt: "你是一个调试专家. 系统化地排查问题: 复现 -> 定位 -> 分析根因 -> 验证修复. 善用工具收集诊断信息, 不靠猜测下结论.",
    },
    Persona {
        name: "socratic",
        description: "苏格拉底式 -- 用提问引导思考",
        system_prompt: "你采用苏格拉底式对话法. 通过提问引导用户自主思考, 而非直接给出答案. 帮助用户发现自身推理中的漏洞, 培养独立解决问题的能力.",
    },
];

/// 按名称获取内置 persona
///
/// # 参数
/// - `name`: persona 名称 (如 "helpful", "concise")
///
/// # 返回
/// 匹配的 [`Persona`], 未找到返回 `None`
#[must_use]
pub fn get_persona(name: &str) -> Option<Persona> {
    BUILTIN_PERSONAS.iter().find(|p| p.name == name).cloned()
}

/// 列出所有内置 persona 名称
#[must_use]
pub fn list_personas() -> Vec<&'static str> {
    BUILTIN_PERSONAS.iter().map(|p| p.name).collect()
}

/// 默认 persona (helpful)
#[must_use]
pub fn default_persona() -> Persona {
    BUILTIN_PERSONAS[0].clone()
}

/// 人格段 -- 将 persona 的系统提示注入 system prompt
///
/// priority=99, 紧随 Identity(100) 之后, 在 Bootstrap(95) 与 DateTime(90) 之前.
/// 这样身份声明在前, 人格风格紧跟其后.
pub struct PersonaSection {
    /// 人格定义
    persona: Persona,
}

impl PersonaSection {
    /// 创建人格段
    #[must_use]
    pub fn new(persona: Persona) -> Self {
        Self { persona }
    }

    /// 按名称创建人格段, 未找到名称时使用默认 persona
    #[must_use]
    pub fn from_name(name: &str) -> Self {
        let persona = get_persona(name).unwrap_or_else(default_persona);
        Self { persona }
    }

    /// 获取当前 persona 引用
    pub fn persona(&self) -> &Persona {
        &self.persona
    }
}

impl PromptSection for PersonaSection {
    fn name(&self) -> &str {
        "persona"
    }
    fn render(&self, _ctx: &PromptContext) -> String {
        format!(
            "[人格模式: {}]\n{}",
            self.persona.name, self.persona.system_prompt
        )
    }
    fn priority(&self) -> i32 {
        99
    }
}

// ── 单元测试 ──
#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompt::{PromptContext, SystemPromptBuilder};
    use std::path::PathBuf;

    /// 构造测试用 PromptContext
    fn make_ctx() -> PromptContext {
        PromptContext {
            alias: "shadow".to_string(),
            model: "gpt-4o".to_string(),
            tool_count: 5,
            workspace_dir: PathBuf::from("/tmp/workspace"),
        }
    }

    /// 测试: 内置 persona 数量为 10
    #[test]
    fn test_builtin_count() {
        assert_eq!(BUILTIN_PERSONAS.len(), 10, "应有 10 个内置 persona");
        assert_eq!(list_personas().len(), 10);
    }

    /// 测试: list_personas 返回预期名称
    #[test]
    fn test_list_personas() {
        let names = list_personas();
        assert!(names.contains(&"helpful"));
        assert!(names.contains(&"concise"));
        assert!(names.contains(&"technical"));
        assert!(names.contains(&"creative"));
        assert!(names.contains(&"teacher"));
        assert!(names.contains(&"code_reviewer"));
        assert!(names.contains(&"explorer"));
        assert!(names.contains(&"planner"));
        assert!(names.contains(&"debugger"));
        assert!(names.contains(&"socratic"));
    }

    /// 测试: get_persona 找到已知 persona
    #[test]
    fn test_get_persona_found() {
        let p = get_persona("concise").expect("concise 应存在");
        assert_eq!(p.name, "concise");
        assert!(!p.description.is_empty());
        assert!(!p.system_prompt.is_empty());
    }

    /// 测试: get_persona 未找到返回 None
    #[test]
    fn test_get_persona_not_found() {
        assert!(get_persona("nonexistent").is_none());
    }

    /// 测试: default_persona 返回 helpful
    #[test]
    fn test_default_persona() {
        let p = default_persona();
        assert_eq!(p.name, "helpful");
    }

    /// 测试: PersonaSection 名称与优先级
    #[test]
    fn test_section_name_priority() {
        let section = PersonaSection::from_name("technical");
        assert_eq!(section.name(), "persona");
        assert_eq!(section.priority(), 99);
    }

    /// 测试: PersonaSection 渲染包含人格名与提示
    #[test]
    fn test_section_render() {
        let section = PersonaSection::from_name("concise");
        let text = section.render(&make_ctx());
        assert!(text.contains("[人格模式: concise]"), "应包含人格标识");
        assert!(text.contains("简洁"), "应包含 concise 的系统提示关键词");
    }

    /// 测试: from_name 未找到时回退到默认 persona
    #[test]
    fn test_from_name_fallback() {
        let section = PersonaSection::from_name("unknown_persona");
        assert_eq!(section.persona().name, "helpful");
    }

    /// 测试: 在 SystemPromptBuilder 中, Persona(99) 应排在 Identity(100) 之后
    #[test]
    fn test_priority_after_identity() {
        let builder =
            SystemPromptBuilder::with_defaults().section(PersonaSection::from_name("technical"));
        let prompt = builder.build(&make_ctx());

        let identity_pos = prompt.find("你是 shadow").expect("应包含身份");
        let persona_pos = prompt.find("[人格模式: technical]").expect("应包含人格");

        // priority 降序: identity(100) > persona(99)
        assert!(
            identity_pos < persona_pos,
            "Identity(100) 应在 Persona(99) 之前"
        );
    }

    /// 测试: Persona(99) 应在 DateTime(90) 之前
    #[test]
    fn test_priority_before_datetime() {
        let builder =
            SystemPromptBuilder::with_defaults().section(PersonaSection::from_name("concise"));
        let prompt = builder.build(&make_ctx());

        let persona_pos = prompt.find("[人格模式: concise]").expect("应包含人格");
        let datetime_pos = prompt.find("当前时间").expect("应包含时间");

        assert!(
            persona_pos < datetime_pos,
            "Persona(99) 应在 DateTime(90) 之前"
        );
    }
}
