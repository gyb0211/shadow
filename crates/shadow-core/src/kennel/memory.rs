//! 记忆 trait -- 对话存储与检索
//!
//! 参考 ZeroClaw 设计, 重构为分参数 store + 枚举 category + session 过滤。

use crate::kennel::attribution::{Attributable, Role};
use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExportFilter {
    pub namespace: Option<String>,
    pub session_id: Option<String>,
    pub category: Option<MemoryCategory>,
    pub since: Option<String>,
    pub until: Option<String>,
}
/// 记忆分类枚举
///
/// - [`MemoryCategory::Core`] — 长期事实/偏好
/// - [`MemoryCategory::Daily`] — 日常会话
/// - [`MemoryCategory::Conversation`] — 对话上下文
/// - [`MemoryCategory::Custom`] — 自定义分类
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryCategory {
    /// 长期事实/偏好
    Core,
    /// 日常会话
    Daily,
    /// 对话上下文
    Conversation,
    /// 自定义分类 (传入任意字符串)
    Custom(String),
}

impl MemoryCategory {
    /// 返回分类的字符串标识
    ///
    /// Core -> "core", Daily -> "daily", Conversation -> "conversation",
    /// Custom(s) -> s
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Core => "core",
            Self::Daily => "daily",
            Self::Conversation => "conversation",
            Self::Custom(s) => s.as_str(),
        }
    }

    /// 从字符串解析为 MemoryCategory
    ///
    /// "core" -> Core, "daily" -> Daily, "conversation" -> Conversation,
    /// 其他 -> Custom(s)
    #[must_use]
    pub fn from_name(s: &str) -> Self {
        match s {
            "core" => Self::Core,
            "daily" => Self::Daily,
            "conversation" => Self::Conversation,
            other => Self::Custom(other.to_string()),
        }
    }
}

impl fmt::Display for MemoryCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

// 自定义序列化: 序列化为字符串
// Core -> "core", Daily -> "daily", Conversation -> "conversation", Custom(s) -> s
impl Serialize for MemoryCategory {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

// 自定义反序列化: 从字符串解析
impl<'de> Deserialize<'de> for MemoryCategory {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Ok(Self::from_name(&s))
    }
}

/// 记忆条目
///
/// 一条记忆的完整数据。timestamp 使用 RFC 3339 字符串 (序列化友好),
/// score 为检索相关度 (仅 recall 结果有意义)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    /// 唯一标识 (uuid)
    pub id: String,
    #[serde(default, alias = "agent_id")]
    pub agent_id: Option<String>,
    /// 业务键 (用于 get/forget)
    pub key: String,
    /// 记忆内容
    pub content: String,
    /// 时间戳 (RFC 3339 字符串)
    pub timestamp: String,
    /// 会话 ID (可选, 用于会话级记忆隔离)
    pub session_id: Option<String>,
    /// 检索相关度 (仅 recall 结果有意义)
    pub score: Option<f64>,
    /// Agent 别名 (归因)
    #[serde(default)]
    pub agent_alias: Option<String>,
    #[serde(default = "default_namespace")]
    pub namespace: String,
    /// 检索权重 重要程度
    #[serde(default)]
    pub importance: Option<f64>,
    /// 如果这条记录被新的取代了 这里就是新的id
    #[serde(default)]
    pub superseded_by: Option<String>,
    /// 内存类型 与 持久性和实效性无关
    pub kind: Option<MemoryKind>,
    /// 是否受到预算驱逐保护（？ 压缩时 这段记忆必须保留？）
    pub pinned: bool,
    /// 多用户内存隔离范围（租户或终端）
    pub tenant_id: Option<String>,

    pub category: MemoryCategory,
}

pub fn default_namespace() -> String {
    "default".to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryKind {
    /// 场景记忆 记录什么时候 在那里 发生了什么事
    Episodic,
    /// 语义记忆 记录事实和知识
    Semantic(SemanticSubType),
    /// 程序性记忆 记录怎么做 如何用rust打开一个文件
    Procedural,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticSubType {
    /// 偏好
    Preference,
    /// 事实
    Fact,
    /// 决策
    Decision,
    /// 实体
    Entity,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProceduralMessage {
    pub role: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StoreOptions {
    pub namespace: Option<String>,
    pub importance: Option<f64>,
    pub kind: Option<MemoryKind>,
    pub pinned: bool,
    pub tenant_id: Option<String>,
}

impl StoreOptions {
    pub fn with_namespace(mut self, namespace: impl Into<String>) -> Self {
        self.namespace = Some(namespace.into());
        self
    }

    pub fn with_importance(mut self, importance: f64) -> Self {
        self.importance = Some(importance);
        self
    }

    pub fn with_kind(mut self, kind: MemoryKind) -> Self {
        self.kind = Some(kind);
        self
    }

    pub fn pinned(mut self, pinned: bool) -> Self {
        self.pinned = pinned;
        self
    }

    pub fn with_tenant_id(mut self, tenant_id: impl Into<String>) -> Self {
        self.tenant_id = Some(tenant_id.into());
        self
    }
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MemoryStats {
    pub total_rows: u64,
    pub by_category: Vec<(String, u64)>,
    pub superseded_rows: u64,
    pub pinned_rows: u64,
    pub bytes: u64,
}

/// 记忆后端 trait
///
/// 每个存储后端实现此 trait (SQLite/Markdown/None...)。
/// 参考 ZeroClaw 设计: store 使用分参数, 调用方无需构造完整 MemoryEntry。
#[async_trait]
pub trait Memory: Attributable {
    /// 后端名称 (如 "sqlite" / "markdown" / "none")
    fn name(&self) -> &str;

    /// 存储记忆 -- 分参数, 后端负责生成 id/timestamp
    async fn store(
        &self,
        key: &str,
        content: &str,
        category: MemoryCategory,
        session_id: Option<&str>,
    ) -> Result<()>;

    /// 检索记忆 -- 支持关键词 + session 过滤
    async fn recall(
        &self,
        query: &str,
        limit: usize,
        session_id: Option<&str>,
        since: Option<&str>,
        until: Option<&str>,
    ) -> Result<Vec<MemoryEntry>>;

    /// 获取单条记忆 (按 key)
    async fn get(&self, key: &str) -> Result<Option<MemoryEntry>>;

    /// 获取单条记忆 (按 key)
    async fn get_for_agent(&self, key: &str, agent_id: &str) -> Result<Option<MemoryEntry>> {
        let hit = self.get(key).await?;
        Ok(hit.filter(|v| v.agent_id.as_deref() == Some(agent_id)))
    }

    /// 列出记忆 (可选按 category 过滤)
    async fn list(
        &self,
        category: Option<&MemoryCategory>,
        session_id: Option<&str>,
    ) -> Result<Vec<MemoryEntry>>;

    /// 删除记忆, 返回是否删除成功
    async fn forget(&self, key: &str) -> Result<bool>;

    /// 删除记忆, 返回是否删除成功
    async fn forget_for_agent(&self, key: &str) -> Result<bool>;

    async fn purge_namespace(&self, _namespace: &str) -> Result<usize> {
        anyhow::bail!("purge_namespace not supported by this memory backend")
    }

    async fn purge_session(&self, _session_id: &str) -> Result<usize> {
        anyhow::bail!("purge_session not supported by this memory backend")
    }

    async fn purge_session_for_agent(&self, _session_id: &str, _agent_id: &str) -> Result<usize> {
        anyhow::bail!("purge_session_for_agent not supported by this memory backend ")
    }

    async fn purge_agent(&self, _agent_alias: &str) -> Result<usize> {
        anyhow::bail!("purge_agent not supported by this memory backend ")
    }

    async fn export_agent(&self, _agent_alias: &str) -> Result<Vec<MemoryEntry>> {
        Ok(Vec::new())
    }

    async fn rename_agent(&self, _agent_alias: &str) -> Result<usize> {
        anyhow::bail!("rename_agent not supported by this memory backend ")
    }

    /// 记忆总数
    async fn count_agent(&self, _agent_alias: &str) -> Result<usize> {
        Ok(0)
    }

    /// 记忆总数
    async fn count(&self) -> Result<usize>;

    /// 健康检查
    async fn health_check(&self) -> bool;

    async fn supersede(&self, _superseded_ids: &[String], _new_id: &str) -> Result<()> {
        Ok(())
    }

    async fn store_procedural(
        &self,
        _messages: &[ProceduralMessage],
        _session_id: &str,
    ) -> Result<()> {
        Ok(())
    }

    async fn count_in_scope(
        &self,
        _namespace: Option<&str>,
        _category: Option<&MemoryCategory>,
    ) -> Result<u64> {
        Ok(0)
    }

    async fn stats(&self) -> Result<MemoryStats> {
        Ok(MemoryStats::default())
    }

    async fn reindex(&self) -> Result<usize> {
        Ok(0)
    }

    async fn refresh_embedder(
        &self,
        _model_provider: &str,
        _api_key: Option<&str>,
        _model: &str,
        _dimensions: usize,
    ) {
    }

    async fn recall_namespaced(
        &self,
        namespace: &str,
        query: &str,
        limit: usize,
        session_id: Option<&str>,
        since: Option<&str>,
        until: Option<&str>,
    ) -> Result<Vec<MemoryEntry>> {
        let entries = self
            .recall(query, limit * 2, session_id, since, until)
            .await?;
        let filtered = entries
            .into_iter()
            .filter(|e| e.namespace == namespace)
            .take(limit)
            .collect();
        Ok(filtered)
    }

    async fn export(&self, filter: &ExportFilter) -> Result<Vec<MemoryEntry>> {
        let entries = self
            .list(filter.category.as_ref(), filter.session_id.as_deref())
            .await?;

        let filtered = entries
            .into_iter()
            .filter(|e| {
                if let Some(ref ns) = filter.namespace
                    && e.namespace != *ns
                {
                    return false;
                }
                if let Some(ref since) = filter.since
                    && e.timestamp.as_str() < since.as_str()
                {
                    return false;
                }

                if let Some(ref until) = filter.until
                    && e.timestamp.as_str() > until.as_str()
                {
                    return false;
                }
                true
            })
            .collect();
        Ok(filtered)
    }

    async fn store_with_metadata(
        &self,
        key: &str,
        content: &str,
        category: MemoryCategory,
        session_id: Option<&str>,
        _namespace: Option<&str>,
        _importance: Option<f64>,
    ) -> Result<()> {
        self.store(key, content, category, session_id).await
    }

    async fn store_with_options(
        &self,
        key: &str,
        content: &str,
        category: MemoryCategory,
        session_id: Option<&str>,
        options: StoreOptions,
    ) -> Result<()> {
        self.store_with_metadata(
            key,
            content,
            category,
            session_id,
            options.namespace.as_deref(),
            options.importance,
        )
        .await
    }

    async fn store_with_agent(
        &self,
        key: &str,
        content: &str,
        category: MemoryCategory,
        session_id: Option<&str>,
        _namespace: Option<&str>,
        _importance: Option<f64>,
        agent_id: Option<&str>,
    ) -> Result<()>;

    async fn recall_for_agent(
        &self,
        allowed_agent_ids: &[&str],
        query: &str,
        limit: usize,
        session_id: Option<&str>,
        since: Option<&str>,
        until: Option<&str>,
    ) -> Result<Vec<MemoryEntry>>;

    async fn ensure_agent_uuid(&self, alias: &str) -> Result<String> {
        Ok(alias.to_string())
    }
}

// ── 记忆策略 trait ──

#[async_trait]
pub trait MemoryStrategy: Send + Sync {
    /// 加载并格式化 轮次相关的记忆上下文
    async fn load_context(
        &self,
        observer: &dyn crate::kennel::observer::Observer,
        query: &str,
        session_id: Option<&str>,
    ) -> Vec<MemoryEntry>;

    /// 将对话转为长期记忆
    async fn consolidate_turn(
        &self,
        user_message: &str,
        assistant_response: &str,
        session_id: Option<&str>,
    ) -> Result<()>;

    // 运行一个内存治理 （清理 归档 后台整合）
    async fn run_governance(&self) -> Result<()>;
}

// 共享内存策略决策
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "decision")]
pub enum MemoryPolicyDecision {
    /// 同意
    Allow,
    /// 拒绝并给出理由
    Deny { reason: String },
}

/// 当召回查询应被解释为最近/仅时间召回时返回 true
pub fn is_recent_recall_query(query: &str) -> bool {
    let trimmed = query.trim();
    trimmed.is_empty() || trimmed == "*"
}

/// 将近期/仅时间的召回查询标准化为对后端中立的空查询
pub fn normalize_recent_recall_query(query: &str) -> &str {
    if is_recent_recall_query(query) {
        ""
    } else {
        query
    }
}
