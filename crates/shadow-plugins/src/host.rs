use crate::{signature, PluginCapability, PluginManifest};
use crate::error::PluginError;
use crate::signature::{SignatureMode, VerificationResult};
use std::collections::HashMap;
use std::fs::exists;
use std::path::{Path, PathBuf};
use shadow_log::Severity::Error;

pub struct PluginHost {
    plugins_dir: PathBuf,
    loaded: HashMap<String, LoadedPlugin>,
    signature_mode: SignatureMode,
    trusted_publisher_keys: Vec<String>,
}

struct LoadedPlugin {
    manifest: PluginManifest,
    plugin_dir: PathBuf,
    wasm_path: Option<PathBuf>,
    verification: VerificationResult,
}

impl PluginHost {
    pub fn new(workspace_dir: &Path) -> Result<Self, PluginError> {
        Self::with_security(workspace_dir, SignatureMode::Disabled, Vec::new())
    }

    pub fn with_security(
        workspace_dir: &Path,
        signature_mode: SignatureMode,
        trusted_publisher_keys: Vec<String>,
    ) -> Result<Self, PluginError> {
        Self::from_plugins_dir_with_security(
            &workspace_dir.join("plugins"),
            signature_mode,
            trusted_publisher_keys,
        )
    }

    pub fn from_plugins_dir(plugins_dir: &Path) -> Result<Self, PluginError> {
        Self::from_plugins_dir_with_security(plugins_dir, SignatureMode::Disabled, Vec::new())
    }

    pub fn from_plugins_dir_with_security(
        plugins_dir: &Path,
        signature_mode: SignatureMode,
        trusted_publisher_keys: Vec<String>,
    ) -> Result<Self, PluginError> {
        if !plugins_dir.exists() {
            std::fs::create_dir_all(plugins_dir)?;
        }

        let mut host = Self {
            plugins_dir: plugins_dir.to_path_buf(),
            loaded: HashMap::new(),
            signature_mode,
            trusted_publisher_keys,
        };

        host.discover()?;

        Ok(host)
    }

    fn discover(&mut self) -> Result<(), PluginError> {
        if !self.plugins_dir.exists() {
            return Ok(());
        }

        let entries = std::fs::read_dir(&self.plugins_dir)?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let manifest_path = path.join("manifest.toml");
                if manifest_path.exists()
                    && let Ok(manifest) = self.load_manifest(&manifest_path)
                {
                    if let Err(e) = validate_manifest_shape(&manifest, &path) {
                        shadow_log::record!(
                            WARN,
                            shadow_log::Event::new(module_path!(), shadow_log::Action::Note)
                                .with_outcome(shadow_log::EventOutcome::Unknown)
                                .with_attrs(serde_json::json!({
                                    "plugin": path.display().to_string(),
                                    "error": e.to_string(),
                                })),
                            "skipping plugin due to invalid manifest shape"
                        );
                        continue;
                    }

                    let manifest_toml = std::fs::read_to_string(&manifest_path).unwrap_or_default();
                    match self.verify_plugin_signature(&manifest.name, &manifest_toml, &manifest) {
                        Ok(verification) => {
                            let wasm_path = manifest.wasm_path.as_deref().map(|p| path.join(p));
                            self.loaded.insert(
                                manifest.name.clone(),
                                LoadedPlugin {
                                    manifest,
                                    plugin_dir: path.clone(),
                                    wasm_path,
                                    verification,
                                },
                            );
                        }
                        Err(e) => {
                            shadow_log::record!(
                            WARN,
                            shadow_log::Event::new(module_path!(), shadow_log::Action::Note)
                                .with_outcome(shadow_log::EventOutcome::Unknown)
                                .with_attrs(serde_json::json!({
                                    "plugin": path.display().to_string(),
                                    "error": e.to_string(),
                                })),
                            "skipping plugin due to signature verification failure"
                        );
                        }
                    }
                }
            }
        }

        Ok(())
    }

    fn load_manifest(&self, path: &Path) -> Result<PluginManifest, PluginError> {
        let content= std::fs::read_to_string(path)?;
        let manifest: PluginManifest = toml::from_str(&content)?;
        Ok(manifest)
    }

    fn verify_plugin_signature(&self, name: &str, manifest_toml: &str, manifest: &PluginManifest) -> Result<VerificationResult, PluginError> {
        signature::enforce_signature_policy(name, manifest_toml, manifest.signature.as_deref(), manifest.publisher_key.as_deref(), &self.trusted_publisher_keys,
                                            self.signature_mode)
    }
}


fn validate_manifest_shape(manifest: &PluginManifest, plugin_dir: &Path) -> Result<(), PluginError> {
    if manifest.capabilities.is_empty() {
        return Err(PluginError::InvalidManifest(format!("plugin '{}' declares no capabilities", manifest.name)));
    }

    let is_skill_only = manifest.capabilities.len() == 1 && manifest.capabilities[0] == PluginCapability::Skill;
    if !is_skill_only && manifest.wasm_path.is_none() {
        return Err(PluginError::InvalidManifest(format!("plugin '{}' is missing required 'wasm_path' for non-skill capabilities", manifest.name)));
    }

    if manifest.capabilities.contains(&PluginCapability::Skill) {
        validate_skill_bundle(&manifest.name, plugin_dir)?;
    }
    Ok(())
}

const SKILLS_SUBDIR: &str = "skills";

fn validate_skill_bundle(plugin_name: &str, plugin_dir: &Path) -> Result<(), PluginError> {
    let skills_dir = plugin_dir.join(SKILLS_SUBDIR);
    if !skills_dir.is_dir() {
        return Err(PluginError::InvalidManifest(format!(
            "skill plugin '{}' is missing '{}/' directory at {}", plugin_name, SKILLS_SUBDIR, skills_dir.display()
        )));
    }

    let mut found_any = false;
    for entry in std::fs::read_dir(&skills_dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        found_any = true;
        let skill_md = path.join("SKILL.md");
        if !skill_md.is_file() {
            return Err(PluginError::InvalidManifest(format!(
                "skill plugin '{}' subdirectory '{}' is missing SKILL.md", plugin_name, path.file_name().and_then(|n| n.to_str()).unwrap_or("?")
            )));
        }
        validate_skill_md_frontmatter(plugin_name, &skill_md)?;
    }

    if !found_any {
        return Err(PluginError::InvalidManifest(format!(
            "skill plugin '{}' has empty '{}/' directory ", plugin_name, SKILLS_SUBDIR
        )));
    }

    Ok(())
}

fn validate_skill_md_frontmatter(plugin_name: &str, skill_md: &Path) -> Result<(), PluginError> {
    let content = std::fs::read_to_string(skill_md)?;
    let normalized = content.replace("\r\n", "\n");
    let rest = normalized.strip_prefix("---\n").ok_or_else(|| {
        PluginError::InvalidManifest(format!(
            "skill plugin '{}': {} is missing YAML frontmatter",
            plugin_name,
            skill_md.display()
        ))
    })?;
    let frontmatter = if let Some(idx) = rest.find("\n---\n") {
        &rest[..idx]
    } else if let Some(stripped) = rest.strip_suffix("\n---") {
        stripped
    } else {
        return Err(PluginError::InvalidManifest(format!(
            "skill plugin '{}': {} has unterminated frontmatter",
            plugin_name,
            skill_md.display()
        )));
    };

    let mut has_name = false;
    let mut has_description = false;
    for line in frontmatter.lines() {
        let trimmed = line.trim_start();
        if let Some((key, value)) = trimmed.split_once(':') {
            let key = key.trim();
            let value = value.trim();
            let has_value = !value.is_empty();
            match key {
                "name" if has_value => has_name = true,
                "description" if has_value => has_description = true,
                _ => {}
            }
        }
    }
    if !has_name || !has_description {
        return Err(PluginError::InvalidManifest(format!(
            "skill plugin '{}': {} frontmatter must declare `name` and `description`",
            plugin_name,
            skill_md.display()
        )));
    }

    Ok(())
}