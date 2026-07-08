use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RuntimeProfileConfig{
    /// 是否启动自主代理模式（多轮对话+工具调用循环）
    pub agentic: bool,
    /// 最大tool-call循环次数
    pub max_tool_iterations:usize,
    /// 每小时允许的最大操作数
    pub max_actions_per_hour: u32,
    /// 每日最高使用成本
    pub max_cost_per_day_cents: u32,
    /// shell子进程命令超时时间 0 继承全局配置
    pub shell_timeout_secs: u64,
    /// 最大递归调用深度
    pub max_delegation_depth:u32,
    /// 委托调用超时时间
    pub delegation_timeout_secs: Option<u64>,
    /// 代理委托运行超时时间
    pub agentic_timeout_secs: Option<u64>,
    /// 最大历史消息长度
    pub max_history_messages: Option<usize>,
    /// 最大上下文token数
    pub max_context_tokens: Option<usize>,
    /// 压缩上下文
    pub compact_context: Option<bool>,
    /// 并行tool-call
    pub parallel_tools: Option<bool>,
    /// tool 分发策略
    pub tool_dispatcher: Option<String>,
    /// 不受重复调用检查的工具
    pub tool_call_dedup_exempt: Vec<String>,
    /// 系统提示词最大字符数
    pub max_system_prompt_chars: Option<usize>,
    /// 工具调用结果最大字符数
    pub max_tool_result_chars: Option<usize>,
    /// 工具调用保持多少轮对话
    pub keep_tool_context_turns: Option<usize>,
    /// 每轮最多携带多少条记忆信息
    pub memory_recall_limit: Option<usize>,
    /// 是否启动严格工具解析
    pub strict_tool_parsing: bool,

    // todo!()
    // pub thinking: bool,
    // pub history_pruning: bool,
    // pub eval: bool,
    // pub auto_classify: bool,
    // pub context_compression: bool,
    // pub tool_receipts: bool,
    // pub tool_filter_groups: bool,
}

impl Default for RuntimeProfileConfig{
    fn default() -> Self {
        Self{
            agentic: false,
            max_tool_iterations: 0,
            max_actions_per_hour:30,
            max_cost_per_day_cents: 500,
            shell_timeout_secs: 60,
            max_delegation_depth: 0,
            delegation_timeout_secs: None,
            agentic_timeout_secs: None,
            max_history_messages: None,
            max_context_tokens: None,
            compact_context: None,
            parallel_tools: None,
            tool_dispatcher: None,
            tool_call_dedup_exempt: vec![],
            max_system_prompt_chars: None,
            max_tool_result_chars: None,
            keep_tool_context_turns: None,
            memory_recall_limit: None,
            strict_tool_parsing: false,
        }
    }
}