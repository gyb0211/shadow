use reqwest::Client;
use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

pub fn build_runtime_proxy_client_with_timeouts(
    service_key: &str,
    timeout: u64,
    conn_timeout_secs: u64,
) -> Client {
    Client::new()
}

pub fn build_channel_proxy_client(service_key: &str, proxy_url: Option<&str>) -> reqwest::Client {
    match normalize_proxy_url_option(proxy_url) {
        Some(url) => build_explicit_proxy_client(service_key, &url, None, None),
        None => build_runtime_proxy_client(service_key),
    }
}


fn runtime_proxy_cache_key(
    service_key: &str,
    timeout_secs: Option<u64>,
    connect_timeout_secs: Option<u64>,
) -> String {
    format!(
        "{}|timeout={}|connect_timeout={}",
        service_key.trim().to_ascii_lowercase(),
        timeout_secs
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_string()),
        connect_timeout_secs
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_string())
    )
}

pub fn apply_runtime_proxy_to_builder(
    builder: reqwest::ClientBuilder,
    service_key: &str,
) -> reqwest::ClientBuilder {
    builder
    // runtime_proxy_config().apply_to_reqwest_builder(builder, service_key)
}

// pub fn runtime_proxy_config() -> ProxyConfig {
//     match runtime_proxy_state().read() {
//         Ok(guard) => guard.clone(),
//         Err(poisoned) => poisoned.into_inner().clone(),
//     }
// }

pub fn build_runtime_proxy_client(service_key: &str) -> reqwest::Client {
    let cache_key = runtime_proxy_cache_key(service_key, None, None);
    if let Some(client) = runtime_proxy_cached_client(&cache_key) {
        return client;
    }

    let builder = apply_runtime_proxy_to_builder(reqwest::Client::builder(), service_key);
    let client = builder.build().unwrap_or_else(|error| {
        ::shadow_log::record!(
            WARN,
            ::shadow_log::Event::new(module_path!(), ::shadow_log::Action::Note)
                .with_outcome(::shadow_log::EventOutcome::Unknown)
                .with_attrs(
                    ::serde_json::json!({"service_key": service_key, "error": format!("{}", error)})
                ),
            "Failed to build proxied client: "
        );
        reqwest::Client::new()
    });
    set_runtime_proxy_cached_client(cache_key, client.clone());
    client
}

fn normalize_proxy_url_option(raw: Option<&str>) -> Option<String> {
    let value = raw?.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn build_explicit_proxy_client(
    service_key: &str,
    proxy_url: &str,
    timeout_secs: Option<u64>,
    connect_timeout_secs: Option<u64>,
) -> reqwest::Client {
    let cache_key = format!(
        "explicit|{}|{}|timeout={}|connect_timeout={}",
        service_key.trim().to_ascii_lowercase(),
        proxy_url,
        timeout_secs
            .map(|v| v.to_string())
            .unwrap_or_else(|| "none".to_string()),
        connect_timeout_secs
            .map(|v| v.to_string())
            .unwrap_or_else(|| "none".to_string()),
    );
    if let Some(client) = runtime_proxy_cached_client(&cache_key) {
        return client;
    }

    let mut builder = reqwest::Client::builder();
    if let Some(t) = timeout_secs {
        builder = builder.timeout(std::time::Duration::from_secs(t));
    }
    if let Some(ct) = connect_timeout_secs {
        builder = builder.connect_timeout(std::time::Duration::from_secs(ct));
    }
    builder = apply_explicit_proxy_to_builder(builder, service_key, proxy_url);
    let client = builder.build().unwrap_or_else(|error| {
        ::shadow_log::record!(WARN, ::shadow_log::Event::new(module_path!(), ::shadow_log::Action::Note).with_outcome(::shadow_log::EventOutcome::Unknown).with_attrs(::serde_json::json!({"service_key": service_key, "proxy_url": proxy_url, "error": format!("{}", error)})), "Failed to build channel proxy client: ");
        reqwest::Client::new()
    });
    set_runtime_proxy_cached_client(cache_key, client.clone());
    client
}

fn apply_explicit_proxy_to_builder(
    mut builder: reqwest::ClientBuilder,
    service_key: &str,
    proxy_url: &str,
) -> reqwest::ClientBuilder {
    match reqwest::Proxy::all(proxy_url) {
        Ok(proxy) => {
            builder = builder.proxy(proxy);
        }
        Err(error) => {
            ::shadow_log::record!(WARN, ::shadow_log::Event::new(module_path!(), ::shadow_log::Action::Note).with_outcome(::shadow_log::EventOutcome::Unknown).with_attrs(::serde_json::json!({"proxy_url": proxy_url, "service_key": service_key, "error": format!("{}", error)})), "Ignoring invalid channel proxy_url: ");
        }
    }
    builder
}

fn runtime_proxy_cached_client(cache_key: &str) -> Option<reqwest::Client> {
    match runtime_proxy_client_cache().read() {
        Ok(guard) => guard.get(cache_key).cloned(),
        Err(poisoned) => poisoned.into_inner().get(cache_key).cloned(),
    }
}

fn set_runtime_proxy_cached_client(cache_key: String, client: reqwest::Client) {
    match runtime_proxy_client_cache().write() {
        Ok(mut guard) => {
            guard.insert(cache_key, client);
        }
        Err(poisoned) => {
            poisoned.into_inner().insert(cache_key, client);
        }
    }
}
static RUNTIME_PROXY_CLIENT_CACHE: OnceLock<RwLock<HashMap<String, reqwest::Client>>> =
    OnceLock::new();
fn runtime_proxy_client_cache() -> &'static RwLock<HashMap<String, reqwest::Client>> {
    RUNTIME_PROXY_CLIENT_CACHE.get_or_init(|| RwLock::new(HashMap::new()))
}

fn clear_runtime_proxy_client_cache() {
    match runtime_proxy_client_cache().write() {
        Ok(mut guard) => {
            guard.clear();
        }
        Err(poisoned) => {
            poisoned.into_inner().clear();
        }
    }
}
