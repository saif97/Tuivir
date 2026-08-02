use std::{future::Future, pin::Pin};

use serde::Deserialize;

use crate::{
    application::InteractiveShellProcess,
    infrastructure::process::{CliRunner, ProcessError, ProcessSpec},
    provider::{
        DetailView, DetailViewId, Provider, ProviderDiscovery, ProviderId, ProviderWorkspace,
        Resource, ResourceCommand, ResourceDetails, ResourceId, ResourcePanel, ResourcePanelId,
        ResourceState, ResourceTarget, TargetEnvironment, WorkspaceError, WorkspaceSnapshot,
        provider_cli_error,
    },
};

const PROVIDER_ID: &str = "incus";
const PROVIDER_NAME: &str = "Incus";
/// What a user can run to check the Target Environment a refresh could not read.
const REFRESH_HELP: &str = "Run `incus list` to verify access to the current Target Environment.";
const INSTANCES_PANEL_ID: &str = "instances";

pub struct IncusWorkspace;

#[derive(Deserialize)]
struct InstanceRow {
    name: String,
    status: String,
    #[serde(rename = "type")]
    instance_type: String,
    architecture: String,
    location: String,
}

impl ProviderWorkspace for IncusWorkspace {
    fn id(&self) -> ProviderId {
        ProviderId::new(PROVIDER_ID)
    }

    fn discover<'a>(
        &'a self,
        cli: &'a dyn CliRunner,
    ) -> Pin<Box<dyn Future<Output = Option<ProviderDiscovery>> + Send + 'a>> {
        Box::pin(async move {
            let remote = match cli
                .run(ProcessSpec::new("incus", &["remote", "get-default"]))
                .await
            {
                Err(ProcessError::ExecutableNotFound) => return None,
                Err(ProcessError::SpawnFailed(message)) => {
                    return Some(discovery_error(
                        format!("{PROVIDER_NAME} CLI could not be started: {message}"),
                        "incus remote get-default",
                    ));
                }
                Err(ProcessError::Exited(failure)) => {
                    return Some(discovery_error(
                        failure.message_or("Incus could not report its default remote"),
                        "incus remote get-default",
                    ));
                }
                Ok(output) => output.stdout.trim().to_owned(),
            };
            let project = match cli
                .run(ProcessSpec::new("incus", &["project", "get-current"]))
                .await
            {
                Err(ProcessError::ExecutableNotFound) => {
                    return Some(discovery_error(
                        format!("{PROVIDER_NAME} CLI is no longer available"),
                        "incus project get-current",
                    ));
                }
                Err(ProcessError::SpawnFailed(message)) => {
                    return Some(discovery_error(
                        format!("{PROVIDER_NAME} CLI could not be started: {message}"),
                        "incus project get-current",
                    ));
                }
                Err(ProcessError::Exited(failure)) => {
                    return Some(discovery_error(
                        failure.message_or("Incus could not report the current project"),
                        "incus project get-current",
                    ));
                }
                Ok(output) => output.stdout.trim().to_owned(),
            };

            Some(ProviderDiscovery::new(
                Provider::new(
                    self.id(),
                    PROVIDER_NAME,
                    TargetEnvironment::new(format!("{remote} / {project}")),
                    None,
                ),
                None,
            ))
        })
    }

    fn refresh<'a>(
        &'a self,
        cli: &'a dyn CliRunner,
    ) -> Pin<Box<dyn Future<Output = Result<WorkspaceSnapshot, WorkspaceError>> + Send + 'a>> {
        Box::pin(async move {
            let output = cli
                .run(ProcessSpec::new("incus", &["list", "--format=json"]))
                .await
                .map_err(refresh_failure)?;
            let rows: Vec<InstanceRow> = serde_json::from_str(&output.stdout)
                .map_err(|error| WorkspaceError::new(error.to_string()))?;
            let resources = rows
                .into_iter()
                .map(|row| {
                    let state = incus_resource_state(&row.status);
                    let available_commands = incus_commands(state);
                    let shell = instance_shell(state, &row.name);
                    Resource {
                        id: ResourceId::new(&row.name),
                        name: row.name,
                        status: Some(row.status),
                        state: Some(state),
                        fields: vec![
                            ("Type".to_owned(), row.instance_type),
                            ("Architecture".to_owned(), row.architecture),
                            ("Location".to_owned(), row.location),
                        ],
                        available_commands,
                        shell,
                    }
                })
                .collect();

            Ok(WorkspaceSnapshot {
                panels: vec![ResourcePanel {
                    id: ResourcePanelId::new(INSTANCES_PANEL_ID),
                    title: "Instances".to_owned(),
                    detail_views: instance_detail_views(),
                    resources,
                }],
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
            if panel_id.0 != INSTANCES_PANEL_ID {
                return Err(WorkspaceError::new(format!(
                    "Incus has no {command} command for Resource Panel {panel_id}"
                )));
            }
            let verb = match command {
                ResourceCommand::Start => "start",
                ResourceCommand::Stop => "stop",
                ResourceCommand::Restart => "restart",
                ResourceCommand::Resume => "unfreeze",
                ResourceCommand::Delete => "delete",
            };
            let mut args = vec![verb];
            // Incus deletes an instance plainly only from a stopped state; a
            // running or frozen one needs the force the user already confirmed.
            if command == ResourceCommand::Delete && state != ResourceState::Stopped {
                args.push("--force");
            }
            args.push(resource_id.0.as_str());
            cli.run(ProcessSpec::new("incus", &args))
                .await
                .map_err(|error| {
                    WorkspaceError::new(provider_cli_error(
                        PROVIDER_NAME,
                        &error,
                        &format!("Incus could not {command} instance {resource_id}"),
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
            if panel_id.0 != INSTANCES_PANEL_ID {
                return Err(WorkspaceError::new(format!(
                    "Incus has no {view_id} view for Resource Panel {panel_id}"
                )));
            }
            let Some(args) = instance_detail_command(view_id, resource_id.0.as_str()) else {
                return Err(WorkspaceError::new(format!(
                    "Incus has no {view_id} view for instance {resource_id}"
                )));
            };
            let output = cli
                .run(ProcessSpec::new("incus", &args))
                .await
                .map_err(|error| {
                    WorkspaceError::new(provider_cli_error(
                        PROVIDER_NAME,
                        &error,
                        &format!("Incus could not load {view_id} for instance {resource_id}"),
                    ))
                })?;
            Ok(ResourceDetails::from_output(&output.stdout))
        })
    }
}

/// The Incus command behind each declared view, or `None` for a view this
/// workspace never declared.
fn instance_detail_command<'a>(
    view_id: &DetailViewId,
    resource_id: &'a str,
) -> Option<Vec<&'a str>> {
    match view_id.0.as_str() {
        "info" => Some(vec!["info", resource_id]),
        "config" => Some(vec!["config", "show", resource_id]),
        "console-log" => Some(vec!["console", "--show-log", resource_id]),
        _ => None,
    }
}

/// The views Incus itself offers for an instance, in the order the user moves
/// through them.
fn instance_detail_views() -> Vec<DetailView> {
    vec![
        DetailView::new("info", "Info"),
        DetailView::new("config", "Config"),
        DetailView::new("console-log", "Console Log"),
    ]
}

/// Maps an Incus instance status onto the shared vocabulary.
///
/// Incus reports settled statuses (`Running`, `Stopped`, `Frozen`, `Error`)
/// alongside the transitional ones an operation passes through; anything else
/// is deliberately left `Unknown` rather than assumed to be stopped.
fn incus_resource_state(status: &str) -> ResourceState {
    match status.to_ascii_lowercase().as_str() {
        "running" => ResourceState::Running,
        "stopped" => ResourceState::Stopped,
        "frozen" => ResourceState::Paused,
        "starting" | "stopping" | "freezing" | "thawing" => ResourceState::Transitioning,
        "error" => ResourceState::Broken,
        _ => ResourceState::Unknown,
    }
}

fn incus_commands(state: ResourceState) -> Vec<ResourceCommand> {
    match state {
        ResourceState::Running => vec![
            ResourceCommand::Stop,
            ResourceCommand::Restart,
            ResourceCommand::Delete,
        ],
        ResourceState::Stopped => vec![ResourceCommand::Start, ResourceCommand::Delete],
        // A frozen instance resumes rather than starts: `incus start` fails
        // against it, and `incus unfreeze` fails against everything else.
        ResourceState::Paused => vec![ResourceCommand::Resume, ResourceCommand::Delete],
        // A transitioning, errored, or unrecognised instance has no lifecycle
        // Command that reliably applies. Deletion always does.
        ResourceState::Transitioning | ResourceState::Broken | ResourceState::Unknown => {
            vec![ResourceCommand::Delete]
        }
    }
}

/// The Interactive Shell Incus offers inside an instance.
///
/// `incus exec` reaches only into a running instance, so every other state
/// offers none. An instance is a whole system rather than one packaged
/// process, so the shell worth giving the user is root's login shell: this is
/// what Incus's own `shell` alias expands to, spelled out here so it does not
/// depend on that alias surviving in the user's configuration.
fn instance_shell(state: ResourceState, name: &str) -> Option<InteractiveShellProcess> {
    (state == ResourceState::Running)
        .then(|| InteractiveShellProcess::new("incus", &["exec", name, "--", "su", "-l"]))
}

fn discovery_error(message: impl AsRef<str>, command: &str) -> ProviderDiscovery {
    ProviderDiscovery::new(
        Provider::new(
            ProviderId::new(PROVIDER_ID),
            PROVIDER_NAME,
            TargetEnvironment::new("unavailable"),
            None,
        ),
        Some(WorkspaceError::with_help(
            message,
            &format!("Run `{command}` to verify the selected Target Environment."),
        )),
    )
}

/// A failed listing, carrying help only where it applies.
///
/// An Incus that is gone or would not start cannot answer `incus list`, so
/// suggesting it would send the user nowhere.
fn refresh_failure(error: ProcessError) -> WorkspaceError {
    let message = provider_cli_error(PROVIDER_NAME, &error, "Incus could not list instances");
    match error {
        ProcessError::Exited(_) => refresh_error(message),
        _ => WorkspaceError::new(message),
    }
}

fn refresh_error(message: impl AsRef<str>) -> WorkspaceError {
    WorkspaceError::with_help(message, REFRESH_HELP)
}
