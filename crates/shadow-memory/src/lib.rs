//! 影子记忆后端实现
//!
//! 当前实现: None (空) + Markdown (文件) + SQLite (FTS5 + 语义检索)
//!
//! 语义检索模块:
//! - [`embedding`]: EmbeddingProvider trait + Noop/OpenAI 实现
//! - [`vector`]: 余弦相似度 + 混合检索融合

pub mod markdown;
pub mod none;
// pub mod sqlite;
pub mod agent_scoped_markdown;
pub mod embedding;
pub mod sqlite;
pub mod strategy;
pub mod vector;
pub mod agent_scoped;

use std::fmt::{Debug, Formatter};
use std::path::Path;
pub use strategy::{DefaultMemoryStrategy, format_entries};

use crate::agent_scoped_markdown::{AgentScopedMarkdownMemory, MarkdownPeer};
use crate::markdown::MarkdownMemory;
use crate::none::NoneMemory;
use crate::sqlite::SqliteMemory;
use anyhow::{Context, Result};
use embedding::EmbeddingProvider;
use serde::{Deserialize, Serialize};
use shadow_config::{resolve_provider, Config};
use shadow_config::multi::alias_agent::MemoryBackendKind;
use shadow_config::providers::ModelProviders;
use shadow_config::schema::{ActiveStorage, EmbeddingRouteConfig, MemoryConfig};
use shadow_core::Memory;
use std::sync::Arc;
use crate::agent_scoped::AgentScopedMemory;

fn resolve_provider_ref(model_provider: String,
                        model: String, dimensions: usize, explicit_api_key: Option<String>,
                        inherited_api_key: Option<String>, providers: Option<&ModelProviders>) -> ResolvedEmbeddingConfig {
    let trimmed = model_provider.trim();
    let is_dotted_ref = !trimmed.is_empty()
        && !trimmed.starts_with("custom")
        && trimmed.contains('.');
    if !is_dotted_ref {
        return ResolvedEmbeddingConfig {
            model_provider,
            model,dimensions,
            api_key: explicit_api_key.or(inherited_api_key),
        };
    }
    let reference = trimmed.to_string();
    let Some((kind, _alias, provider_cfg)) =
    providers.and_then(|pros| pros.find_by_name(&reference))
    else {
        // todo record
        return ResolvedEmbeddingConfig {
            model_provider, model, dimensions,
            api_key: explicit_api_key.or(inherited_api_key)
        };
    };

    let provider_key = provider_cfg.api_key.as_deref()
        .map(str::trim).filter(|value| !value.is_empty())
        .map(str::to_string);

    let concrete_provider = match provider_cfg.uri.as_deref().map(str::trim).filter(|value| !value.is_empty())
    {
        Some(uri) => Some(format!("custom:{uri}")),
        None if matches!(kind, "openai" | "openrouter") => Some(kind.to_string()),
        None => None,
    };

    let Some(concrete_provider) = concrete_provider else {
        return ResolvedEmbeddingConfig {
            model_provider, model, dimensions,
            api_key: explicit_api_key.or(inherited_api_key)
        };
    };

    ResolvedEmbeddingConfig {
        model_provider, model, dimensions,
        api_key: explicit_api_key.or(provider_key).or(inherited_api_key)
    }

}
pub async fn create_memory_for_agent(
    config: &Config,
    agent_alias: &str,
    api_key: Option<&str>,
) -> Result<Arc<dyn Memory>> {
    let agent_cfg = config
        .agents
        .get(agent_alias)
        .with_context(|| format!("agents.{agent_alias} is not configured."))?;

    let backend_kind = agent_cfg.memory.backend;
    if matches!(backend_kind, MemoryBackendKind::Markdown) {
        let own_workspace = config.agent_workspace_dir(agent_alias);
        let own = MarkdownMemory::new("markdown", &own_workspace);

        let mut peers = Vec::new();

        for peer in &agent_cfg.workspace.read_memory_from {
            let peer_alias = peer.as_str();
            let peer_workspace = config.agent_workspace_dir(agent_alias);
            peers.push(MarkdownPeer {
                alias: peer_alias.to_string(),
                memory: MarkdownMemory::new("markdown", &peer_workspace),
            });
        }

        let scoped = AgentScopedMarkdownMemory::new(agent_alias, own, peers);
        return Ok(Arc::new(scoped));
    }

    if matches!(backend_kind, MemoryBackendKind::None) {
        return Ok(Arc::new(NoneMemory::new("none")));
    }

    let inner = create_memory_with_storage_and_routes(
        &config.memory,
        &config.embedding_routes,
        config.resolve_active_storage(),
        &config.data_dir,
        api_key,
        Some(&config.providers.models),
    )?;
    let inner_arc: Arc<dyn Memory> = Arc::from(inner);

    let bound_id = inner_arc.ensure_agent_uuid(agent_alias).await?;

    let mut allowlist_ids = Vec::with_capacity(agent_cfg.workspace.read_memory_from.len());

    for peer in &agent_cfg.workspace.read_memory_from {
        let uuid = inner_arc.ensure_agent_uuid(agent_alias);
        allowlist_ids.push(uuid);
    }
    let scoped = AgentScopedMemory::new(inner_arc, bound_id, allowlist_ids);
    Ok(Arc::new(scoped))
}

pub fn create_memory_with_storage_and_routes(
    config: &MemoryConfig,
    embedding_routes: &[EmbeddingRouteConfig],
    active_storage: ActiveStorage<'_>,
    workspace_dir: &Path,
    api_key: Option<&str>,
    providers: Option<&ModelProviders>,
) -> anyhow::Result<Box<dyn Memory>> {
    let backend_name = backend_kind_from_dotted(&config.backend);
    let backend_kind = classify_memory_backend(&backend_name);
    let resolved_embedding = resolve_embedding_config(config, embedding_routes, api_key, providers);

    // todo hygiene

    // todo snapshot hygiene

    // todo auto_hydrate

    fn build_sqlite_memory(
        config: &MemoryConfig,
        sqlite_open_timeout_secs: Option<u64>,
        workspace_dir: &Path,
        resolved_embedding: &ResolvedEmbeddingConfig,
    ) -> anyhow::Result<SqliteMemory> {
        let embedder: Arc<dyn EmbeddingProvider> = Arc::from(embedding::create_embedding_provider(
            &resolved_embedding.model_provider,
            resolved_embedding.api_key.as_deref(),
            &resolved_embedding.model,
            resolved_embedding.dimensions,
        ));

        #[allow(clippy::cast_possible_truncation)]
        let mem = SqliteMemory::with_embedder(
            "sqlite",
            workspace_dir,
            embedder,
            config.vector_weight as f32,
            config.keyword_weight as f32,
            config.embedding_cache_size,
            sqlite_open_timeout_secs,
            config.search_mode.clone(),
        )?;
        Ok(mem)
    }

    let sqlite_open_secs = match active_storage{

        ActiveStorage::Sqlite(sqlite) => sqlite.open_timeout_secs,
        _ => None,
    };


    create_memory_with_builders(
        &backend_name, workspace_dir,|| {
            build_sqlite_memory(
                config,
                sqlite_open_secs,workspace_dir, &resolved_embedding
            )
        } ,""
    )

}

fn create_memory_with_builders<F>(
    backend_name: &str,
    workspace_dir: &Path,
    mut sqlite_builder: F,
    unknown_context: &str,
) -> anyhow::Result<Box<dyn Memory>>
where
    F: FnMut() -> anyhow::Result<SqliteMemory>,
{
    match classify_memory_backend(backend_name) {
        MemoryBackendKind::Sqlite => Ok(Box::new(sqlite_builder()?)),
        // MemoryBackendKind::Lucid => {
        //     let local = sqlite_builder()?;
        //     Ok(Box::new(LucidMemory::new("lucid", workspace_dir, local)))
        // }
        // MemoryBackendKind::Postgres => {
        //     // Postgres requires a typed `[storage.postgres.<alias>]` config, which this
        //     // builder-only entry point does not receive. All supported call paths go
        //     // through `create_memory_with_storage_and_routes`, which handles postgres via
        //     // an early return. Fail loudly if a caller ever reaches this arm, rather than
        //     // pretending to work with default configs that can never connect.
        //     anyhow::bail!(
        //         "postgres backend requires storage config; \
        //          call create_memory_with_storage_and_routes instead of create_memory_with_builders"
        //     )
        // }
        // MemoryBackendKind::Qdrant | MemoryBackendKind::Markdown => {
        //     Ok(Box::new(MarkdownMemory::new("markdown", workspace_dir)))
        // }
        MemoryBackendKind::None => Ok(Box::new(NoneMemory::new("none"))),
        _ => {
            Ok(Box::new(MarkdownMemory::new("markdown", workspace_dir)))
        }
    }
}

pub fn resolve_embedding_config(
    config: &MemoryConfig,
    embedding_routes: &[EmbeddingRouteConfig],
    api_key: Option<&str>,
    providers: Option<&ModelProviders>,
) -> ResolvedEmbeddingConfig {
    let inherited_api_key = api_key
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);

    let configured_api_key = config
        .embedding_api_key.as_deref()
        .map(str::trim).filter(|value| !value.is_empty())
        .map(str::to_string);

    let fallback = || {
        resolve_provider_ref(
            config.embedding_provider.to_string(),
            config.embedding_model.to_string(),
            config.embedding_dimensions,
            configured_api_key.clone(),
            inherited_api_key.clone(),
            providers
        )
    };

    let Some(hint) = config.embedding_model.strip_prefix("hint:").map(str::trim)
        .filter(|value| !value.is_empty()) else {
        return fallback();
    };

    let Some(route) = embedding_routes.iter()
        .find(|route| route.hint.trim() == hint)
    else {
        return fallback();
    };

    let model_provider = route.model_provider.as_str();
    let model = route.model.as_str();
    let dimensions = route.dimensions.unwrap_or(config.embedding_dimensions);

    if model_provider.is_empty() || model.is_empty() || dimensions==0 {
        // todo  log
        return fallback();
    }

    let routed_api_key =route.api_key.as_deref()   .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);

    resolve_provider_ref(
        model_provider.to_string(),
        model.to_string(),
        dimensions,
        routed_api_key.or(configured_api_key),
        inherited_api_key,
        providers
    )



}

pub fn backend_kind_from_dotted(memory_backend: &String) -> String {
    memory_backend
        .trim()
        .split_once(".")
        .map_or(memory_backend.trim(), |(kind, _)| kind)
        .to_ascii_lowercase()
}

pub fn classify_memory_backend(backend: &str) -> MemoryBackendKind {
    match backend {
        "sqlite" => MemoryBackendKind::Sqlite,
        "none" => MemoryBackendKind::None,
        _ => MemoryBackendKind::Unknown,
    }
}

#[derive(Clone, PartialEq, Eq)]
struct ResolvedEmbeddingConfig {
    model_provider: String,
    model: String,
    dimensions: usize,
    api_key: Option<String>,
}

impl Debug for ResolvedEmbeddingConfig {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolvedEmbeddingConfig")
            .field("model_provider", &self.model_provider)
            .field("model", &self.model)
            .field("dimensions", &self.dimensions)
            .finish_non_exhaustive()
    }
}
