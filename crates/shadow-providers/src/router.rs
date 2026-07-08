// //! Router -- 按 alias 路由到具体 family provider + 跨 provider fallback
// //!
// //! 3 层架构的顶层。Agent 只面对 `dyn ModelProvider`, 不感知后端是 OpenAiProvider
// //! 还是未来的 AnthropicNative。Router 接收请求, 按 model 字段中的 hint (MVP 阶段)
// //! 或 default 转发到注册的 inner provider.
// //!
// //! 行为:
// //! - model = "hint:reasoning" → 查 routes 表, 路由到指定 provider + 替换 model 名
// //! - model = "gpt-4o" → default provider, model 名原样透传
// //! - 未注册 hint 调用返回 default provider
// //! - 主 provider chat/chat_stream 返回 Err → 按 fallback_chains 依次尝试备选 provider
// //!
// //! 关键: Router 看到的 inner 通常是 Reliable-wrapped, 内部已耗尽重试才到这.
// //! Router 不分类错误, 任何 Err 都触发 fallback (除非 chain 也耗尽).
// 
// use crate::dispatch::ProviderDispatch;
// use anyhow::Result;
// use async_trait::async_trait;
// use futures::stream::BoxStream;
// use shadow_core::{Attributable, ChatRequest, ChatResponse, ModelProvider, Role};
// use std::collections::HashMap;
// use tracing::{debug, info, warn};
// 
// #[derive(Debug, Clone)]
// pub struct Route {
//     pub provider_name: String, // 要跟 model_providers 里的 name 对上
//     pub model: String,         // 实际下发给该 provider 的 model 字符串
// }
// 
// /// 路由器 -- 按 model hint 路由 Provider 调用 + 跨 provider fallback
// pub struct RouterModelProvider {
//     alias: String,
//     /// 例: "reasoning" → (2, "claude-opus-4")
//     routes: HashMap<String, (usize, String)>,
// 
//     model_providers: Vec<(String, Box<dyn ModelProvider>)>,
// 
//     default_index: usize,
// 
//     /// hint (或 "default") → 备选 provider index 列表
//     ///
//     /// 当主 provider 失败时, 按 chain 顺序依次尝试. chain 中的 provider 用各自
//     /// 默认 model (不再做 hint→model 替换).
//     fallback_chains: HashMap<String, Vec<usize>>,
// 
//     /// 保留默认模型名 (chat_via_router 用)
//     #[allow(dead_code)]
//     default_model: String,
// }
// 
// impl RouterModelProvider {
//     /// 构造器 -- 指定默认 alias (后续 register 必须包含此 alias)
//     #[must_use]
//     pub fn new(
//         default_alias: impl Into<String>,
//         model_providers: Vec<(String, Box<dyn ModelProvider>)>,
//         routes: Vec<(String, Route)>,
//         default_model: String,
//     ) -> Self {
//         Self::with_fallback_chains(
//             default_alias,
//             model_providers,
//             routes,
//             default_model,
//             Vec::new(),
//         )
//     }
// 
//     /// 完整构造器 (含 fallback_chains)
//     ///
//     /// `fallback_chains: Vec<(hint_or_"default", Vec<provider_name>)>` --
//     /// 主 provider 失败时按此顺序尝试备选. 未指定的 hint 回退到 "default" chain.
//     #[must_use]
//     pub fn with_fallback_chains(
//         default_alias: impl Into<String>,
//         model_providers: Vec<(String, Box<dyn ModelProvider>)>,
//         routes: Vec<(String, Route)>,
//         default_model: String,
//         fallback_chains: Vec<(String, Vec<String>)>,
//     ) -> Self {
//         let name_to_index: HashMap<&str, usize> = model_providers
//             .iter()
//             .enumerate()
//             .map(|(i, (name, _))| (name.as_str(), i))
//             .collect();
// 
//         let resolve_routes = routes
//             .into_iter()
//             .filter_map(|(hint, route)| {
//                 name_to_index
//                     .get(route.provider_name.as_str())
//                     .copied()
//                     .map(|index| (hint, (index, route.model)))
//             })
//             .collect();
// 
//         let resolve_chains = fallback_chains
//             .into_iter()
//             .filter_map(|(key, names)| {
//                 let idxs: Vec<usize> = names
//                     .iter()
//                     .filter_map(|n| name_to_index.get(n.as_str()).copied())
//                     .collect();
//                 if idxs.is_empty() {
//                     None
//                 } else {
//                     Some((key, idxs))
//                 }
//             })
//             .collect();
// 
//         Self {
//             alias: default_alias.into(),
//             routes: resolve_routes,
//             model_providers,
//             default_index: 0,
//             fallback_chains: resolve_chains,
//             default_model,
//         }
//     }
// 
//     // 协议约定:
//     //
//     //     - model = "hint:reasoning" → 查 routes 表
//     //     - model = "gpt-4o" → default provider, model 名原样透传下去
//     fn resolve(&self, model: &str) -> (usize, String) {
//         if let Some(hint) = model.strip_prefix("hint:")
//             && let Some((idx, resolve_model)) = self.routes.get(hint)
//         {
//             return (*idx, resolve_model.clone());
//         }
// 
//         (self.default_index, model.to_string())
//     }
// 
//     /// 取 hint 的 fallback chain. 优先 hint 专属 chain, 否则回退到 "default".
//     fn fallback_chain_for(&self, model: &str) -> Option<&[usize]> {
//         let hint = model.strip_prefix("hint:").unwrap_or("default");
//         self.fallback_chains
//             .get(hint)
//             .or_else(|| self.fallback_chains.get("default"))
//             .map(Vec::as_slice)
//     }
// 
//     /// 借用默认 provider (用于非路由方法: provider_type / list_models / ...)
//     fn default_provider(&self) -> &dyn ModelProvider {
//         let (_, provider) = &self.model_providers[self.default_index];
//         &**provider
//     }
// }
// 
// impl Attributable for RouterModelProvider {
//     fn role(&self) -> Role {
//         Role::Provider
//     }
//     fn alias(&self) -> &str {
//         &self.alias
//     }
// }
// 
// #[async_trait]
// impl ModelProvider for RouterModelProvider {
//     async fn chat_with_system(
//         &self,
//         system_prompt: Option<&str>,
//         message: &str,
//         model: &str,
//         temperature: Option<f64>,
//     ) -> Result<String> {
//         let (provider_idx, resolved_model) = self.resolve(model);
//         let (_, provider) = &self.model_providers[provider_idx];
//         let mut messages = Vec::new();
//         if let Some(sys) = system_prompt {
//             messages.push(shadow_core::ChatMessage::system(sys));
//         }
//         messages.push(shadow_core::ChatMessage { role: "user".into(), content: message.into() });
//         provider.chat_with_history(&messages, &resolved_model, temperature).await
//     }
// 
//     async fn list_models(&self) -> Result<Vec<String>> {
//         self.default_provider().list_models().await
//     }
// 
//     fn supports_native_tools(&self) -> bool {
//         self.default_provider().supports_native_tools()
//     }
// 
//     fn default_temperature(&self) -> f64 {
//         self.default_provider().default_temperature()
//     }
// }
