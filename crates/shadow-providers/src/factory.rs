use crate::OpenAiCompatibleModelProvider;
use shadow_config::{Config, CustomModelProviderConfig, for_each_model_provider_slot, ModelProviderConfig};
use shadow_core::{AuthStyle, ModelProvider, ModelProviderRuntimeOptions};

pub trait FamilyProviderFactory {
    fn create_provider(
        &self,
        alias: &str,
        key: Option<&str>,
        api_url: Option<&str>,
        options: &ModelProviderRuntimeOptions,
    ) -> anyhow::Result<Box<dyn ModelProvider>>;
}

impl FamilyProviderFactory for CustomModelProviderConfig {
    fn create_provider(
        &self,
        alias: &str,
        key: Option<&str>,
        api_url: Option<&str>,
        options: &ModelProviderRuntimeOptions,
    ) -> anyhow::Result<Box<dyn ModelProvider>> {
        let base_url =
            api_url.ok_or_else(|| anyhow::Error::msg("Custom model_provider required `url`"))?;

        let mut p = crate::openai::OpenAiCompatibleModelProvider::new_with_vision(
            alias,
            "Custom",
            base_url,
            key,
            AuthStyle::Bearer,
            true,
        );

        if options.native_tools != Some(true) {
            p = p.without_native_tools();
        }

        Ok(apply_compat_options(p, options))
    }
}

impl FamilyProviderFactory for ModelProviderConfig {
    fn create_provider(
        &self,
        alias: &str,
        key: Option<&str>,
        api_url: Option<&str>,
        options: &ModelProviderRuntimeOptions,
    ) -> anyhow::Result<Box<dyn ModelProvider>> {
        let base_url =
            api_url.ok_or_else(|| anyhow::Error::msg("Custom model_provider required `url`"))?;

        let mut p = crate::openai::OpenAiCompatibleModelProvider::new_with_vision(
            alias,
            "OpenAI Compatible",
            base_url,
            key,
            AuthStyle::Bearer,
            true,
        );

        if options.native_tools != Some(true) {
            p = p.without_native_tools();
        }

        Ok(apply_compat_options(p, options))
    }
}

fn apply_compat_options(
    mut p: OpenAiCompatibleModelProvider,
    opts: &ModelProviderRuntimeOptions,
) -> Box<dyn ModelProvider> {
    Box::new(p)
    // todo
}

pub fn dispatch_family_factory(
    config: Option<&Config>,
    family: &str,
    alias: &str,
    key: Option<&str>,
    api_url: Option<&str>,
    options: &ModelProviderRuntimeOptions,
) -> anyhow::Result<Box<dyn ModelProvider>> {
    macro_rules! emit_dispatch {
        ($(($field: ident,$type_str: literal, $cfg_ty: ty)) + $(,)?) => {
                match family {
                    "openai_compatible" => {
                        let default_cfg =shadow_config::schema::ModelProviderConfig::default();
                        let cfg = config.and_then(|c| c.providers.models.find("openai", alias)).unwrap_or(&default_cfg);
                        cfg.create_provider(alias, key, api_url, options)

                    }
                    $($type_str => {
                        let default_cfg : $cfg_ty;
                        let cfg: &$cfg_ty = match config.and_then(|c| c.providers.models.$field.get(alias)) {
                            Some(c) => c,
                            None => {
                                default_cfg = <$cfg_ty>::default();
                                &default_cfg
                            }
                        };
                        cfg.create_provider(alias, key, api_url, options)
                    } )+
                    _ => Err(anyhow::Error::msg(format!(
                        "Unknown model_provider family: {family}"
                    ))),
                }
            };
    }
    for_each_model_provider_slot!(emit_dispatch)
}
