use async_trait::async_trait;
use serde_json::{Value, json};
use shadow_config::policy::{SecurityPolicy, ToolOperation};
use shadow_core::{Tool, ToolResult};
use shadow_providers::{ModelProviderRuntimeOptions, ProviderDispatch};
use std::sync::Arc;

pub struct LlmTaskTool {
    security: Arc<SecurityPolicy>,
    default_model_provider: String,
    default_model: String,
    default_temperature: Option<f64>,
    api_key: Option<String>,
    provider_runtime_options: ModelProviderRuntimeOptions,
}

impl LlmTaskTool {
    pub fn new(
        security: Arc<SecurityPolicy>,
        default_model_provider: String,
        default_model: String,
        default_temperature: Option<f64>,
        api_key: Option<String>,
        provider_runtime_options: ModelProviderRuntimeOptions,
    ) -> Self {
        Self {
            security,
            default_model_provider,
            default_model,
            default_temperature,
            api_key,
            provider_runtime_options,
        }
    }
}

#[async_trait]
impl Tool for LlmTaskTool {
    fn name(&self) -> &str {
        "llm_task"
    }

    fn description(&self) -> &str {
        "Run a prompt through an LLM with no tool access and return the response. \
         Optionally validates the output against a JSON Schema. Ideal for structured \
         data extraction, classification, summarization, and transformation tasks."
    }

    fn parameters_schema(&self) -> serde_json::value::Value {
        json!({
            "type": "object",
            "properties": {
                "prompt": {
                    "type": "string",
                    "description": "The prompt to send to the LLM."
                },
                "schema": {
                    "type": "object",
                    "description": "Optional JSON Schema to validate the LLM response against. \
                                    When provided, the LLM is instructed to return valid JSON \
                                    matching this schema."
                },
                "model": {
                    "type": "string",
                    "description": "Optional model override (e.g. 'anthropic/claude-sonnet-4-6'). \
                                    Defaults to the configured default model."
                },
                "temperature": {
                    "type": "number",
                    "description": "Optional temperature override (0.0-2.0). \
                                    Defaults to the configured default temperature."
                }
            },
            "required": ["prompt"]
        })
    }

    async fn execute(&self, args: serde_json::value::Value) -> anyhow::Result<ToolResult> {
        if let Err(error) = self
            .security
            .enforce_tool_operation(ToolOperation::Act, "llm_task")
        {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(error),
            });
        }

        let prompt = match args.get("prompt").and_then(|arg| arg.as_str()) {
            Some(p) if !p.is_empty() => p,
            _ => {
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some("Missing or empty required parameter: prompt".to_string()),
                });
            }
        };

        let schema = args.get("schema").and_then(|v| v.as_object());
        let model = args
            .get("model")
            .and_then(|v| v.as_str())
            .unwrap_or(&self.default_model);
        let temperature = args
            .get("temperature")
            .and_then(|v| v.as_f64())
            .or(self.default_temperature);

        let effective_prompt = if let Some(schema_obj) = schema {
            let schema_json =
                serde_json::to_string_pretty(&serde_json::Value::Object(schema_obj.clone()))
                    .unwrap_or_else(|_| "{}".to_string());
            format!(
                "{prompt}\n\n\
                IMPORTANRT: You Must respond with valid JSON that conforms to this schema:\n
                ```json\n{schema_json\n}```\n\
                Respond Only with the JSON object, no explanation or markdown"
            )
        } else {
            prompt.to_string()
        };

        let api_key_ref = self.api_key.as_deref();
        let model_provider = match shadow_providers::create_model_provider_with_options(
            &self.default_model_provider,
            api_key_ref,
            &self.provider_runtime_options,
        ) {
            Ok(p) => p,
            Err(error) => {
                return Ok(ToolResult::err(format!("Failed to create model_provider: {error}")));
            }
        };

        let response = match ProviderDispatch::from_ref(&*model_provider)
            .simple_chat(&effective_prompt, model, temperature)
            .await
        {
            Ok(text) => text,
            Err(err) => {
                return Ok(ToolResult::err(format!("LLM call failed: {err}")));
            }
        };

        if let Some(schema_obj) = schema {
            let schema_value = serde_json::Value::Object(schema_obj.clone());
            match validate_json_response(&response, &schema_value) {
                Ok(validated_json) => Ok(ToolResult::ok(validated_json)),
                Err(err) => Ok(ToolResult::err(format!("Schema validation failed: {err}"))),
            }
        } else {
            Ok(ToolResult::ok(response))
        }
    }
}

fn validate_json_response(llm_resp: &str, expect: &Value) -> Result<String, String> {
    let trimmed = llm_resp.trim();
    let json_str = if trimmed.starts_with("```") {
        trimmed
            .trim_start_matches("```json")
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim()
    } else {
        trimmed
    };

    // Parse as JSON
    let parsed: serde_json::Value =
        serde_json::from_str(json_str).map_err(|e| format!("Invalid JSON: {e}"))?;

    // Check required fields
    if let Some(required) = expect.get("required").and_then(|v| v.as_array()) {
        for req in required {
            if let Some(field_name) = req.as_str()
                && parsed.get(field_name).is_none()
            {
                return Err(format!("Missing required field: {field_name}"));
            }
        }
    }

    // Check property types
    if let Some(properties) = expect.get("properties").and_then(|v| v.as_object()) {
        for (prop_name, prop_schema) in properties {
            if let Some(value) = parsed.get(prop_name)
                && let Some(expected_type) = prop_schema.get("type").and_then(|t| t.as_str())
                && !type_matches(value, expected_type)
            {
                return Err(format!(
                    "Field '{prop_name}' has wrong type: expected {expected_type}, \
                             got {}",
                    json_type_name(value)
                ));
            }
        }
    }

    // Return the cleaned, re-serialized JSON
    serde_json::to_string(&parsed).map_err(|e| format!("JSON serialization error: {e}"))
}

fn type_matches(value: &serde_json::Value, expected: &str) -> bool {
    match expected {
        "string" => value.is_string(),
        "number" => value.is_number(),
        "integer" => value.is_i64() || value.is_u64(),
        "boolean" => value.is_boolean(),
        "array" => value.is_array(),
        "object" => value.is_object(),
        "null" => value.is_null(),
        _ => true, // Unknown type — accept
    }
}

/// Return a human-readable type name for a JSON value.
fn json_type_name(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}
