use std::{future::Future, pin::Pin};

use serde::Deserialize;

use crate::{
    application::{InteractiveShellProcess, LifecycleCommandPolicy, lifecycle_commands},
    infrastructure::process::{CliRunner, ProcessError, ProcessSpec},
    infrastructure::provider::{
        DetailView, DetailViewId, Provider, ProviderDiscovery, ProviderId, ProviderWorkspace,
        Resource, ResourceCommand, ResourceDetails, ResourceId, ResourcePanel, ResourcePanelId,
        ResourceState, ResourceTarget, TargetEnvironment, WorkspaceError, WorkspaceSnapshot,
        provider_cli_error,
    },
};

const PROVIDER_ID: &str = "docker";
const PROVIDER_NAME: &str = "Docker";
const CONTAINER_REFRESH_HELP: &str =
    "Run `docker container ls --all` to verify access to the current Target Environment.";
const IMAGE_REFRESH_HELP: &str =
    "Run `docker image ls` to verify access to the current Target Environment.";
const CONTAINERS_PANEL_ID: &str = "containers";
const IMAGES_PANEL_ID: &str = "images";

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

#[derive(Deserialize)]
struct ImageRow {
    #[serde(rename = "ID")]
    id: String,
    #[serde(rename = "Repository")]
    repository: String,
    #[serde(rename = "Tag")]
    tag: String,
    #[serde(rename = "Size")]
    size: String,
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
                Ok(output) => Some(ProviderDiscovery::new(
                    Provider::new(
                        self.id(),
                        PROVIDER_NAME,
                        Some(TargetEnvironment::new(output.stdout.trim())),
                        None,
                    ),
                    None,
                )),
            }
        })
    }

    fn refresh<'a>(
        &'a self,
        cli: &'a dyn CliRunner,
    ) -> Pin<Box<dyn Future<Output = Result<WorkspaceSnapshot, WorkspaceError>> + Send + 'a>> {
        Box::pin(async move {
            // Two independent listings, so they wait on Docker together rather
            // than one after the other.
            let (containers, images) = tokio::try_join!(
                async {
                    cli.run(ProcessSpec::new(
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
                    .map_err(|error| {
                        refresh_failure(
                            error,
                            "Docker could not list containers",
                            CONTAINER_REFRESH_HELP,
                        )
                    })
                },
                async {
                    cli.run(ProcessSpec::new(
                        "docker",
                        &["image", "ls", "--no-trunc", "--format", "{{json .}}"],
                    ))
                    .await
                    .map_err(|error| {
                        refresh_failure(error, "Docker could not list images", IMAGE_REFRESH_HELP)
                    })
                },
            )?;

            let container_resources = containers
                .stdout
                .lines()
                .filter(|line| !line.trim().is_empty())
                .map(|line| {
                    let row: ContainerRow = serde_json::from_str(line).map_err(|error| {
                        refresh_error(
                            format!("Docker returned malformed container data: {error}"),
                            CONTAINER_REFRESH_HELP,
                        )
                    })?;
                    let state = docker_resource_state(&row.state);
                    let available_commands =
                        lifecycle_commands(state, LifecycleCommandPolicy::Restartable);
                    let shell = container_shell(state, &row.id);
                    Ok(Resource {
                        id: ResourceId::new(row.id),
                        name: row.names,
                        status: Some(row.state),
                        state: Some(state),
                        fields: vec![("Image", row.image), ("Status", row.status)],
                        snapshot_details: Vec::new(),
                        available_commands,
                        shell,
                    })
                })
                .collect::<Result<Vec<_>, WorkspaceError>>()?;

            let image_resources = images
                .stdout
                .lines()
                .filter(|line| !line.trim().is_empty())
                .map(|line| {
                    let row: ImageRow = serde_json::from_str(line).map_err(|error| {
                        refresh_error(
                            format!("Docker returned malformed image data: {error}"),
                            IMAGE_REFRESH_HELP,
                        )
                    })?;
                    // One image carries one digest per tag it was given, so the
                    // digest repeats down the listing and cannot identify a
                    // row. `repository:tag` is what Docker itself accepts and
                    // what it holds unique, so it identifies the Resource; an
                    // untagged image has only its digest to be known by.
                    let name = match (row.repository.as_str(), row.tag.as_str()) {
                        ("<none>", _) | (_, "<none>") => row.id.clone(),
                        _ => format!("{}:{}", row.repository, row.tag),
                    };
                    Ok(Resource {
                        id: ResourceId::new(name.clone()),
                        name,
                        status: None,
                        state: None,
                        fields: vec![
                            ("Repository", row.repository),
                            ("Tag", row.tag),
                            ("Identity", row.id),
                            ("Size", row.size),
                        ],
                        snapshot_details: Vec::new(),
                        available_commands: &[],
                        shell: None,
                    })
                })
                .collect::<Result<Vec<_>, WorkspaceError>>()?;

            Ok(WorkspaceSnapshot {
                panels: vec![
                    ResourcePanel {
                        id: ResourcePanelId::new(CONTAINERS_PANEL_ID),
                        title: "Containers".to_owned(),
                        detail_views: container_detail_views(),
                        resources: container_resources,
                    },
                    ResourcePanel {
                        id: ResourcePanelId::new(IMAGES_PANEL_ID),
                        title: "Images".to_owned(),
                        detail_views: vec![DetailView::new("inspect", "Inspect")],
                        resources: image_resources,
                    },
                ],
            })
        })
    }

    fn execute_command<'a>(
        &'a self,
        cli: &'a dyn CliRunner,
        target: &'a ResourceTarget,
        command: ResourceCommand,
        state: ResourceState,
    ) -> Pin<Box<dyn Future<Output = Result<(), WorkspaceError>> + Send + 'a>> {
        Box::pin(async move {
            let panel_id = target.panel_id();
            let resource_id = target.resource_id();
            if panel_id.0 != CONTAINERS_PANEL_ID {
                return Err(WorkspaceError::new(format!(
                    "Docker has no {command} command for Resource Panel {panel_id}"
                )));
            }
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
        target: &'a ResourceTarget,
        view_id: &'a DetailViewId,
    ) -> Pin<Box<dyn Future<Output = Result<ResourceDetails, WorkspaceError>> + Send + 'a>> {
        Box::pin(async move {
            let panel_id = target.panel_id();
            let resource_id = target.resource_id();
            let (resource_kind, args) = match panel_id.0.as_str() {
                CONTAINERS_PANEL_ID => (
                    "container",
                    container_detail_command(view_id, resource_id.0.as_str()),
                ),
                IMAGES_PANEL_ID if view_id.0 == "inspect" => (
                    "image",
                    Some(vec!["image", "inspect", resource_id.0.as_str()]),
                ),
                _ => (panel_id.0.as_str(), None),
            };
            let Some(args) = args else {
                return Err(WorkspaceError::new(format!(
                    "Docker has no {view_id} view for {resource_kind} {resource_id}"
                )));
            };
            let output = cli
                .run(ProcessSpec::new("docker", &args))
                .await
                .map_err(|error| {
                    WorkspaceError::new(provider_cli_error(
                        PROVIDER_NAME,
                        &error,
                        &format!(
                            "Docker could not load {view_id} for {resource_kind} {resource_id}"
                        ),
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
    if state.eq_ignore_ascii_case("running") {
        ResourceState::Running
    } else if state.eq_ignore_ascii_case("exited") || state.eq_ignore_ascii_case("created") {
        ResourceState::Stopped
    } else if state.eq_ignore_ascii_case("paused") {
        ResourceState::Paused
    } else if state.eq_ignore_ascii_case("restarting") || state.eq_ignore_ascii_case("removing") {
        ResourceState::Transitioning
    } else if state.eq_ignore_ascii_case("dead") {
        ResourceState::Broken
    } else {
        ResourceState::Unknown
    }
}

/// The Interactive Shell Docker offers inside a container.
///
/// `docker exec` attaches only to a running container, so every other state
/// offers none. Plain `/bin/sh` is the shell that exists wherever any shell
/// does, including the minimal images Docker containers are so often built
/// from; reaching for a login shell instead would fail on exactly those.
fn container_shell(state: ResourceState, resource_id: &str) -> Option<InteractiveShellProcess> {
    (state == ResourceState::Running)
        .then(|| InteractiveShellProcess::new("docker", &["exec", "-it", resource_id, "/bin/sh"]))
}

fn discovery_with_error(message: impl Into<String>) -> ProviderDiscovery {
    let message = message.into();
    ProviderDiscovery::new(
        Provider::new(ProviderId::new(PROVIDER_ID), PROVIDER_NAME, None, None),
        Some(WorkspaceError::with_help(
            message,
            "Run `docker context show` to verify the selected context and ensure Docker is running.",
        )),
    )
}

/// A failed listing, carrying help only where it applies.
///
/// A Docker that is gone or would not start cannot answer another listing, so
/// suggesting one would send the user nowhere.
fn refresh_failure(error: ProcessError, fallback: &str, help: &str) -> WorkspaceError {
    let message = provider_cli_error(PROVIDER_NAME, &error, fallback);
    match error {
        ProcessError::Exited(_) => refresh_error(message, help),
        _ => WorkspaceError::new(message),
    }
}

fn refresh_error(message: impl AsRef<str>, help: &str) -> WorkspaceError {
    WorkspaceError::with_help(message, help)
}
