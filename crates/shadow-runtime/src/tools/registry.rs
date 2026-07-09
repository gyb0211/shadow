//! 工具注册表 -- 统一管理 agent 可用的工具

use shadow_core::{Tool, ToolSpec};

/// 工具注册表 -- 持有所有已注册的工具, 提供按名称查找
///
/// 替代原先 Agent 中的 `Vec<Box<dyn Tool>>`, 提供更清晰的 API:
/// - `register()`: 注册新工具
/// - `find()`: 按名称查找工具
/// - `specs()`: 导出所有工具规格 (给 LLM)
/// - `iter()`: 遍历所有工具
pub struct ToolRegistry {
    tools: Vec<Box<dyn Tool>>,
}

impl ToolRegistry {
    /// 创建空注册表
    pub fn new() -> Self {
        Self { tools: Vec::new() }
    }

    /// 注册一个工具
    pub fn register(&mut self, tool: Box<dyn Tool>) {
        self.tools.push(tool);
    }

    /// 批量注册工具 (从 Vec 扩展)
    pub fn extend(&mut self, tools: Vec<Box<dyn Tool>>) {
        self.tools.extend(tools);
    }

    /// 按名称查找工具 -- 返回不可变引用
    pub fn find(&self, name: &str) -> Option<&dyn Tool> {
        self.tools
            .iter()
            .find(|t| t.name() == name)
            .map(|t| t.as_ref())
    }

    /// 导出所有工具规格 (给 LLM 的 tool 列表)
    pub fn specs(&self) -> Vec<ToolSpec> {
        self.tools.iter().map(|t| t.spec()).collect()
    }

    /// 工具数量
    pub fn len(&self) -> usize {
        self.tools.len()
    }

    /// 是否为空
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    /// 遍历所有工具
    pub fn iter(&self) -> impl Iterator<Item = &dyn Tool> {
        self.tools.iter().map(|t| t.as_ref())
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}
