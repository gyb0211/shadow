use crate::PluginPermission;
use anyhow::Result;
use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};
use wasmtime::component::{Component, ResourceTable};
use wasmtime::{Config, Engine, Store, StoreLimits, StoreLimitsBuilder};
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};
use wasmtime_wasi_http::WasiHttpCtx;
use wasmtime_wasi_http::p2::{WasiHttpCtxView, WasiHttpView};

#[derive(Clone, Default)]
pub struct InboundQueue {
    inner: Arc<Mutex<VecDeque<HostInboundMessage>>>,
}

#[derive(Debug, Clone, Default)]
pub struct HostInboundMessage {
    pub id: String,
    pub sender: String,
    pub reply_target: String,
    pub content: String,
    pub channel: String,
    pub channel_alias: Option<String>,
    pub timestamp: u64,
    pub thread_ts: Option<String>,
    pub interruption_scope_id: Option<String>,
    pub subject: Option<String>,
}

impl InboundQueue {
    pub fn enqueue(&self, msg: HostInboundMessage) {
        let mut q = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        q.push_back(msg);
    }

    pub fn poll(&self) -> Option<HostInboundMessage> {
        let mut q = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        q.pop_front()
    }

    pub fn pending(&self) -> u32 {
        let mut q = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        q.len() as u32
    }
}
#[derive(Debug, Clone, Copy)]
pub struct PluginLimits {
    pub call_fuel: u64,
    pub max_memory_bytes: usize,
    pub max_table_elements: usize,
    pub max_instances: usize,
}

pub struct PluginState {
    wasi: WasiCtx,
    table: ResourceTable,
    http: Option<WasiHttpCtx>,
    inbound: InboundQueue,
    limits: StoreLimits,
    fuel_per_call: u64,
}

impl PluginState {
    pub fn new(permissions: &[PluginPermission], limits: PluginLimits) -> Self {
        Self::with_inbound(permissions, InboundQueue::default(), limits)
    }

    pub fn with_inbound(
        permissions: &[PluginPermission],
        inbound: InboundQueue,
        limits: PluginLimits,
    ) -> Self {
        let http = permissions
            .contains(&PluginPermission::HttpClient)
            .then(WasiHttpCtx::new);
        Self {
            wasi: WasiCtx::builder().build(),
            table: ResourceTable::new(),
            http,
            inbound,
            limits: StoreLimitsBuilder::new()
                .memory_size(limits.max_memory_bytes)
                .table_elements(limits.max_table_elements)
                .instances(limits.max_instances)
                .build(),
            fuel_per_call: limits.call_fuel,
        }
    }

    pub fn http_enabled(&self) -> bool {
        self.http.is_some()
    }

    pub fn inbound(&self) -> &InboundQueue {
        &self.inbound
    }
}

impl WasiView for PluginState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

impl WasiHttpView for PluginState {
    fn http(&mut self) -> WasiHttpCtxView<'_> {
        let ctx = self
            .http
            .as_mut()
            .expect("wasi:http called on a plugin without HttpClient permission");

        WasiHttpCtxView {
            ctx,
            table: &mut self.table,
            hooks: wasmtime_wasi_http::p2::default_hooks(),
        }
    }
}

pub fn add_wasi(linker: &mut wasmtime::component::Linker<PluginState>) -> Result<()> {
    wt(
        wasmtime_wasi::p2::add_to_linker_async(linker),
        "failed to add WASI imports to plugin linker",
    )
}
pub fn add_http_wasi(linker: &mut wasmtime::component::Linker<PluginState>) -> Result<()> {
    wt(
        wasmtime_wasi_http::p2::add_only_http_to_linker_async(linker),
        "failed to add WASI:http imports to plugin linker",
    )
}

pub fn ensure_http_coherent(store: &Store<PluginState>, linker_has_http: bool) -> Result<()> {
    let store_has_http = store.data().http_enabled();
    if store_has_http != linker_has_http {
        anyhow::bail!(
            "Plugin store/linker http mismatch: store HttpClient={store_has_http}, \
            linker wasi:http={linker_has_http}; refusing to instantiate"
        );
    }
    Ok(())
}

pub fn engine() -> &'static Engine {
    static ENGINE: OnceLock<Engine> = OnceLock::new();
    ENGINE.get_or_init(|| {
        let mut config = Config::new();
        config.consume_fuel(true);
        Engine::new(&config).expect("async-capable wasmtime engine")
    })
}

pub fn new_store(permissions: &[PluginPermission], limits: PluginLimits) -> Store<PluginState> {
    new_store_with_inbound(permissions, InboundQueue::default(), limits)
}

pub fn new_store_with_inbound(
    permissions: &[PluginPermission],
    inbound: InboundQueue,
    limits: PluginLimits,
) -> Store<PluginState> {
    let state = PluginState::with_inbound(permissions, inbound, limits);
    let mut store = Store::new(engine(), state);
    store.limiter(|state| &mut state.limits);
    set_call_fuel(&mut store, limits.call_fuel);
    store
}

fn set_call_fuel(store: &mut Store<PluginState>, call_fuel: u64) {
    store
        .set_fuel(call_fuel)
        .expect("fuel is enabled on plugin engine");
}

pub fn refuel(store: &mut Store<PluginState>) {
    let call_fuel = store.data().fuel_per_call;
    set_call_fuel(store, call_fuel);
}

pub fn wt<T>(r: wasmtime::Result<T>, ctx: &'static str) -> Result<T> {
    r.map_err(|e| anyhow::Error::msg(format!("{ctx}: {e}")))
}

pub fn load_component(wasm_path: &Path) -> Result<Component> {
    wt(load_inner(wasm_path), "failed to load WASM component")
}

// fn load_inner(wasm_path: &Path) -> wasmtime::Result<Component> {
//     Component::from_file(engine(), wasm_path)
// }
//
//
fn load_inner(wasm_path: &Path) -> wasmtime::Result<Component> {
    unsafe { Component::deserialize_file(engine(), wasm_path) }
}

macro_rules! call_plugin {
    ($self: expr, $body: expr) => {
        {
            let mut guard = $self.state.lock();
            let (ref mut store, ref mut bindings) = *guard;
            crate::component::refuel(store);
            let f = $body,
            f(store, bindings).await
        }
    };
}

pub(crate) use call_plugin;

pub mod bindings {
    pub mod tool {
        wasmtime::component::bindgen!({
            world: "tool-plugin",
            path: "../../wit/v0",
            imports: {default: async},
            exports: {default: async},
        });
    }

    pub mod channel {
        wasmtime::component::bindgen!({
            world: "channel-plugin",
            path: "../../wit/v0",
            imports: {default: async},
            exports: {default: async},
        });
    }

    pub mod memory {
        wasmtime::component::bindgen!({
            world: "memory-plugin",
            path: "../../wit/v0",
            imports: {default: async},
            exports: {default: async},
        });
    }
}
