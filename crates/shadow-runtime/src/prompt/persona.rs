//! Persona 系统 -- 多角色切换 (配置文件驱动)
//!
//! 人格定义从 config.toml 的 [personas] 段加载,
//! 内置 10 种作为 fallback (配置未指定时使用).
//!
//! 配置示例:
//! ```toml
//! [personas]
//! helpful = "你是一个乐于助人的助手."
//! pirate = "Arrr! 你是海盗船长!"
//! my_custom = "你是一个自定义角色."
//! ```
//!
//! 如果配置中有同名的 persona, 配置覆盖内置定义.

use super::{PromptContext, PromptSection};
use std::collections::HashMap;

/// 人格定义 -- 描述 agent 的行为风格与系统提示
#[derive(Debug, Clone)]
pub struct Persona {
    /// 人格名称 (唯一标识)
    pub name: String,
    /// 人格描述 (供 UI 展示)
    pub description: String,
    /// 系统提示内容 (注入 system prompt)
    pub system_prompt: String,
}

/// 内置默认 persona (fallback, 当配置文件未指定时使用)
fn builtin_personas() -> Vec<Persona> {
    vec![
        Persona { name: "helpful".into(), description: "通用助手".into(), system_prompt: "你是一个乐于助人的助手, 尽力提供准确、有用的回答. 遇到不确定的问题时坦诚说明, 而非编造答案.".into() },
        Persona { name: "concise".into(), description: "简洁模式".into(), system_prompt: "你是一个简洁的助手. 回答尽量简短直接, 避免不必要的解释和铺垫.".into() },
        Persona { name: "technical".into(), description: "技术专家".into(), system_prompt: "你是一个技术专家. 回答时使用准确的专业术语, 注重技术细节和正确性.".into() },
        Persona { name: "creative".into(), description: "创意伙伴".into(), system_prompt: "你是一个创意伙伴. 鼓励发散思维, 提供新颖的想法和视角.".into() },
        Persona { name: "teacher".into(), description: "教师".into(), system_prompt: "你是一个耐心的教师. 循序渐进地解释概念, 用类比和示例帮助理解.".into() },
        Persona { name: "code_reviewer".into(), description: "代码审查者".into(), system_prompt: "你是一个严格的代码审查者. 关注代码质量、可读性、安全性和性能.".into() },
        Persona { name: "explorer".into(), description: "探索者".into(), system_prompt: "你是一个主动的探索者. 面对问题时主动调研上下文, 深入挖掘根因.".into() },
        Persona { name: "planner".into(), description: "规划者".into(), system_prompt: "你是一个善于规划的助手. 面对复杂任务时先制定清晰的步骤计划, 再逐步执行.".into() },
        Persona { name: "debugger".into(), description: "调试专家".into(), system_prompt: "你是一个调试专家. 系统化地排查问题: 复现->定位->分析根因->验证修复.".into() },
        Persona { name: "socratic".into(), description: "苏格拉底式".into(), system_prompt: "你是一个苏格拉底式助手. 用提问引导用户思考, 而非直接给出答案.".into() },
    ]
}

/// 从配置加载 persona 列表
///
/// 合并策略: 配置中的 persona 优先, 内置作为 fallback.
/// 如果配置中有同名的, 配置覆盖内置定义.
pub fn load_personas(config_personas: &HashMap<String, String>) -> Vec<Persona> {
    let mut result: HashMap<String, Persona> = HashMap::new();

    // 1. 先加载内置
    for p in builtin_personas() {
        result.insert(p.name.clone(), p);
    }

    // 2. 配置覆盖/新增
    for (name, prompt) in config_personas {
        let existing = result.get(name);
        let description = existing.map(|p| p.description.clone()).unwrap_or_else(|| "自定义".to_string());
        result.insert(name.clone(), Persona {
            name: name.clone(),
            description,
            system_prompt: prompt.clone(),
        });
    }

    // 3. 排序: 内置在前 (按 builtin 顺序), 自定义在后 (按字母序)
    let builtin_order: Vec<String> = builtin_personas().iter().map(|p| p.name.clone()).collect();
    let mut all: Vec<Persona> = result.into_values().collect();
    all.sort_by(|a, b| {
        let ai = builtin_order.iter().position(|n| n == &a.name);
        let bi = builtin_order.iter().position(|n| n == &b.name);
        match (ai, bi) {
            (Some(ai), Some(bi)) => ai.cmp(&bi),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a.name.cmp(&b.name),
        }
    });

    all
}

/// 按名称获取 persona
///
/// 先从配置加载的列表中查找, 如果没有则从内置中查找.
pub fn get_persona(name: &str, config_personas: &HashMap<String, String>) -> Option<Persona> {
    // 配置优先
    if let Some(prompt) = config_personas.get(name) {
        let builtin = builtin_personas().into_iter().find(|p| p.name == name);
        let description = builtin.map(|p| p.description).unwrap_or_else(|| "自定义".to_string());
        return Some(Persona { name: name.to_string(), description, system_prompt: prompt.clone() });
    }
    // 内置 fallback
    builtin_personas().into_iter().find(|p| p.name == name)
}

/// 列出所有 persona 名称
pub fn list_personas(config_personas: &HashMap<String, String>) -> Vec<String> {
    load_personas(config_personas).into_iter().map(|p| p.name).collect()
}

/// 默认 persona
pub fn default_persona() -> Persona {
    builtin_personas().into_iter().next().unwrap()
}

/// Persona PromptSection -- 注入 system prompt
pub struct PersonaSection {
    persona: Persona,
}

impl PersonaSection {
    /// 用指定 persona 创建
    pub fn new(persona: Persona) -> Self {
        Self { persona }
    }

    /// 按名称创建, 未找到则回退到默认
    pub fn from_name(name: &str, config_personas: &HashMap<String, String>) -> Self {
        let persona = get_persona(name, config_personas).unwrap_or_else(|| default_persona());
        Self { persona }
    }

    /// 获取 persona 引用
    pub fn persona(&self) -> &Persona {
        &self.persona
    }
}

impl PromptSection for PersonaSection {
    fn name(&self) -> &str {
        "persona"
    }
    fn render(&self, _ctx: &PromptContext) -> String {
        format!("[人格模式: {}]\n{}", self.persona.name, self.persona.system_prompt)
    }
    fn priority(&self) -> i32 {
        99
    }
}

