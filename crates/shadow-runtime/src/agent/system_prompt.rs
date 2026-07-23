use shadow_config::RiskProfileConfig;

pub fn build_system_prompt_with_mode_and_autonomy(
    workspace_dir: &std::path::Path,
    model_name: &str,
    tools: &[(&str, &str)],
    autonomy_config: Option<&RiskProfileConfig>,
    show_tool_calls: bool,
)-> String{
    
}