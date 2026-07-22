
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use serde::Deserialize;
use tokio::sync::OnceCell;

const CATALOG_URL: &str = "https://models.dev/api.json";

#[derive(Debug, Deserialize)]
pub(crate) struct ProviderEntry{
    models: HashMap<String, ModelEntry>,
}

#[derive(Debug, Deserialize)]
struct ModelEntry{
    id: String,
    #[serde(default)]
    cost: Option<ModelCost>,
}

#[derive(Debug, Deserialize, Clone, Copy, Default)]
struct ModelCost{
    #[serde(default)]
    input: Option<f64>,
    #[serde(default)]
    output: Option<f64>,
    #[serde(default)]
    cache_read: Option<f64>,

}

pub(crate) type Catalog = HashMap<String, ProviderEntry>;

static CACHED_CATALOG: OnceCell<Arc<Catalog>> = OnceCell::const_new();

pub async fn list_models_for(provider_key: &str) -> anyhow::Result<Vec<String>> {
    shadow_log::scope!(
        model_provider_type: "models_dev",
        model_provider_alias: "catalog",
        => async move {
           let catalog = CACHED_CATALOG.get_or_try_init(fetch_catalog).await?;
            filter_models(catalog, provider_key)
        }
    )
    .await
}
pub(crate) async fn fetch_catalog() -> anyhow::Result<Arc<Catalog>>{
    let client = reqwest::Client::builder().timeout(Duration::from_secs(10))
        .build()?;
    let response  =client.get(CATALOG_URL).send().await?.error_for_status()?;
    let bytes = response.bytes().await?;
    Ok(Arc::new(parse_catalog(&bytes)?))
}

fn parse_catalog(bytes: &[u8]) -> anyhow::Result<Catalog> {
    Ok(serde_json::from_slice(bytes)?)
}

pub(crate) fn filter_models(catalog: &Catalog, provider_key: &str) -> anyhow::Result<Vec<String>> {
    let entry = catalog.get(provider_key).ok_or_else(|| {
        shadow_log::record!(
            WARN,
            shadow_log::Event::new(module_path!(), shadow_log::Action::Reject)
                .with_outcome(shadow_log::EventOutcome::Failure)
                .with_attrs(serde_json::json!({"model_provider":provider_key})),
            "models_dev: provider not in catalog"
        );
        anyhow::Error::msg(format!(
            "model_provider {provider_key:?} is not in the models.dev catalog",
        ))
    })?;

    let mut ids: Vec<String> = entry
        .models
        .values()
        .map(|entry| entry.id.clone())
        .collect();
    ids.sort();
    ids.dedup();
    Ok(ids)
}
