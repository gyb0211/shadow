//! Embedding provider -- 文本向量化
//!
//! 提供:
//! - [`EmbeddingProvider`] trait: 文本转向量的统一接口
//! - [`NoopEmbedding`]: 空实现, 用于无 embedding 场景 (退化为纯 FTS5)
//! - [`OpenAiEmbedding`]: 调用 OpenAI 兼容 /v1/embeddings API
//! - [`create_embedding_provider`] — 工厂函数

use anyhow::Context;
use anyhow::Result;
use async_trait::async_trait;

/// Embedding provider trait -- 将文本转换为向量
///
/// 实现方负责调用具体的 embedding API (如 OpenAI / Ollama)。
/// 当 provider 为 Noop 时, SqliteMemory 退化为纯 FTS5 检索。
#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    /// Provider 名称 (如 "openai" / "none")
    fn name(&self) -> &str;

    /// Embedding 维度 (Noop 返回 0)
    fn dimensions(&self) -> usize;

    /// 批量文本转向量
    async fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>>;

    /// 单条文本转向量 (默认实现: 调用 embed 取第一个)
    async fn embed_one(&self, text: &str) -> Result<Vec<f32>> {
        let mut results = self.embed(&[text]).await?;
        results.pop().context("embedding provider 返回空结果")
    }

    /// 是否为 Noop (无 embedding 能力), 用于 SqliteMemory 判断退化逻辑
    fn is_noop(&self) -> bool {
        false
    }
}

// ── Noop embedding (纯关键词检索退化) ──────────────────────────

/// 空 embedding provider -- 不调用任何 API, 返回空向量
///
/// 当未配置 embedding 时使用, SqliteMemory 会退化为纯 FTS5 检索。
pub struct NoopEmbedding;

#[async_trait]
impl EmbeddingProvider for NoopEmbedding {
    fn name(&self) -> &str {
        "none"
    }

    fn dimensions(&self) -> usize {
        0
    }

    async fn embed(&self, _texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        Ok(Vec::new())
    }

    fn is_noop(&self) -> bool {
        true
    }
}

// ── OpenAI 兼容 embedding provider ─────────────────────────────

/// OpenAI 兼容 embedding provider -- 调用 /v1/embeddings API
///
/// 支持 OpenAI / OpenRouter / Ollama 等兼容 endpoint。
/// base_url 尾部斜杠自动去除, 自动拼接 /embeddings 或 /v1/embeddings。
pub struct OpenAiEmbedding {
    base_url: String,
    api_key: String,
    model: String,
    dims: usize,
    /// 复用连接池
    client: reqwest::Client,
}

impl OpenAiEmbedding {
    /// 构造器 -- 创建 OpenAI 兼容 embedding provider
    ///
    /// - `base_url`: API 根地址 (如 "https://api.openai.com")
    /// - `api_key`: API 密钥 (可为空, 用于本地 ollama 等匿名端点)
    /// - `model`: 模型名 (如 "text-embedding-3-small")
    /// - `dims`: 向量维度 (如 1536)
    #[must_use]
    pub fn new(base_url: &str, api_key: &str, model: &str, dims: usize) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: api_key.to_string(),
            model: model.to_string(),
            dims,
            client: reqwest::Client::new(),
        }
    }

    /// base_url 是否已有显式 API 路径 (非根 /)
    fn has_explicit_api_path(&self) -> bool {
        let Ok(url) = reqwest::Url::parse(&self.base_url) else {
            return false;
        };
        let path = url.path().trim_end_matches('/');
        !path.is_empty() && path != "/"
    }

    /// base_url 是否已指向 embeddings 端点
    fn has_embeddings_endpoint(&self) -> bool {
        let Ok(url) = reqwest::Url::parse(&self.base_url) else {
            return false;
        };
        url.path().trim_end_matches('/').ends_with("/embeddings")
    }

    /// 计算完整的 embeddings API URL
    fn embeddings_url(&self) -> String {
        // 已包含 /embeddings 端点, 直接使用
        if self.has_embeddings_endpoint() {
            return self.base_url.clone();
        }
        // 已有显式路径 (如 /v1), 追加 /embeddings
        if self.has_explicit_api_path() {
            return format!("{}/embeddings", self.base_url);
        }
        // 仅根域名, 追加 /v1/embeddings
        format!("{}/v1/embeddings", self.base_url)
    }
}

#[async_trait]
impl EmbeddingProvider for OpenAiEmbedding {
    fn name(&self) -> &str {
        "openai"
    }

    fn dimensions(&self) -> usize {
        self.dims
    }

    async fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        let body = serde_json::json!({
            "model": self.model,
            "input": texts,
        });

        let resp = self
            .client
            .post(self.embeddings_url())
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .context("embedding API 请求失败")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("embedding API 错误 {status}: {text}");
        }

        let json: serde_json::Value = resp.json().await.context("解析 embedding 响应失败")?;
        let data = json
            .get("data")
            .and_then(|d| d.as_array())
            .context("embedding 响应缺少 'data' 字段")?;

        let mut embeddings = Vec::with_capacity(data.len());
        for item in data {
            let embedding = item
                .get("embedding")
                .and_then(|e| e.as_array())
                .context("embedding 响应项缺少 'embedding' 数组")?;

            #[allow(clippy::cast_possible_truncation)]
            let vec: Vec<f32> = embedding
                .iter()
                .filter_map(|v| v.as_f64().map(|f| f as f32))
                .collect();

            embeddings.push(vec);
        }

        Ok(embeddings)
    }
}

// ── 工厂函数 ──────────────────────────────────────────────────

/// 创建 embedding provider 的工厂函数
///
/// - `"none"` / `""` / 未知: 返回 NoopEmbedding (退化为纯 FTS5)
/// - `"openai"`: 使用 api.openai.com
/// - `"openrouter"`: 使用 openrouter.ai/api/v1
/// - `"custom:<url>"`: 使用自定义 OpenAI 兼容端点
#[must_use]
pub fn create_embedding_provider(
    provider: &str,
    api_key: Option<&str>,
    model: &str,
    dims: usize,
) -> Box<dyn EmbeddingProvider> {
    let key = api_key.unwrap_or("");
    match provider {
        "openai" => Box::new(OpenAiEmbedding::new(
            "https://api.openai.com",
            key,
            model,
            dims,
        )),
        "openrouter" => Box::new(OpenAiEmbedding::new(
            "https://openrouter.ai/api/v1",
            key,
            model,
            dims,
        )),
        name if name.starts_with("custom:") => {
            let base_url = name.strip_prefix("custom:").unwrap_or("");
            Box::new(OpenAiEmbedding::new(base_url, key, model, dims))
        }
        _ => Box::new(NoopEmbedding),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noop_name() {
        let p = NoopEmbedding;
        assert_eq!(p.name(), "none");
        assert_eq!(p.dimensions(), 0);
        assert!(p.is_noop());
    }

    #[tokio::test]
    async fn noop_embed_returns_empty() {
        let p = NoopEmbedding;
        let result = p.embed(&["hello"]).await.unwrap();
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn noop_embed_one_returns_error() {
        let p = NoopEmbedding;
        let result = p.embed_one("hello").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn noop_embed_empty_batch() {
        let p = NoopEmbedding;
        let result = p.embed(&[]).await.unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn factory_none() {
        let p = create_embedding_provider("none", None, "model", 1536);
        assert_eq!(p.name(), "none");
        assert!(p.is_noop());
    }

    #[test]
    fn factory_openai() {
        let p = create_embedding_provider("openai", Some("key"), "text-embedding-3-small", 1536);
        assert_eq!(p.name(), "openai");
        assert_eq!(p.dimensions(), 1536);
        assert!(!p.is_noop());
    }

    #[test]
    fn factory_openrouter() {
        let p = create_embedding_provider(
            "openrouter",
            Some("sk-or-test"),
            "openai/text-embedding-3-small",
            1536,
        );
        assert_eq!(p.name(), "openai");
        assert_eq!(p.dimensions(), 1536);
    }

    #[test]
    fn factory_custom_url() {
        let p = create_embedding_provider("custom:http://localhost:1234", None, "model", 768);
        assert_eq!(p.name(), "openai");
        assert_eq!(p.dimensions(), 768);
    }

    #[test]
    fn factory_empty_string_returns_noop() {
        let p = create_embedding_provider("", None, "model", 1536);
        assert_eq!(p.name(), "none");
    }

    #[test]
    fn factory_unknown_provider_returns_noop() {
        let p = create_embedding_provider("cohere", None, "model", 1536);
        assert_eq!(p.name(), "none");
    }

    #[test]
    fn openai_trailing_slash_stripped() {
        let p = OpenAiEmbedding::new("https://api.openai.com/", "key", "model", 1536);
        assert_eq!(p.base_url, "https://api.openai.com");
    }

    #[test]
    fn openai_dimensions_custom() {
        let p = OpenAiEmbedding::new("http://localhost", "k", "m", 384);
        assert_eq!(p.dimensions(), 384);
    }

    #[test]
    fn embeddings_url_standard_openai() {
        let p = OpenAiEmbedding::new("https://api.openai.com", "key", "model", 1536);
        assert_eq!(p.embeddings_url(), "https://api.openai.com/v1/embeddings");
    }

    #[test]
    fn embeddings_url_openrouter() {
        let p = OpenAiEmbedding::new(
            "https://openrouter.ai/api/v1",
            "key",
            "openai/text-embedding-3-small",
            1536,
        );
        assert_eq!(
            p.embeddings_url(),
            "https://openrouter.ai/api/v1/embeddings"
        );
    }

    #[test]
    fn embeddings_url_custom_full_endpoint() {
        let p = OpenAiEmbedding::new(
            "https://my-api.example.com/api/v2/embeddings",
            "key",
            "model",
            1536,
        );
        assert_eq!(
            p.embeddings_url(),
            "https://my-api.example.com/api/v2/embeddings"
        );
    }

    #[test]
    fn embeddings_url_base_with_v1_no_duplicate() {
        let p = OpenAiEmbedding::new("https://api.example.com/v1", "key", "model", 1536);
        assert_eq!(p.embeddings_url(), "https://api.example.com/v1/embeddings");
    }
}
