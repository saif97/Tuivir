use std::{future::Future, pin::Pin};

use serde::Deserialize;

use crate::{
    cli::{CliRunner, ProcessError, ProcessSpec},
    provider::{
        ProviderDiscovery, ProviderId, ProviderWorkspace, Resource, ResourceCommand, ResourceId,
        ResourcePanel, ResourceState, WorkspaceError, WorkspaceSnapshot,
    },
};

const PROVIDER_ID: &str = "docker";
const PROVIDER_NAME: &str = "Docker";

pub struct DockerWorkspace;

#[derive(Deserialize)]
struct ContainerRow {
    #[serde(rename = "ID")]
    id: String,
    #[serde(rename = "Image")]
    image: String,
    #[serde(rename = "Names")]
    names: String,
    #[serde(rename = "State")]
    state: String,
    #[serde(rename = "Status")]
    status: String,
}

impl ProviderWorkspace for DockerWorkspace {
    fn id(&self) -> ProviderId {
        ProviderId::new(PROVIDER_ID)
    }

    fn discover<'a>(
        &'a self,
        cli: &'a dyn CliRunner,
    ) -> Pin<Box<dyn Future<Output = Option<ProviderDiscovery>> + Send + 'a>> {
        Box::pin(async move {
            let result = cli
                .run(ProcessSpec::new("docker", &["context", "show"]))
                .await;

            match result {
                Err(ProcessError::ExecutableNotFound) => None,
                Err(ProcessError::SpawnFailed(message)) => {
                    Some(discovery_with_error(not_started(&message)))
                }
                Err(ProcessError::Exited(failure)) => Some(discovery_with_error(
                    failure.message_or("Docker could not report its current context"),
                )),
                Ok(output) => Some(ProviderDiscovery {
                    id: self.id(),
                    name: PROVIDER_NAME.to_owned(),
                    target_environment: output.stdout.trim().to_owned(),
                    error: None,
                }),
            }
        })
    }

    fn refresh<'a>(
        &'a self,
        cli: &'a dyn CliRunner,
    ) -> Pin<Box<dyn Future<Output = Result<WorkspaceSnapshot, WorkspaceError>> + Send + 'a>> {
        Box::pin(async move {
            let output = cli
                .run(ProcessSpec::new(
                    "docker",
                    &[
                        "container",
                        "ls",
                        "--all",
                        "--no-trunc",
                        "--format",
                        "{{json .}}",
                    ],
                ))
                .await
                .map_err(|error| match error {
                    ProcessError::ExecutableNotFound => {
                        WorkspaceError::new("Docker CLI is no longer available")
                    }
                    ProcessError::SpawnFailed(message) => {
                        WorkspaceError::new(not_started(&message))
                    }
                    ProcessError::Exited(failure) => {
                        refresh_error(failure.message_or("Docker could not list containers"))
                    }
                })?;

            let resources = output
                .stdout
                .lines()
                .filter(|line| !line.trim().is_empty())
                .map(|line| {
                    let row: ContainerRow = serde_json::from_str(line).map_err(|error| {
                        refresh_error(format!("Docker returned malformed container data: {error}"))
                    })?;
                    let state = docker_resource_state(&row.state);
                    let available_commands = docker_commands(state);
                    Ok(Resource {
                        id: ResourceId::new(row.id),
                        name: row.names,
                        status: Some(row.state),
                        state,
                        fields: vec![
                            ("Image".to_owned(), row.image),
                            ("Status".to_owned(), row.status),
                        ],
                        available_commands,
                    })
                })
                .collect::<Result<Vec<_>, WorkspaceError>>()?;

            Ok(WorkspaceSnapshot {
                panels: vec![ResourcePanel {
                    title: "Containers".to_owned(),
                    resources,
                }],
            })
        })
    }

    fn execute_command<'a>(
        &'a self,
        cli: &'a dyn CliRunner,
        resource_id: &'a ResourceId,
        command: ResourceCommand,
        state: ResourceState,
    ) -> Pin<Box<dyn Future<Output = Result<(), WorkspaceError>> + Send + 'a>> {
        Box::pin(async move {
            let verb = match command {
                ResourceCommand::Start => "start",
                ResourceCommand::Stop => "stop",
                ResourceCommand::Restart => "restart",
                ResourceCommand::Delete => "rm",
            };
            let mut args = vec!["container", verb];
            // Docker removes a container plainly only from a stopped state; a
            // running, paused, or restarting one needs the force the user
            // already confirmed.
            if command == ResourceCommand::Delete && !state.is_stopped() {
                args.push("--force");
            }
            args.push(resource_id.0.as_str());
            cli.run(ProcessSpec::new("docker", &args))
                .await
                .map_err(|error| {
                    command_error(
                        error,
                        &format!("Docker could not {command} container {resource_id}"),
                    )
                })?;
            Ok(())
        })
    }
}

/// Maps a Docker container's `State` onto the shared vocabulary.
///
/// Docker's own set is `created`, `running`, `paused`, `restarting`,
/// `removing`, `exited`, and `dead`; anything else is deliberately left
/// `Unknown` rather than assumed to be stopped.
fn docker_resource_state(state: &str) -> ResourceState {
    match state.to_ascii_lowercase().as_str() {
        "running" => ResourceState::Running,
        "exited" | "created" => ResourceState::Stopped,
        "paused" => ResourceState::Paused,
        "restarting" | "removing" => ResourceState::Transitioning,
        "dead" => ResourceState::Broken,
        _ => ResourceState::Unknown,
    }
}

fn docker_commands(state: ResourceState) -> Vec<ResourceCommand> {
    match state {
        ResourceState::Running => vec![
            ResourceCommand::Stop,
            ResourceCommand::Restart,
            ResourceCommand::Delete,
        ],
        ResourceState::Stopped => vec![ResourceCommand::Start, ResourceCommand::Delete],
        // Starting a paused container fails — it needs an unpause Virtui does
        // not offer yet — and a transitioning, dead, or unrecognised container
        // has no lifecycle Command that reliably applies. Deletion always does.
        ResourceState::Paused
        | ResourceState::Transitioning
        | ResourceState::Broken
        | ResourceState::Unknown => vec![ResourceCommand::Delete],
    }
}

fn discovery_with_error(message: impl Into<String>) -> ProviderDiscovery {
    let message = message.into();
    ProviderDiscovery {
        id: ProviderId::new(PROVIDER_ID),
        name: PROVIDER_NAME.to_owned(),
        target_environment: "unavailable".to_owned(),
        error: Some(WorkspaceError::new(format!(
            "{message}. Run `docker context show` to verify the selected context and ensure Docker is running."
        ))),
    }
}

fn not_started(reason: &str) -> String {
    format!("Docker CLI could not be started: {reason}")
}

fn refresh_error(message: impl Into<String>) -> WorkspaceError {
    WorkspaceError::new(format!(
        "{}. Run `docker container ls --all` to verify access to the current Target Environment.",
        message.into()
    ))
}

fn command_error(error: ProcessError, fallback: &str) -> WorkspaceError {
    let message = match error {
        ProcessError::ExecutableNotFound => "Docker CLI is no longer available".to_owned(),
        ProcessError::SpawnFailed(message) => not_started(&message),
        ProcessError::Exited(failure) => failure.message_or(fallback),
    };
    WorkspaceError::new(message)
}
