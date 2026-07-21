use std::path::Path;
use std::sync::Arc;
use shadow_config::multi::risk_profile::SandboxConfig;
use shadow_config::risk_profile::SandboxBackend;
use shadow_config::RuntimeKind;
use crate::security::{NoopSandbox, Sandbox};

pub fn create_sandbox(sandbox: &SandboxConfig, runtime_kind: &str, workspace_dir: Option<&Path>) -> Arc<dyn Sandbox>{
    let backend = &sandbox.backend;
    if matches!(backend, SandboxBackend::None) || sandbox.enabled == Some(false) {
        return Arc::new(NoopSandbox);
    }
    
    match backend {
        SandboxBackend::Auto |  SandboxBackend::None => {
            detect_best_sandbox(runtime_kind, workspace_dir);
        }
        requested => {
            let selected = configured_backedn_selection(requested, runtime_kind, workspace_dir);
            if let Some(sandbox) = create_selected_sandbox(selected, workspace_dir) {
                return sandbox;
            }
            Arc::new(NoopSandbox)
        }
    } 
    
}

fn detect_best_sandbox(runtime_kind: &str, workspace_dir: Option<&Path>) -> Arc<dyn Sandbox> {
    let selected = detect_best_backend(runtime_kind, workspace_dir);
    if let Some(sandbox) = create_selected_sandbox(selected, workspace_dir) {
        return sandbox;
    }
    Arc::new(NoopSandbox)
}

fn detect_best_backend(runtime_kind: &str, workspace_dir: Option<&Path>) -> SelectedSandboxBackend {
    let skip_docker = runtime_kind == "native";
    #[cfg(target_os = "linux")]
    {
        #[cfg(feature = "sandbox-landlock")]
        {
            
        }
    }

    #[cfg(target_os = "macos")]
    {
        #[cfg(feature = "sandbox-bubblewrap")]
        {
            
        }
        
        if sandbox_backend_available(SelectedSandboxBackend::SandboxExec, workspace_dir) {
            return SelectedSandboxBackend::SandboxExec;
        }
    }
}

fn sandbox_backend_available(backend: SelectedSandboxBackend, , workspace_dir: Option<&Path>) -> bool {
    match backend {
        SelectedSandboxBackend::None => {}
        SelectedSandboxBackend::Landlock => {}
        SelectedSandboxBackend::Firejail => {}
        SelectedSandboxBackend::Bubblewrap => {}
        SelectedSandboxBackend::Docker => {}
        SelectedSandboxBackend::SandboxExec => {}
    }
}



enum SelectedSandboxBackend{
    None,
    Landlock,
    Firejail,
    Bubblewrap,
    Docker,
    SandboxExec,
}
const NOOP_DESCRIPTION: &str = "No sandboxing (application-layer security only)";
const LANDLOCK_DESCRIPTION: &str = "Linux kernel LSM sandboxing (filesystem access control)";
const FIREJAIL_DESCRIPTION: &str = "Linux user-space sandbox (requires firejail to be installed)";
const BUBBLEWRAP_DESCRIPTION: &str = "User namespace sandbox (requires bwrap)";
const DOCKER_DESCRIPTION: &str = "Docker container isolation (requires docker)";
const SEATBELT_DESCRIPTION: &str = "macOS Seatbelt sandbox (built-in sandbox-exec)";
impl SelectedSandboxBackend {
    fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Landlock => "landlock",
            Self::Firejail => "firejail",
            Self::Bubblewrap => "bubblewrap",
            Self::Docker => "docker",
            Self::SandboxExec => "sandbox-exec",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::None => NOOP_DESCRIPTION,
            Self::Landlock => LANDLOCK_DESCRIPTION,
            Self::Firejail => FIREJAIL_DESCRIPTION,
            Self::Bubblewrap => BUBBLEWRAP_DESCRIPTION,
            Self::Docker => DOCKER_DESCRIPTION,
            Self::SandboxExec => SEATBELT_DESCRIPTION,
        }
    }
}
