use shadow_core::ModelProvider;

pub struct ResolvedModelAccess<'a>{
    pub model_provider: &'a dyn ModelProvider,
    pub provider_name: &'a str,
    pub model: &'a str,
    pub temperature: Option<f64>,
}

pub struct ResolvedAgentExecution<'a> {
    pub model_access: ResolvedModelAccess<'a>,
}