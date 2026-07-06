//! 进程内 Agent 传输 -- 同 Shadow 实例, 不同配置的 agent

use anyhow::Result;
use async_trait::async_trait;
use futures::stream::BoxStream;
use futures::StreamExt;
use shadow_core::{ChatMessage, ChatRequest, ModelProvider};
use std::sync::Arc;

use crate::card::AgentCard;
use crate::transport::AgentTransport;

/// 进程内 agent -- 共享 Provider, 不同 system prompt / model
pub struct LocalAgent {
    card: AgentCard,
    provider: Arc<dyn ModelProvider>,
    model: String,
    system_prompt: Option<String>,
    temperature: Option<f64>,
}

impl LocalAgent {
    pub fn new(
        name: &str,
        capabilities: Vec<String>,
        provider: Arc<dyn ModelProvider>,
        model: String,
    ) -> Self {
        Self {
            card: AgentCard::local(name, capabilities),
            provider,
            model,
            system_prompt: None,
            temperature: None,
        }
    }

    pub fn with_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = Some(prompt.into());
        self
    }

    pub fn with_temperature(mut self, temp: f64) -> Self {
        self.temperature = Some(temp);
        self
    }
}

#[async_trait]
impl AgentTransport for LocalAgent {
    async fn chat(&self, prompt: &str) -> Result<String> {
        let mut messages = Vec::new();
        if let Some(sys) = &self.system_prompt {
            messages.push(ChatMessage {
                role: "system".into(),
                content: sys.clone(),
                ..Default::default()
            });
        }
        messages.push(ChatMessage {
            role: "user".into(),
            content: prompt.to_string(),
            ..Default::default()
        });

        let request = ChatRequest {
            messages,
            model: self.model.clone(),
            temperature: self.temperature,
            max_tokens: None,
            tools: vec![],
        };

        let response = self.provider.chat(request).await?;
        Ok(response.content)
    }

    async fn chat_stream(&self, prompt: &str) -> BoxStream<'_, Result<String>> {
        // 简化版: 先同步获取, 再包装成 stream
        // 完整版应调用 provider.chat_stream()
        let result = self.chat(prompt).await;
        futures::stream::once(async move { result }).boxed()
    }

    fn card(&self) -> &AgentCard {
        &self.card
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::AgentTransport;
    use shadow_core::{Attributable, ChatResponse, ChatRequest, ModelProvider, Role, TokenUsage};

    struct MockProvider;

    impl Attributable for MockProvider {
        fn role(&self) -> Role { Role::Agent }
        fn alias(&self) -> &str { "mock" }
    }

    #[async_trait::async_trait]
    impl ModelProvider for MockProvider {
        fn provider_type(&self) -> &str { "mock" }
        async fn chat(&self, _request: ChatRequest) -> Result<ChatResponse> {
            Ok(ChatResponse {
                content: "mock response".to_string(),
                reasoning_content: None,
                tool_calls: vec![],
                usage: TokenUsage::default(),
            })
        }
        // chat_stream 使用默认实现
        async fn list_models(&self) -> Result<Vec<String>> {
            Ok(vec!["mock-model".into()])
        }
    }

    #[tokio::test]
    async fn local_agent_chat() {
        let provider: Arc<dyn ModelProvider> = Arc::new(MockProvider);
        let agent = LocalAgent::new("test", vec!["coding".into()], provider, "test-model".into());

        let result = agent.chat("hello").await.unwrap();
        assert_eq!(result, "mock response");
        assert_eq!(agent.card().name, "test");
        assert!(agent.card().has_capability("coding"));
    }
}
