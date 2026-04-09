use std::collections::HashMap;

use bollard::container::{
    Config as ContainerConfig, CreateContainerOptions, LogOutput, LogsOptions,
    RemoveContainerOptions, StartContainerOptions, StopContainerOptions,
};
use bollard::models::{HostConfig, Mount, MountTypeEnum};
use bollard::Docker;
use futures_util::StreamExt;

/// Unique container identifier returned by create.
#[derive(Debug, Clone)]
pub struct ContainerId(pub String);

/// Result of running a command inside a container.
#[derive(Debug)]
pub struct RunResult {
    pub exit_code: i64,
    pub stdout: String,
    pub stderr: String,
}

/// Specification for creating a task container.
#[derive(Debug)]
pub struct ContainerSpec {
    pub name: String,
    pub image: String,
    pub worktree_path: String,
    pub logs_path: String,
    pub env: Vec<String>,
    pub memory_bytes: Option<i64>,
    pub cpu_count: Option<i64>,
    pub pids_limit: Option<i64>,
    pub network: Option<String>,
}

/// Docker/OrbStack container runtime using bollard.
pub struct DockerRuntime {
    client: Docker,
}

impl DockerRuntime {
    pub fn new() -> anyhow::Result<Self> {
        let client = Docker::connect_with_local_defaults()?;
        Ok(Self { client })
    }

    pub async fn create(&self, spec: &ContainerSpec) -> anyhow::Result<ContainerId> {
        let mounts = vec![
            Mount {
                target: Some("/workspace".into()),
                source: Some(spec.worktree_path.clone()),
                typ: Some(MountTypeEnum::BIND),
                read_only: Some(false),
                ..Default::default()
            },
            Mount {
                target: Some("/var/log/reckoner".into()),
                source: Some(spec.logs_path.clone()),
                typ: Some(MountTypeEnum::BIND),
                read_only: Some(false),
                ..Default::default()
            },
        ];

        let host_config = HostConfig {
            mounts: Some(mounts),
            memory: spec.memory_bytes,
            nano_cpus: spec.cpu_count.map(|c| c * 1_000_000_000),
            pids_limit: spec.pids_limit,
            cap_drop: Some(vec!["ALL".into()]),
            security_opt: Some(vec!["no-new-privileges".into()]),
            network_mode: spec.network.clone(),
            tmpfs: Some(HashMap::from([
                ("/tmp".into(), "size=512m".into()),
                ("/home/agent/.cache".into(), "size=1073741824".into()),
            ])),
            ..Default::default()
        };

        let config = ContainerConfig {
            image: Some(spec.image.clone()),
            env: Some(spec.env.clone()),
            working_dir: Some("/workspace".into()),
            user: Some("1000:1000".into()),
            host_config: Some(host_config),
            ..Default::default()
        };

        let options = CreateContainerOptions {
            name: &spec.name,
            platform: None,
        };

        let resp = self.client.create_container(Some(options), config).await?;
        tracing::info!(container_id = %resp.id, name = %spec.name, "container created");
        Ok(ContainerId(resp.id))
    }

    pub async fn start(&self, id: &ContainerId) -> anyhow::Result<()> {
        self.client
            .start_container(&id.0, None::<StartContainerOptions<String>>)
            .await?;
        tracing::info!(container_id = %id.0, "container started");
        Ok(())
    }

    pub async fn stop(&self, id: &ContainerId) -> anyhow::Result<()> {
        self.client
            .stop_container(&id.0, Some(StopContainerOptions { t: 30 }))
            .await?;
        tracing::info!(container_id = %id.0, "container stopped");
        Ok(())
    }

    pub async fn remove(&self, id: &ContainerId) -> anyhow::Result<()> {
        self.client
            .remove_container(
                &id.0,
                Some(RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }),
            )
            .await?;
        tracing::info!(container_id = %id.0, "container removed");
        Ok(())
    }

    /// Run a command inside the container, capturing stdout and stderr.
    pub async fn run_command(
        &self,
        id: &ContainerId,
        cmd: &[&str],
    ) -> anyhow::Result<RunResult> {
        use bollard::exec::{CreateExecOptions, StartExecResults};

        let exec_instance = self
            .client
            .create_exec(
                &id.0,
                CreateExecOptions {
                    cmd: Some(cmd.iter().map(|s| s.to_string()).collect()),
                    attach_stdout: Some(true),
                    attach_stderr: Some(true),
                    working_dir: Some("/workspace".into()),
                    user: Some("1000:1000".into()),
                    ..Default::default()
                },
            )
            .await?;

        let mut stdout = String::new();
        let mut stderr = String::new();

        if let StartExecResults::Attached { mut output, .. } =
            self.client.start_exec(&exec_instance.id, None).await?
        {
            while let Some(Ok(msg)) = output.next().await {
                match msg {
                    LogOutput::StdOut { message } => {
                        stdout.push_str(&String::from_utf8_lossy(&message));
                    }
                    LogOutput::StdErr { message } => {
                        stderr.push_str(&String::from_utf8_lossy(&message));
                    }
                    _ => {}
                }
            }
        }

        let inspect = self.client.inspect_exec(&exec_instance.id).await?;
        let exit_code = inspect.exit_code.unwrap_or(-1);

        Ok(RunResult {
            exit_code,
            stdout,
            stderr,
        })
    }

    /// Collect container logs as a string.
    pub async fn collect_logs(&self, id: &ContainerId) -> anyhow::Result<String> {
        let mut output = String::new();
        let mut stream = self.client.logs(
            &id.0,
            Some(LogsOptions::<String> {
                stdout: true,
                stderr: true,
                follow: false,
                ..Default::default()
            }),
        );

        while let Some(Ok(msg)) = stream.next().await {
            match msg {
                LogOutput::StdOut { message } | LogOutput::StdErr { message } => {
                    output.push_str(&String::from_utf8_lossy(&message));
                }
                _ => {}
            }
        }

        Ok(output)
    }
}
