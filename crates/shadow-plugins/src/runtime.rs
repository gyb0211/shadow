use crate::PluginPermission;
use crate::component::bindings::tool::ToolPlugin;
use crate::component::bindings::tool::exports::shadow::plugin::tool::ToolResult as WitToolResult;
use crate::component::{PluginLimits, PluginState, call_plugin, engine, load_component, wt};
use anyhow::Context;
use serde_json;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};
use wasmtime::Store;
use wasmtime::component::Linker;

use shadow_core::ToolResult;
#[derive(Debug)]
pub struct ToolMetadata {
    pub name: String,
    pub description: String,
    pub parameters_schema: serde_json::Value,
}

pub struct Plugin {
    state: Arc<Mutex<(Store<PluginState>, ToolPlugin)>>,
}

fn base_linker() -> anyhow::Result<Linker<PluginState>> {
    let mut linker = Linker::new(engine());
    crate::component::add_wasi(&mut linker);
    let mut options = crate::component::bindings::tool::LinkOptions::default();
    options.plugins_wit_v0(true);
    wt(
        ToolPlugin::add_to_linker::<_, wasmtime::component::HasSelf<_>>(
            &mut linker,
            &options,
            |s| s,
        ),
        "failed to add tool plugin imports to linker",
    )?;
    Ok(linker)
}
fn tool_linker() -> &'static Linker<PluginState> {
    static LINKER: OnceLock<Linker<PluginState>> = OnceLock::new();
    LINKER.get_or_init(|| base_linker().expect("tool linker"))
}

fn tool_linker_http() -> &'static Linker<PluginState> {
    static LINKER: OnceLock<Linker<PluginState>> = OnceLock::new();
    LINKER.get_or_init(|| {
        let mut linker = base_linker().expect("tool linker");
        crate::component::add_http_wasi(&mut linker).expect("tool http linker");
        linker
    })
}

pub async fn crate_plugin(
    wasm_path: &Path,
    permissions: &[PluginPermission],
    limits: PluginLimits,
) -> anyhow::Result<Plugin> {
    let component = load_component(wasm_path)?;
    let mut store = crate::component::new_store(permissions, limits);
    let http = store.data().http_enabled();
    let linker = if http {
        tool_linker_http()
    } else {
        tool_linker()
    };

    crate::component::ensure_http_coherent(&store, http)?;
    let bindings = wt(
        ToolPlugin::instantiate_async(&mut store, &component, linker).await,
        "failed to instantiate tool plugin",
    )?;

    Ok(Plugin {
        state: Arc::new(Mutex::new((store, bindings))),
    })
}

pub async fn call_tool_metadata(plugin: &mut Plugin) -> anyhow::Result<ToolMetadata> {
    call_plugin!(
        plugin,
        async move |store: &mut Store<PluginState>, bindings: &mut ToolPlugin| {
            let tool = bindings.shadow_plugin_tool();
            let name = wt(tool.call_name(&mut *store).await, "tool.name failed")?;
            let description = wt(
                tool.call_description(&mut *store).await,
                "tool.description failed",
            )?;
            let schema_json = wt(
                tool.call_parameters_schema(&mut *store).await,
                "tool.parameters-schema failed",
            )?;

            let parameters_schema = serde_json::from_str(&schema_json)
                .context("tool parameters-schema is not valid JSON")?;

            Ok(ToolMetadata {
                name,
                description,
                parameters_schema,
            })
        }
    )
}

pub async fn call_execute(
    plugin: &mut Plugin,
    args_json: &[u8],
    config: &HashMap<String, String>,
    permissions: &[PluginPermission],
) -> anyhow::Result<ToolResult> {
    let input = inject_config(args_json, effective_config(config, permissions))?;
    call_plugin!(
        plugin,
        async move |store: &mut Store<PluginState>, bindings: &mut ToolPlugin| {
            let result = wt(
                bindings
                    .shadow_plugin_tool()
                    .call_execute(store, &input)
                    .await,
                "tool.execute trapped",
            )?
            .map_err(|e| anyhow::Error::msg(format!("plugin execute returned error: {e}")))?;
            Ok(into_tool_result(result))
        }
    )
}

fn into_tool_result(result: WitToolResult) -> ToolResult {
    ToolResult {
        success: result.success,
        output: result.output,
        error: result.error,
    }
}

fn inject_config(args_json: &[u8], config: &HashMap<String, String>) -> anyhow::Result<String> {
    let mut args: serde_json::Value =
        serde_json::from_slice(args_json).expect("plugin args are not valid JSON ");

    let obj = args
        .as_object_mut()
        .context("plugin args must be a JSON object")?;

    obj.remove("__config");
    if !obj.is_empty() {
        obj.insert(
            "__config".to_string(),
            serde_json::to_value(config).context("failed to serialize plugin config")?,
        );
    }
    serde_json::to_string(&args).context("failed to serialize plugin input")
}

fn effective_config<'a>(
    config: &'a HashMap<String, String>,
    permissions: &[PluginPermission],
) -> &'a HashMap<String, String> {
    static EMPTY: OnceLock<HashMap<String, String>> = OnceLock::new();
    if permissions.contains(&PluginPermission::ConfigRead) {
        config
    } else {
        EMPTY.get_or_init(HashMap::new)
    }
}
