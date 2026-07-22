//! 集中式 Attributable impl -- 为本 crate 所有 Tool 实现归因
//!
//! 每行 tool_attribution! 生成 Role::Tool(ToolKind) + alias = Tool::name()
//! 日志归因使用 <kind>.<alias> 复合标识, 和 channel/provider/memory 一致.
//!
//! 新增 Tool 时在此文件追加一行即可, 无需在工具源码里写 impl Attributable.

use shadow_core::{tool_attribution, ToolKind};
use crate::tools::cron::add::CronAddTool;
use crate::tools::model_switch::ModelSwitchTool;

// tool_attribution!(CronAddTool, ToolKind::Plugin);
tool_attribution!(ModelSwitchTool, ToolKind::Plugin);