use shadow_config::RiskProfileConfig;

pub fn build_system_prompt_with_mode_and_autonomy(
    workspace_dir: &std::path::Path,
    model_name: &str,
    tools: &[(&str, &str)],
    autonomy_config: Option<&RiskProfileConfig>,
    show_tool_calls: bool,
)-> String{
    // use std::fmt::Write;
    // let mut prompt = String::with_capacity(8192);
    "You are Shadow, a fast and efficient AI assistant. Be helpful, concise, and direct.".to_string()

}