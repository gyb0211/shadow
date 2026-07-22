use crate::models_dev;

pub async fn list_models_for_family(family: &str) -> anyhow::Result<Vec<String>> {
    let Some((md, prefix)) = catalog_source_for(family) else {
        anyhow::bail!("unknown provider family {family:?}");
    };

    if let Some(k) = md
        && let Ok(ms) = models_dev::list_models_for(k).await
        && !ms.is_empty()
    {
        return Ok(ms);
    }

    if let Some(p) = prefix {
        // todo
    }

    anyhow::bail!("no public catalog for family {family:?}")
}

pub fn catalog_source_for(family: &str) -> Option<(Option<&'static str>, Option<&'static str>)> {
    let pair: (Option<&'static str>, Option<&'static str>) = match family {
        "openai" => (Some("openai"), Some("openai")),
        "anthropic" => (Some("anthropic"), Some("anthropic")),
        "gemini" => (Some("google"), Some("google")),
        "minimax" => (Some("minimax"), Some("minimax")),
        "glm" => (Some("zhupuai"), None),
        "qwen" => (Some("alibaba"), Some("qwen")),
        "deepseek" => (Some("deepseek"), Some("deepseek")),
        "custom" => (None, None),
        _ => return None,
    };

    Some(pair)
}
