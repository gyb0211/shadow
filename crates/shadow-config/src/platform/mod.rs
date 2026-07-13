pub mod native;
pub mod docker;

use shadow_core::platform;
use shadow_core::runtime::RuntimeAdapter;
use crate::{RuntimeConfig, RuntimeKind};
use crate::platform::docker::DockerRuntime;
use crate::platform::native::NativeRuntime;

pub fn create_runtime(config: &RuntimeConfig) -> anyhow::Result<Box<dyn RuntimeAdapter>> {
    match config.kind {
        RuntimeKind::Native => {
            let shell = config.shell.clone().unwrap_or_else(|| "sh".into());
            #[cfg(unix)]
            validate_shell(&shell)?;
            Ok(Box::new(NativeRuntime::with_shell(shell)))
        }
        RuntimeKind::Docker => Ok(Box::new(DockerRuntime::new(config.docker.clone()))),
        RuntimeKind::Cloudflare => anyhow::bail!(
            "runtime.kind='cloudflare' is not implemented yet. Use runtime.kind='native' for now."
        ),
    }
}

#[cfg(unix)]
fn validate_shell(shell: &str) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    // Android pins the shell to /system/bin/sh; the configured value is never
    // used, so don't reject it.
    if platform::is_android() {
        return Ok(());
    }

    if shell.trim().is_empty() {
        anyhow::bail!("runtime.shell must not be empty or whitespace");
    }

    let path = std::path::Path::new(shell);
    let resolved = if path.is_absolute() {
        path.to_path_buf()
    } else if path.components().count() > 1 {
        anyhow::bail!(
            "runtime.shell {shell:?} is a relative path; use a bare name resolved on PATH (e.g. \"bash\") or an absolute path (e.g. \"/bin/bash\")"
        );
    } else {
        match std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
            .map(|dir| dir.join(shell))
            .find(|candidate| candidate.is_file())
        {
            Some(found) => found,
            None => anyhow::bail!(
                "runtime.shell {shell:?} was not found on PATH; use an absolute path or install the shell"
            ),
        }
    };

    if !resolved.exists() {
        anyhow::bail!(
            "runtime.shell {shell:?} (resolved to {}) does not exist",
            resolved.display()
        );
    }

    // Coarse check: reject only when no execute bit is set at all. A precise
    // "can *we* execute it" test (uid/gid vs. the file owner) buys little —
    // the kernel's spawn is the real authority (ACLs, caps, mount flags) — and
    // this is a fail-fast sanity check, not a security gate.
    let mode = match resolved.metadata() {
        Ok(meta) => meta.permissions().mode(),
        Err(e) => anyhow::bail!(
            "runtime.shell {shell:?} (resolved to {}) could not be inspected: {e}",
            resolved.display()
        ),
    };
    if mode & 0o111 == 0 {
        anyhow::bail!(
            "runtime.shell {shell:?} (resolved to {}) is not executable",
            resolved.display()
        );
    }

    Ok(())
}