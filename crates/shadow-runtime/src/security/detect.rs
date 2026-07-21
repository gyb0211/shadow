use crate::security::{NoopSandbox, Sandbox};
use shadow_config::RuntimeKind;
use shadow_config::multi::risk_profile::SandboxConfig;
use shadow_config::risk_profile::SandboxBackend;
use std::path::Path;
use std::sync::Arc;
use crate::security::seatbelt::SeatbeltSandbox;

pub fn create_sandbox(
    sandbox: &SandboxConfig,
    runtime_kind: &str,
    workspace_dir: Option<&Path>,
) -> Arc<dyn Sandbox> {
    let backend = &sandbox.backend;
    if matches!(backend, SandboxBackend::None) || sandbox.enabled == Some(false) {
        return Arc::new(NoopSandbox);
    }

    match backend {
        SandboxBackend::Auto | SandboxBackend::None => {
            detect_best_sandbox(runtime_kind, workspace_dir)
        }
        requested => {
            let selected = configured_backend_selection(requested, runtime_kind, workspace_dir);
            if let Some(sandbox) = create_selected_sandbox(selected, workspace_dir) {
                return sandbox;
            }
            Arc::new(NoopSandbox)
        }
    }
}

fn configured_backend_selection(
    backend: &SandboxBackend,
    runtime_kind: &str,
    workspace_dir: Option<&Path>,
) -> SelectedSandboxBackend {
    if matches!(backend, SandboxBackend::None) {
        return detect_best_backend(runtime_kind, workspace_dir);
    }
    SelectedSandboxBackend::from_config(backend)
        .filter(|selected| sandbox_backend_available(*selected, workspace_dir))
        .unwrap_or(SelectedSandboxBackend::None)
}

fn create_selected_sandbox(
    selected: SelectedSandboxBackend,
    workspace_dir: Option<&Path>,
) -> Option<Arc<dyn Sandbox>> {
    match selected {
        SelectedSandboxBackend::None => None,
        SelectedSandboxBackend::Landlock => {
            #[cfg(all(feature = "sandbox-landlock", target_os = "linux"))]
            {
                None
            }
            #[cfg(not(all(feature = "sandbox-landlock", target_os = "linux")))]
            {
                None
            }
        }
        SelectedSandboxBackend::Firejail => {
            #[cfg(target_os = "linux")]
            {
                None
            }
            #[cfg(not(target_os = "linux"))]
            {
                None
            }
        }
        SelectedSandboxBackend::Bubblewrap => {
            #[cfg(all(
                feature = "sandbox-bubblewrap",
                any(target_os = "linux", target_os = "macos")
            ))]
            {
                None
            }
            #[cfg(not(all(
                feature = "sandbox-bubblewrap",
                any(target_os = "linux", target_os = "macos")
            )))]
            {
                None
            }
        }
        SelectedSandboxBackend::Docker => {
            None
        }
        SelectedSandboxBackend::SandboxExec => {
            #[cfg(target_os = "macos")]
            {
                SeatbeltSandbox::with_workspace(workspace_dir)
                    .map(|sandbox| Arc::new(sandbox) as Arc<dyn Sandbox>)
                    .ok()
            }
            #[cfg(not(target_os = "macos"))]
            {
                None
            }
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
        {}
    }

    #[cfg(target_os = "macos")]
    {
        #[cfg(feature = "sandbox-bubblewrap")]
        {}

        if sandbox_backend_available(SelectedSandboxBackend::SandboxExec, workspace_dir) {
            return SelectedSandboxBackend::SandboxExec;
        }
    }
}

fn sandbox_backend_available(
    backend: SelectedSandboxBackend,
    workspace_dir: Option<&Path>,
) -> bool {
    match backend {
        SelectedSandboxBackend::None => true,
        SelectedSandboxBackend::Landlock => landlock_available(workspace_dir),
        SelectedSandboxBackend::Firejail => {
            #[cfg(target_os = "linux")]
            {
                false
            }
            #[cfg(not(target_os = "linux"))]
            {
                false
            }
        }
        SelectedSandboxBackend::Bubblewrap => {
            #[cfg(feature = "sandbox-bubblewrap")]
            {
                false
            }
            #[cfg(not(feature = "sandbox-bubblewrap"))]
            {
                false
            }
        }
        SelectedSandboxBackend::Docker => false,
        SelectedSandboxBackend::SandboxExec => seatbelt_available(),
    }
}

#[cfg(target_os = "macos")]
fn seatbelt_available() -> bool {
    Path::new("/usr/bin/sandbox-exec").exists()
        || std::process::Command::new("sandbox-exec")
            .args(["-n", "no-network", "true"])
            .output()
            .map(|out| out.status.success())
            .unwrap_or(false)
}

#[cfg(not(target_os = "macos"))]
fn seatbelt_available() -> bool {
    false
}

#[cfg(not(all(feature = "sandbox-landlock", target_os = "linux")))]
fn landlock_available(workspace_dir: Option<&Path>) -> bool {
    false
}

#[cfg(all(feature = "sandbox-landlock", target_os = "linux"))]
fn landlock_available(workspace_dir: Option<&Path>) -> bool {
    false
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SelectedSandboxBackend {
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
    fn from_config(backend: &SandboxBackend) -> Option<Self> {
        match backend {
            SandboxBackend::Auto | SandboxBackend::None => None,
            SandboxBackend::Landlock => Some(Self::Landlock),
            SandboxBackend::Firejail => Some(Self::Firejail),
            SandboxBackend::Bubblewrap => Some(Self::Bubblewrap),
            SandboxBackend::Docker => Some(Self::Docker),
            SandboxBackend::SandboxExec => Some(Self::SandboxExec),
        }
    }
}
