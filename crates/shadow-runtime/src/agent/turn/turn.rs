use std::sync::Arc;
use std::time::Instant;
use shadow_core::ChatMessage;
use crate::agent::turn::context::TurnCtx;
use crate::agent::turn::execution::{ResolvedAgentExecution, ResolvedModelAccess};
use crate::agent::turn::provider_call::{call_provider, ProviderCallOutcome};

pub struct ToolLoop<'a> {
    pub exec: ResolvedAgentExecution<'a>,
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
        exec: ResolvedAgentExecution{
            model_access: ResolvedModelAccess{
                model_provider,
                provider_name,
                model,
                temperature,
            } ,
        },
        history,
        channel_name,
        channel_reply_target,
        agent_alias,
        turn_id,

        ..
    } = tool_loop;
    let ctx = TurnCtx{
        observer: &(),
        on_delta: None,
        event_tx: None,
        temperature: None,
        agent_alias,
        turn_id,
    };

    let (model_provider,provider_name, model) = (model_provider, provider_name, model);

    let max_iterations = 50;

    let loop_start = Instant::now();

    for iteration in 0..max_iterations{
        let ProviderCallOutcome {
            chat_result,
            ..
        } = call_provider(
            &ctx,
            model_provider,
            model,
            history,None,should_consume_provider_stream,iteration
        );
    }


}