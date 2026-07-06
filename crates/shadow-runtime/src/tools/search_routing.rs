//! Web 搜索引擎路由 -- 根据 query 类型自动选择搜索引擎
//!
//! 路由规则:
//! - 代码/技术搜索 -> Bing (代码索引更全)
//! - 新闻/时事搜索 -> Google (时效性更好)
//! - 隐私/匿名搜索 -> DuckDuckGo (不追踪)
//! - 默认 -> DuckDuckGo

use serde_json::{Value, json};

/// 搜索引擎类型
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchEngine {
    DuckDuckGo,
    Google,
    Bing,
}

impl SearchEngine {
    pub fn as_str(&self) -> &str {
        match self {
            Self::DuckDuckGo => "duckduckgo",
            Self::Google => "google",
            Self::Bing => "bing",
        }
    }
}

/// 根据 query 内容智能选择搜索引擎
pub fn route_query(query: &str) -> SearchEngine {
    let lower = query.to_lowercase();

    // 代码/技术关键词 -> Bing
    let code_keywords = [
        "rust",
        "python",
        "java",
        "javascript",
        "typescript",
        "golang",
        "code",
        "function",
        "error",
        "stack trace",
        "compile",
        "api",
        "github",
        "regex",
        "sql",
        "docker",
        "kubernetes",
        "linux",
        "shell",
        "bash",
        "cargo",
        "npm",
        "pip",
        "crash",
        "bug",
        "debug",
    ];
    if code_keywords.iter().any(|k| lower.contains(k)) {
        return SearchEngine::Bing;
    }

    // 新闻/时事关键词 -> Google
    let news_keywords = [
        "新闻", "today", "latest", "breaking", "update", "2024", "2025", "2026", "刚刚", "最新",
        "今天", "发生", "热点", "时事",
    ];
    if news_keywords.iter().any(|k| lower.contains(k)) {
        return SearchEngine::Google;
    }

    // 隐私关键词 -> DuckDuckGo
    let privacy_keywords = ["private", "anonymous", "隐私", "匿名", "secret", "password"];
    if privacy_keywords.iter().any(|k| lower.contains(k)) {
        return SearchEngine::DuckDuckGo;
    }

    // 默认 -> DuckDuckGo
    SearchEngine::DuckDuckGo
}

/// 路由结果描述 (给 LLM 看的解释)
pub fn route_description(query: &str) -> String {
    let engine = route_query(query);
    let reason = match engine {
        SearchEngine::Bing => "代码/技术搜索",
        SearchEngine::Google => "新闻/时事搜索",
        SearchEngine::DuckDuckGo => "默认/隐私搜索",
    };
    format!("搜索引擎: {} (原因: {reason})", engine.as_str())
}

/// 构建 WebSearchTool 的参数
pub fn build_search_params(query: &str, max_results: usize) -> Value {
    let engine = route_query(query);
    json!({
        "query": query,
        "max_results": max_results,
        "engine": engine.as_str()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_route_code() {
        assert_eq!(route_query("rust async error handling"), SearchEngine::Bing);
        assert_eq!(route_query("python regex example"), SearchEngine::Bing);
        assert_eq!(route_query("docker compose yaml"), SearchEngine::Bing);
        assert_eq!(route_query("cargo build error"), SearchEngine::Bing);
    }

    #[test]
    fn test_route_news() {
        assert_eq!(route_query("latest AI news today"), SearchEngine::Google);
        assert_eq!(route_query("2026 最新科技新闻"), SearchEngine::Google);
        assert_eq!(route_query("breaking news update"), SearchEngine::Google);
    }

    #[test]
    fn test_route_privacy() {
        assert_eq!(
            route_query("private anonymous search"),
            SearchEngine::DuckDuckGo
        );
        assert_eq!(route_query("隐私匿名搜索"), SearchEngine::DuckDuckGo);
    }

    #[test]
    fn test_route_default() {
        assert_eq!(route_query("hello world"), SearchEngine::DuckDuckGo);
        assert_eq!(route_query("天气"), SearchEngine::DuckDuckGo);
        assert_eq!(route_query("how to cook pasta"), SearchEngine::DuckDuckGo);
    }

    #[test]
    fn test_build_params() {
        let params = build_search_params("rust error", 5);
        assert_eq!(params["query"], "rust error");
        assert_eq!(params["engine"], "bing");
        assert_eq!(params["max_results"], 5);
    }

    #[test]
    fn test_description() {
        let desc = route_description("python regex");
        assert!(desc.contains("bing"));
        assert!(desc.contains("代码"));
    }
}
