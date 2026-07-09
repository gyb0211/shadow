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
