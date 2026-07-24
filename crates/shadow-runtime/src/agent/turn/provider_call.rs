use crate::agent::turn::context::TurnCtx;
use crate::agent::turn::events::StreamDelta;
use anyhow::Result;
use shadow_core::{ChatMessage, ChatRequest, ChatResponse, ModelProvider, ToolSpec, NATIVE_THINKING_OVERRIDE};
use shadow_core::kennel::provider::NativeThinkingParams;
use shadow_providers::ProviderDispatch;

pub(crate) struct ProviderCallOutcome {
    pub chat_result: Result<ChatResponse>,
    pub streamed_live_delta: bool,
    pub streamed_protocol_suppressed: bool,
    pub streamed_visible_text: String,
}
// if let Some(tx) = ctx.on_delta {
// let phase = if iteration == 0 {
// "\u{1f914} Thinking...\n".to_string()
// } else {
// format!("\u{1f914} Thinking (round {})...\n", iteration + 1)
// };
// let _ = tx.send(StreamDelta::Status(phase)).await;
// }

pub async fn call_provider(ctx: &TurnCtx<'_>,
                           model_provider: &dyn ModelProvider,
                           model: &str,
                           messages: &[ChatMessage],
                           tools: Option<&[ToolSpec]>,
                           should_consume_provider_stream: bool,
                           iteration: usize) -> Result<ProviderCallOutcome> {
    let mut streamed_live_delta = false;
    let mut streamed_protocol_suppressed = false;
    let mut streamed_visible_text = String::new();

    let chat_result = if should_consume_provider_stream {
        Ok(ChatResponse{
            text: Some("暂未实现stream call".to_string()),
            tool_calls: vec![],
            usage: None,
            reasoning_content: None,
        })
    }else{
        let dispatcher = ProviderDispatch::from_ref(model_provider);
        let chat_future = dispatcher.chat(
            ChatRequest{
                messages,
                tools,
                thinking: NATIVE_THINKING_OVERRIDE.try_with(Clone::clone).ok().flatten()
            },
            model,
            ctx.temperature,
        );
        chat_future.await

    };

    Ok(ProviderCallOutcome{
        chat_result,streamed_live_delta,streamed_protocol_suppressed,streamed_visible_text
    })

}
