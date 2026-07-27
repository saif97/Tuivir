use std::{future::Future, pin::Pin};

use serde::Deserialize;

use crate::{
    cli::{CliRunner, ProcessError, ProcessSpec},
    provider::{
        DetailView, DetailViewId, ProviderDiscovery, ProviderId, ProviderWorkspace, Resource,
        ResourceCommand, ResourceDetails, ResourceId, ResourcePanel, ResourceState, WorkspaceError,
        WorkspaceSnapshot, provider_cli_error,
    },
};

const PROVIDER_ID: &str = "docker";
const PROVIDER_NAME: &str = "Docker";
/// What a user can run to check the Target Environment a refresh could not read.
const REFRESH_HELP: &str =
    "Run `docker container ls --all` to verify access to the current Target Environment.";

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
                Err(ProcessError::SpawnFailed(message)) => Some(discovery_with_error(format!(
                    "{PROVIDER_NAME} CLI could not be started: {message}"
                ))),
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
                .map_err(refresh_failure)?;

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
                    detail_views: container_detail_views(),
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
                ResourceCommand::Resume => "unpause",
                ResourceCommand::Delete => "rm",
            };
            let mut args = vec!["container", verb];
            // Docker removes a container plainly only from a stopped state; a
            // running, paused, or restarting one needs the force the user
            // already confirmed.
            if command == ResourceCommand::Delete && state != ResourceState::Stopped {
                args.push("--force");
            }
            args.push(resource_id.0.as_str());
            cli.run(ProcessSpec::new("docker", &args))
                .await
                .map_err(|error| {
                    WorkspaceError::new(provider_cli_error(
                        PROVIDER_NAME,
                        &error,
                        &format!("Docker could not {command} container {resource_id}"),
                    ))
                })?;
            Ok(())
        })
    }

    fn load_details<'a>(
        &'a self,
        cli: &'a dyn CliRunner,
        resource_id: &'a ResourceId,
        view_id: &'a DetailViewId,
    ) -> Pin<Box<dyn Future<Output = Result<ResourceDetails, WorkspaceError>> + Send + 'a>> {
        Box::pin(async move {
            let Some(args) = container_detail_command(view_id, resource_id.0.as_str()) else {
                return Err(WorkspaceError::new(format!(
                    "Docker has no {view_id} view for container {resource_id}"
                )));
            };
            let output = cli
                .run(ProcessSpec::new("docker", &args))
                .await
                .map_err(|error| {
                    WorkspaceError::new(provider_cli_error(
                        PROVIDER_NAME,
                        &error,
                        &format!("Docker could not load {view_id} for container {resource_id}"),
                    ))
                })?;
            // A container writes wherever it likes, so both streams are its
            // output. Only a non-zero exit means Docker itself failed.
            let mut details = ResourceDetails::from_output(&output.stdout);
            details
                .lines
                .extend(ResourceDetails::from_output(&output.stderr).lines);
            Ok(details)
        })
    }
}

/// The Docker command behind each declared view, or `None` for a view this
/// workspace never declared.
fn container_detail_command<'a>(
    view_id: &DetailViewId,
    resource_id: &'a str,
) -> Option<Vec<&'a str>> {
    match view_id.0.as_str() {
        "logs" => Some(vec!["container", "logs", "--tail", "200", resource_id]),
        "stats" => Some(vec!["container", "stats", "--no-stream", resource_id]),
        "inspect" => Some(vec!["container", "inspect", resource_id]),
        _ => None,
    }
}

/// The diagnostics Docker itself offers for a container, in the order the user
/// moves through them.
fn container_detail_views() -> Vec<DetailView> {
    vec![
        DetailView::new("logs", "Logs"),
        DetailView::new("stats", "Stats"),
        DetailView::new("inspect", "Inspect"),
    ]
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
        // A paused container resumes rather than starts: `docker container
        // start` fails against it, and `unpause` fails against everything else.
        ResourceState::Paused => vec![ResourceCommand::Resume, ResourceCommand::Delete],
        // A transitioning, dead, or unrecognised container has no lifecycle
        // Command that reliably applies. Deletion always does.
        ResourceState::Transitioning | ResourceState::Broken | ResourceState::Unknown => {
            vec![ResourceCommand::Delete]
        }
    }
}

fn discovery_with_error(message: impl Into<String>) -> ProviderDiscovery {
    let message = message.into();
    ProviderDiscovery {
        id: ProviderId::new(PROVIDER_ID),
        name: PROVIDER_NAME.to_owned(),
        target_environment: "unavailable".to_owned(),
        error: Some(WorkspaceError::with_help(
            message,
            "Run `docker context show` to verify the selected context and ensure Docker is running.",
        )),
    }
}

/// A failed listing, carrying help only where it applies.
///
/// A Docker that is gone or would not start cannot answer `docker container
/// ls`, so suggesting it would send the user nowhere.
fn refresh_failure(error: ProcessError) -> WorkspaceError {
    let message = provider_cli_error(PROVIDER_NAME, &error, "Docker could not list containers");
    match error {
        ProcessError::Exited(_) => refresh_error(message),
        _ => WorkspaceError::new(message),
    }
}

fn refresh_error(message: impl AsRef<str>) -> WorkspaceError {
    WorkspaceError::with_help(message, REFRESH_HELP)
}
