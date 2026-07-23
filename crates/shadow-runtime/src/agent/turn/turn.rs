use std::sync::Arc;
use std::time::Instant;
use shadow_core::ChatMessage;
use crate::agent::turn::provider_call::call_provider;

pub struct ToolLoop<'a> {
    // pub exec: ResolvedAgentExecution<'a>,
    pub history: &'a mut Vec<ChatMessage>,
    pub channel_name: &'a str,
    pub channel_reply_target: Option<&'a str>,
    // pub cancellation_token: Option<CancellationToken>,
    // pub on_delta: Option<Arc<std::sync::mpsc::Sender<StreamDelta>>>,
    pub agent_alias: Option<&'a str>,
    pub turn_id: &'a str,


}


pub fn run_tool_call_loop(tool_loop : ToolLoop) -> anyhow::Result<String> {

    let ToolLoop {
        history,
        ..
    } = tool_loop;

    let max_iterations = 50;

    let loop_start = Instant::now();

    for iteration in 0..max_iterations{
        let ProviderCallOutcome {

        } = call_provider(
            &ctx,

        )
    }


}