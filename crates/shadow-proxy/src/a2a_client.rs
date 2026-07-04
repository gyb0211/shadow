//! A2A 客户端 -- 通过 HTTP JSON-RPC 与远程 agent 通信
//!
//! 协议: JSON-RPC 2.0 over HTTP (A2A v1.0)
//! 参考: ZeroClaw gateway/a2a.rs

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use futures::stream::BoxStream;
use futures::StreamExt;
use serde_json::{json, Value};

use crate::card::AgentCard;
use crate::transport::AgentTransport;

/// A2A 远程客户端
pub struct A2aClient {
    card: AgentCard,
    http: reqwest::Client,
    auth_token: Option<String>,
}

impl A2aClient {
    /// 创建 A2A 客户端
    pub fn new(name: &str, endpoint: &str, capabilities: Vec<String>) -> Self {
        let card = AgentCard::a2a(name, endpoint, capabilities);
        Self {
            card,
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(300))
                .build()
                .unwrap_or_default(),
            auth_token: None,
        }
    }

    /// 设置认证 token
    pub fn with_auth(mut self, token: impl Into<String>) -> Self {
        self.auth_token = Some(token.into());
        self
    }

    /// 发送 JSON-RPC 请求
    async fn rpc(&self, method: &str, params: Value) -> Result<Value> {
        let endpoint = self.card.endpoint.as_deref().context("A2A card 缺少 endpoint")?;
        let body = json!({
            "jsonrpc": "2.0",
            "id": uuid::Uuid::new_v4().to_string(),
            "method": method,
            "params": params
        });

        let mut req = self.http.post(endpoint).json(&body);
        if let Some(token) = &self.auth_token {
            req = req.header("Authorization", token);
        }

        let resp = req.send().await.context("A2A HTTP 请求失败")?;
        if !resp.status().is_success() {
            bail!("A2A HTTP 错误: {} {}", resp.status(), endpoint);
        }

        let json: Value = resp.json().await.context("A2A 响应解析失败")?;

        if let Some(error) = json.get("error") {
            bail!("A2A RPC 错误: {}", error);
        }

        Ok(json.get("result").cloned().unwrap_or(Value::Null))
    }

    /// 发现远程 agent 的能力 (GET agent-card.json)
    pub async fn discover(&self) -> Result<AgentCard> {
        let endpoint = self.card.endpoint.as_deref().context("A2A card 缺少 endpoint")?;
        // 尝试 /.well-known/agent-card.json
        let base = endpoint.trim_end_matches('/');
        // 从 endpoint 提取 origin (http://host:port)
        let origin = base.rsplit_once("/a2a/").map(|(o, _)| o).unwrap_or(base);

        let card_url = format!("{origin}/.well-known/agent-card.json");
        let mut req = self.http.get(&card_url);
        if let Some(token) = &self.auth_token {
            req = req.header("Authorization", token);
        }

        let resp = req.send().await.context("A2A 发现请求失败")?;
        if !resp.status().is_success() {
            bail!("A2A 发现失败: {} {}", resp.status(), card_url);
        }

        let card: AgentCard = resp.json().await.context("A2A agent-card 解析失败")?;
        Ok(card)
    }
}

#[async_trait]
impl AgentTransport for A2aClient {
    async fn chat(&self, prompt: &str) -> Result<String> {
        let result = self.rpc("message/send", json!({
            "message": {
                "parts": [{ "kind": "text", "text": prompt }]
            }
        }))
        .await?;

        // 解析 A2A Task 响应
        // { "id": "...", "status": {"state": "completed"}, "artifacts": [{"parts": [{"kind": "text", "text": "..."}]}] }
        if let Some(artifacts) = result.get("artifacts").and_then(|v| v.as_array()) {
            let mut texts = Vec::new();
            for artifact in artifacts {
                if let Some(parts) = artifact.get("parts").and_then(|v| v.as_array()) {
                    for part in parts {
                        if part.get("kind").and_then(|v| v.as_str()) == Some("text")
                            && let Some(text) = part.get("text").and_then(|v| v.as_str()) {
                                texts.push(text.to_string());
                            }
                    }
                }
            }
            if !texts.is_empty() {
                return Ok(texts.join("\n"));
            }
        }

        // 兜底: 返回原始 JSON
        Ok(serde_json::to_string_pretty(&result)?)
    }

    async fn chat_stream(&self, prompt: &str) -> BoxStream<'_, Result<String>> {
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

    #[test]
    fn a2a_client_creation() {
        let client = A2aClient::new("remote", "http://host:9090/a2a/remote", vec!["coding".into()]);
        assert_eq!(client.card().name, "remote");
        assert_eq!(client.card().transport, crate::card::TransportKind::A2a);
        assert_eq!(client.card().endpoint.as_deref(), Some("http://host:9090/a2a/remote"));
    }

    #[test]
    fn a2a_client_with_auth() {
        let client = A2aClient::new("remote", "http://host:9090/a2a/remote", vec![])
            .with_auth("bearer token123");
        assert_eq!(client.auth_token.as_deref(), Some("bearer token123"));
    }
}
