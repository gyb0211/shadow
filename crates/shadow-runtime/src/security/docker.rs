use crate::security::Sandbox;
use std::path::PathBuf;
use std::process::Command;

#[derive(Debug, Clone)]
pub struct DockerSandbox {
    image: String,
    workspace_dir: Option<PathBuf>,
}

impl Default for DockerSandbox {
    fn default() -> Self {
        Self {
            image: "alpine:latest".to_string(),
            workspace_dir: None,
        }
    }
}

impl DockerSandbox {
    fn is_installed() -> bool {
        Command::new("docker")
            .arg("--version")
            .output()
            .map(|r| r.status.success())
            .unwrap_or(false)
    }

    pub fn default_image() -> String {
        Self::default().image
    }

    pub fn new() -> std::io::Result<Self> {
        if Self::is_installed() {
            Ok(Self::default())
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "Docker not found, please check docker --version",
            ))
        }
    }

    pub fn with_workspace(image: String, workspace_dir: PathBuf) -> std::io::Result<Self> {
        if Self::is_installed() {
            Ok(Self {
                image,
                workspace_dir: Some(workspace_dir),
            })
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "Docker not found, please check docker --version",
            ))
        }
    }

    pub fn with_image(image: String) -> std::io::Result<Self> {
        if Self::is_installed() {
            Ok(Self {
                image,
                workspace_dir: None,
            })
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "Docker not found, please check docker --version",
            ))
        }
    }

    pub fn probe() -> std::io::Result<Self> {
        Self::new()
    }
}

impl Sandbox for DockerSandbox {
    fn wrap_command(&self, cmd: &mut Command) -> std::io::Result<()> {
        let program = cmd.get_program().to_string_lossy().to_string();
        let args: Vec<String> = cmd
            .get_args()
            .map(|s| s.to_string_lossy().to_string())
            .collect();
        let mut docker_cmd = Command::new("docker");
        docker_cmd.args([
            "run",
            "--rm",
            "--memory",
            "512m",
            "--cpus",
            "1.0",
            "--network",
            "none",
        ]);

        if let Some(workspace) = &self.workspace_dir {
            let workspace = workspace.to_string_lossy();
            docker_cmd.arg("-v");
            docker_cmd.arg(format!("{workspace}:{workspace}:ro"));
            docker_cmd.arg("-workdir");
            docker_cmd.arg(workspace.as_ref());
        }

        docker_cmd.arg(&self.image);
        docker_cmd.arg(program);
        docker_cmd.args(&args);
        *cmd = docker_cmd;
        Ok(())
    }

    fn is_available(&self) -> bool {
        Self::is_installed()
    }

    fn name(&self) -> &str {
        "docker"
    }

    fn description(&self) -> &str {
        "Docker container isolation (requires docker)"
    }
}
