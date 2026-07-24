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


pub async fn run_tool_call_loop(tool_loop : ToolLoop<'_>) -> anyhow::Result<String> {


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
        // observer: &(),
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
            history,None,false,iteration
        ).await?;

        let (response_text) = match chat_result{
            Ok(resp) => {
                (resp.text.as_deref().unwrap_or("").to_string())

            },
            Err(e) =>return Err(e)
        };

        history.push(ChatMessage::assistant(response_text.clone().to_string()));

        return Ok(response_text.to_string())
    }


    Ok("12312".to_string())

}
#[derive(Debug)]
pub struct ModelSwitchRequested {
    pub model_provider: String,
    pub model: String,
}

impl std::fmt::Display for ModelSwitchRequested {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "model switch requested to {} {}",
            self.model_provider, self.model
        )
    }
}


#[derive(Debug)]
pub struct ToolLoopCancelled;

impl std::fmt::Display for ToolLoopCancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("tool loop cancelled")
    }
}

impl std::error::Error for ToolLoopCancelled {}

impl std::error::Error for ModelSwitchRequested {}
pub fn is_model_switch_requested(error: &anyhow::Error) -> Option<(String, String)> {
    error.chain().filter_map(|s| s.downcast_ref::<ModelSwitchRequested>())
        .map(|e| (e.model_provider.clone(), e.model.clone())).next()
}

pub fn is_tool_loop_cancelled(error: &anyhow::Error) -> bool{
    error.chain().any(|e| e.is::<ToolLoopCancelled>())
}