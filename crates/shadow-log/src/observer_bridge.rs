//! Observer 桥接 -- 将 LogEvent 投影到 ObserverEvent, 统一日志和观察者通道
//!
//! # 角色
//!
//! Shadow 的事件流有两个独立消费者:
//! 1. **writer.rs** -- LogEvent 持久化到 JSONL 文件 + broadcast 给订阅者
//! 2. **Observer trait** (shadow-core) -- 给 TUI / Prometheus / OTel 等后端用的指标通道
//!
//! 这两个通道消费**同一份** record! 事件, 但数据结构不同:
//! - LogEvent 是通用日志 schema (ECS/OTel 风格, 自由 action 字符串)
//! - ObserverEvent 是封闭枚举, 仅 ~20 个变体, 字段类型严格
//!
//! 本模块负责**投影**: LogEvent → Option<ObserverEvent>。
//! 投影规则按 `event.action` 分发到对应 ObserverEvent 变体,
//! 字段从 LogEvent.attribution / .attributes 里捞。
//!
//! # 全局单例
//!
//! 使用 `OnceLock<RwLock<Option<Arc<dyn Observer>>>>` 全局持有一个 observer。
//! 未调用 `set_observer_bridge` 时为 None, `forward` 直接 no-op 返回,
//! 让本 crate 在没有 observer 的场景下也能无副作用运行。
//!
//! # 已知小问题
//!
//! 见 `project` 函数内联 FIXME -- `messages_count` 字段名可能需要与 record! 宏端对齐。
//!
//! 参考 ZeroClaw observer_bridge.rs:
//! - LogEvent → ObserverEvent 投影 (只转发 metric 相关字段)
//! - 让 TUI/Prometheus 等后端消费同一份事件流
//! - 无 observer 绑定时 no-op

use crate::event::{LogEvent};
use parking_lot::RwLock;
use shadow_core::kennel::observer::TurnTokenUsage;
use shadow_core::{Observer, ObserverEvent};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

/// 全局 observer slot -- OnceLock 保证线程安全一次性初始化,
/// RwLock 允许运行时切换 observer (set/clear)。
static OBSERVER: OnceLock<RwLock<Option<Arc<dyn Observer>>>> = OnceLock::new();

/// 取全局 slot 的引用 -- 首次调用时初始化为 None。
fn slot() -> &'static RwLock<Option<Arc<dyn Observer>>> {
    OBSERVER.get_or_init(|| RwLock::new(None))
}

/// 安装日志观察者 -- 覆盖式 (后设置的胜出)。
/// 由 TUI / Prometheus exporter / OTel exporter 在启动时调用。
pub fn set_observer_bridge(observer: Arc<dyn Observer>) {
    *slot().write() = Some(observer);
}

/// 移除日志观察者 -- 进程关闭或后端切换时调用。
pub fn clear_observer_bridge() {
    *slot().write() = None;
}

/// 投影 LogEvent 到已绑定的观察者。
///
/// 调用方: `writer.rs::record_event` -- 每条 LogEvent 在落盘前先经过这里。
/// 无 observer 绑定时 no-op (早返回)。
///
/// 设计: 先用 `project` 把 LogEvent 投影为 ObserverEvent, 投影失败 (action 未识别)
/// 则跳过; 投影成功则通过 `Observer::record_event` 转发。
pub(crate) fn forward(event: &LogEvent) {
    let Some(observer) = slot().read().clone() else {
        return;
    };
    if let Some(obs_event) = project(event) {
        observer.record_event(&obs_event);
    }
}

/// LogEvent → ObserverEvent 投影函数。
///
/// 步骤:
/// 1. 从 attribution 提取公共字段 (model_provider / model / tool / channel / agent_alias / duration / success)
/// 2. 字符串字段归一化为 Option<String> (空串 → None), 让 ObserverEvent 字段语义清晰
/// 3. 按 `event.action` 分发到对应 ObserverEvent 变体
/// 4. action 不在白名单 -> 返回 None (该事件不转发给 observer)
///
/// # action → ObserverEvent 映射表
///
/// | action | ObserverEvent 变体 | 备注 |
/// |--------|---------------------|------|
/// | `agent_start` | AgentStart | |
/// | `agent_end` | AgentEnd | 含 token / cost 提取 |
/// | `llm_request` | LlmRequest | |
/// | `llm_response` | LlmResponse | 含 token / error |
/// | `tool_call_start` | ToolCallStart | tool_call_id / arguments 未填 |
/// | `tool_call` / `tool_call_result` | ToolCall | 二者合并到同一变体 |
/// | `channel_message_inbound` | ChannelMessage | direction="inbound" |
/// | `channel_send` | ChannelMessage | direction="outbound" |
/// | `turn_complete` | TurnComplete | |
/// | `heartbeat_tick` | HeartbeatTick | |
/// | `error` | Error | component 取 channel_type |
fn project(event: &LogEvent) -> Option<ObserverEvent> {
    use crate::event::type_field;
    let action = event.event.action.as_str();
    let attribution = &event.attribution;

    // model_provider: 优先取 model_provider_type (复合前缀展开字段),
    // 兜底取 model_provider (完整复合值), 最终都没有则空串。
    // 注意: 这与 "完整复合值" 还是 "type 部分" 的选择会影响聚合维度 --
    // 当前选 type (如 "openai") 而非 "openai.standard", 这是设计选择。
    let model_provider = attribution
        .get(&type_field("model_provider"))
        .or_else(|| attribution.get("model_provider"))
        .unwrap_or_default()
        .to_string();

    let model = attribution.get("model").unwrap_or_default().to_string();
    let tool = attribution.get("tool").unwrap_or_default().to_string();
    let channel = attribution.get("channel").unwrap_or_default().to_string();
    let duration = attribution
        .duration_ms
        .map(Duration::from_millis)
        .unwrap_or_default();
    let success = matches!(event.event.outcome.as_str(), "success");


    let agent_alias = attribution
        .get("agent_alias")
        .or_else(|| {
            event
                .attributes
                .get("agent_alias")
                .and_then(serde_json::Value::as_str)
        })
        .unwrap_or_default()
        .to_string();

    // turn_id: 优先 attributes.turn_id (本轮对话 id), 兜底 trace_id (跨进程链路 id)
    // -- 二者语义不同, 这里"混用"是为了让没有显式 turn_id 的事件也能被 observer 关联。
    let turn_id = event
        .attributes
        .get("turn_id")
        .or_else(|| event.attributes.get("trace_id"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string();

    // 空串 -> None 归一化, 让 ObserverEvent 的 Option<String> 字段语义明确
    // (None = "未提供", Some("") = 没有意义, 故统一转 None)
    let channel_opt = if channel.is_empty() {
        None
    } else {
        Some(channel.clone())
    };
    let agent_alias_opt = if agent_alias.is_empty() {
        None
    } else {
        Some(agent_alias)
    };
    let turn_id_opt = if turn_id.is_empty() {
        None
    } else {
        Some(turn_id)
    };

    match action {
        "agent_start" => Some(ObserverEvent::AgentStart {
            model_provider,
            model,
            channel: channel_opt,
            agent_alias: agent_alias_opt,
            turn_id: turn_id_opt,
        }),
        "agent_end" => Some(ObserverEvent::AgentEnd {
            model_provider,
            model,
            duration,
            // token 用量: 必须同时有 input + output, 否则视为未观测 (None)
            token_used: {
                let input = event
                    .attributes
                    .get("input_tokens")
                    .and_then(serde_json::Value::as_u64);
                let output = event
                    .attributes
                    .get("output_tokens")
                    .and_then(serde_json::Value::as_u64);
                match (input, output) {
                    (Some(input_tokens), Some(output_tokens)) => Some(TurnTokenUsage {
                        input_tokens,
                        output_tokens,
                    }),
                    _ => None,
                }
            },
            cost_usd: event
                .attributes
                .get("cost_usd")
                .and_then(serde_json::Value::as_f64),
            channel: channel_opt,
            agent_alias: agent_alias_opt,
            turn_id: turn_id_opt,
        }),
        "llm_request" => Some(ObserverEvent::LlmRequest {
            model_provider,
            model,
            message_count: event
                .attributes
                .get("message_count")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or_default() as usize,
            channel: channel_opt,
            agent_alias: agent_alias_opt,
            turn_id: turn_id_opt,
        }),
        "llm_response" => Some(ObserverEvent::LlmResponse {
            model_provider,
            model,
            duration,
            success,
            error_message: event
                .attributes
                .get("error")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string),
            input_tokens: event
                .attributes
                .get("input_tokens")
                .and_then(serde_json::Value::as_u64),
            output_tokens: event
                .attributes
                .get("output_tokens")
                .and_then(serde_json::Value::as_u64),
            messages: None,
            channel: channel_opt,
            agent_alias: agent_alias_opt,
            turn_id: turn_id_opt,
        }),
        "tool_call_start" => Some(ObserverEvent::ToolCallStart {
            tool,
            // 以下两个字段 record! 宏端目前不传, 留 None
            // 未来若宏支持 tool_call_id / arguments, 可从 attributes 提取
            tool_call_id: None,
            arguments: None,
            channel: channel_opt,
            agent_alias: agent_alias_opt,
            turn_id: turn_id_opt,
        }),
        // tool_call 和 tool_call_result 合并到 ToolCall 变体 -- 让 observer 端
        // 只看到一个 "工具调用完成" 事件, 减少噪音。
        "tool_call" | "tool_call_result" => Some(ObserverEvent::ToolCall {
            tool,
            tool_call_id: None,
            duration,
            success,
            arguments: None,
            result: None,
            channel: channel_opt,
            agent_alias: agent_alias_opt,
            turn_id: turn_id_opt,
        }),
        "channel_message_inbound" => Some(ObserverEvent::ChannelMessage {
            channel,
            direction: "inbound".to_string(),
        }),
        "channel_send" => Some(ObserverEvent::ChannelMessage {
            channel,
            direction: "outbound".to_string(),
        }),
        "turn_complete" => Some(ObserverEvent::TurnComplete),
        "heartbeat_tick" => Some(ObserverEvent::HeartbeatTick),
        "error" => Some(ObserverEvent::Error {
            component: attribution
                .get(&type_field("channel"))
                .unwrap_or("system")
                .to_string(),
            message: event.message.clone().unwrap_or_default(),
        }),
        // 未识别的 action 不转发 -- 避免给 observer 通道塞噪音。
        // 这与 Action 封闭枚举 (event.rs) 的设计一致: 新增 action 必须在此显式添加分支。
        _ => None,
    }
}
