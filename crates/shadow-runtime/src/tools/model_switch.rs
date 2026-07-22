use crate::agent::get_model_switch_state;
use async_trait::async_trait;
use serde_json::{Value, json};
use shadow_config::Config;
use shadow_config::policy::{SecurityPolicy, ToolOperation};
use shadow_core::{Tool, ToolResult};
use std::sync::{Arc, LazyLock, Mutex};

#[cfg(test)]
type ModelCatalogResolver = Arc<
    dyn Fn(String) -> std::pin::Pin<Box<dyn Future<Output=anyhow::Result<Vec<String>>> + Send>>
    + Send
    + Sync,
>;

pub struct ModelSwitchTool {
    security: Arc<SecurityPolicy>,
    config: Arc<Config>,
    #[cfg(test)]
    catalog_resolver: Option<ModelCatalogResolver>,
}

impl ModelSwitchTool {
    pub const NAME: &'static str = "model_switch";
    pub fn new(security: Arc<SecurityPolicy>, config: Arc<Config>) -> Self {
        Self {
            security,
            config,
            #[cfg(test)]
            catalog_resolver: None,
        }
    }
}

#[async_trait]
impl Tool for ModelSwitchTool {
    fn name(&self) -> &str {
        Self::NAME
    }

    fn description(&self) -> &str {
        "Request a runtime model switch using a configured provider profile plus provider-local model.\
        Use \
        'get' to see the pending switch, \
        'list_model_providers' to see provider families,\
        'list_models' to see common models for a provider profile,\
        or 'set' with a dotted provider profile ref such as 'openai.default'.\
        The switch is runtime/session state and does not write config."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["get", "set", "list_model_providers", "list_models"],
                    "description": "Action to perform: get pending switch state, set a runtime provider-profile/model switch, list available provider families, or list common models for a provider profile"
                },
                "model_provider": {
                    "type": "string",
                    "description": "Dotted provider profile reference (e.g., 'openai.default', 'anthropic.sonnet', 'ollama.local'). Required for 'set' and 'list_models' actions."
                },
                "model": {
                    "type": "string",
                    "description": "Model ID (e.g., 'gpt-4o', 'claude-sonnet-4-6'). Required for 'set' action."
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let action = args
            .get("action")
            .and_then(|arg| arg.as_str())
            .unwrap_or("get");
        if let Err(error) = self
            .security
            .enforce_tool_operation(ToolOperation::Act, "model_switch")
        {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(error),
            });
        }

        match action {
            "get" => self.handle_get(),
            "set" => self.handle_set(&args),
            "list_model_providers" => self.handle_list_providers(),
            "list_models" => self.handle_list_models(&args).await,
            _ => Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!(
                    "Unknown action: {}. Valid actions: get, set, list_model_providers, list_models",
                    action
                )),
            }),
        }
    }
}

impl ModelSwitchTool {
    fn handle_get(&self) -> anyhow::Result<ToolResult> {
        let switch_state = get_model_switch_state();
        let pending = switch_state.lock().unwrap().clone();
        Ok(ToolResult {
            success: true,
            output: serde_json::to_string_pretty(&json!(
               {
                   "pending_switch":pending,
                    "note": "To switch models, use action 'set' with dotted <type>.<alias> model_provider and model parameters"
               }
            ))?,
            error: None,
        })
    }
    fn handle_set(&self, args: &Value) -> anyhow::Result<ToolResult> {
        let model_provider = args.get("model_provider").and_then(|mp| mp.as_str());
        let model_provider = match model_provider {
            Some(p) => p,
            None => {
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some("Missing 'model_provider' parameter for 'set' action".to_string()),
                });
            }
        };

        let model = args.get("model").and_then(|mp| mp.as_str());
        let model = match model {
            Some(m) if !m.trim().is_empty() => m.trim(),
            Some(m) => {
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some("Model Id cannot be empty".to_string()),
                });
            }
            None => {
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some("Missing 'model' parameter for 'set' action".to_string()),
                });
            }
        };

        let model_provider = match resolve_model_provider_profile_ref(&self.config, model_provider)
        {
            Ok(mp) => mp,
            Err(err) => {
                let known_model_provider = shadow_providers::list_model_providers();
                let configured_profiles = configured_model_provider_profiles(&self.config);
                return Ok(ToolResult {
                    success: false,
                    output: serde_json::to_string_pretty(&json!({

                        "provider_ref_shape": "<type>.<alias>",
                        "available_provider_families": known_model_provider.iter().map(|p| p.name).collect::<Vec<_>>(),
                        "configured_provider_profiles": configured_profiles,

                    }))?,
                    error: Some(err),
                });
            }
        };
        let switch_state = get_model_switch_state();
        *switch_state.lock().unwrap() = Some((model_provider.clone(), model.to_string()));
        Ok(ToolResult {
            success: true,
            output: serde_json::to_string_pretty(&json!({
                "message": "Model switch reuqested",
                "model_provider":model_provider,
                "model":model,
                "note": "The active runtime path will consume this provider-profile/model switch where model_switch is supported. This does not write persisted config."
            }))?,
            error: None,
        })
    }
    fn handle_list_providers(&self) -> anyhow::Result<ToolResult> {
        let providers_list = shadow_providers::list_model_providers();
        let configured_profiles = configured_model_provider_profiles(&self.config);
        let configured_count = configured_profiles.len();
        let model_providers: Vec<serde_json::Value> = providers_list
            .iter()
            .map(|m| {
                json!({
                    "name": m.name,
                    "display_name": m.display_name,
                    "local": m.local
                })
            })
            .collect();

        Ok(ToolResult {
            success: true,
            output: serde_json::to_string_pretty(&json!({
                "model_providers": model_providers,
                "count": model_providers.len(),
                "configured_provider_profiles": configured_profiles,
                "configured_count": configured_count,
                "provider_ref_shape":"<type>.<alias>",
                "example": "Use action 'set' with dotted provider profile ref such as 'openai.default'"
            }))?,
            error: None,
        })
    }
    async fn handle_list_models(&self, args: &Value) -> anyhow::Result<ToolResult> {
        let model_provider = args.get("model_provider").and_then(|mp| mp.as_str());
        let model_provider = match model_provider {
            Some(p) => p,
            None => {
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some(
                        "Missing 'model_provider' parameter for 'list_models' action".to_string(),
                    ),
                });
            }
        };

        let model_provider = match resolve_model_provider_profile_ref(&self.config, model_provider)
        {
            Ok(mp) => mp,
            Err(err) => {
                let configured_profiles = configured_model_provider_profiles(&self.config);
                return Ok(ToolResult {
                    success: false,
                    output: serde_json::to_string_pretty(&json!({
                        "provider_ref_shape": "<type>.<alias>",
                        "configured_provider_profiles": configured_profiles,

                    }))?,
                    error: Some(err),
                });
            }
        };

        let provider_family = model_provider
            .split_once('.')
            .map(|(family, _alias)| family)
            .unwrap_or(model_provider.as_str())
            .to_lowercase();

        let models: Vec<String> = match self.resolve_catalog(&provider_family).await {
            Ok(live) if !live.is_empty() => live,
            Ok(_) => hardcoded_models_for(&provider_family),
            Err(error) => {
                shadow_log::record!(
                    WARN,
                    shadow_log::Event::new(module_path!(), shadow_log::Action::Reject)
                        .with_outcome(shadow_log::EventOutcome::Failure)
                        .with_attrs(serde_json::json!({
                            "model_provider": model_provider,
                            "provider_family":provider_family,
                            "error": error.to_string(),
                        })),
                    "model_switch list_models: live catalog unavailable, using hardcoded fallback"
                );
                hardcoded_models_for(&provider_family)
            }
        };

        if models.is_empty() {
            return Ok(ToolResult {
                success: true,
                output: serde_json::to_string_pretty(&json!({
                    "model_provider": model_provider,
                    "models": [],
                    "note": "No common models listed for this model_provider family. Check model_provider documentation for available models."
                }))?,
                error: None,
            });
        }

        Ok(ToolResult {
            success: true,
            output: serde_json::to_string_pretty(&json!({
                "model_provider": model_provider,
                "models": models,
                "example": "Use action 'set' with this model_provider and a model id to switch"
            }))?,
            error: None,
        })
    }


    async fn resolve_catalog(&self, family: &str) -> anyhow::Result<Vec<String>>{
        #[cfg(test)]
        if let Some(resolver) = &self.catalog_resolver {
            return resolver(family.to_string()).await;
        }
        shadow_providers::catalog::list_models_for_family(family).await

    }
}

fn configured_model_provider_profiles(config: &Config) -> Vec<String> {
    let mut profiles = config
        .providers
        .models
        .iter_entries()
        .map(|(family, alias, _profile)| format!("{family}.{alias}"))
        .collect::<Vec<_>>();
    profiles.sort();
    profiles
}

fn resolve_model_provider_profile_ref(config: &Config, raw: &str) -> Result<String, String> {
    let raw = raw.trim();
    let Some((family, alias)) = raw.split_once('.') else {
        return Err(format!(
            "model_provider must be a dotted '<type>.<alias> provider profile reference, get `{raw}`"
        ));
    };
    let family = family.trim();
    let alias = alias.trim();
    if family.is_empty() || alias.is_empty() {
        return Err(format!(
            "model_provider must be a dotted '<type>.<alias> provider profile reference, get `{raw}`"
        ))
    }

    if config.providers.models.find(family, alias).is_none(){
        let available = configured_model_provider_profiles(config);
        let available = if available.is_empty(){
            "no configured provider profiles".to_string()
        }else{
            available.join(", ")
        };

        return Err(format!(
            "model_provider `{raw} is not a configured provider profile. Add a [providers.models.{family}.{alias}] entry or use one of:{available} `"
        ))
    }

    Ok(format!("{family}.{alias}"))


}


fn hardcoded_models_for(provider_family: &str) -> Vec<String> {
    let models: Vec<&'static str> = match provider_family {
        _ => vec![]
    };
    models.into_iter().map(String::from).collect()
}
