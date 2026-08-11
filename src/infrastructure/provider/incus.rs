use std::{future::Future, pin::Pin};

use serde::Deserialize;

use crate::{
    application::{InteractiveShellProcess, LifecycleCommandPolicy, lifecycle_commands},
    infrastructure::process::{CliRunner, ProcessError, ProcessSpec},
    infrastructure::provider::{
        DetailView, DetailViewId, Provider, ProviderDiscovery, ProviderId, ProviderWorkspace,
        Resource, ResourceCommand, ResourceDetails, ResourceId, ResourcePanel, ResourcePanelId,
        ResourceState, ResourceTarget, TargetEnvironment, WorkspaceError, WorkspaceSnapshot,
        provider_cli_error, require_resource_state,
    },
};

const PROVIDER_ID: &str = "incus";
const PROVIDER_NAME: &str = "Incus";
/// What a user can run to check the Target Environment a refresh could not read.
const REFRESH_HELP: &str = "Run `incus list` to verify access to the current Target Environment.";
const VOLUME_REFRESH_HELP: &str =
    "Run `incus storage volume list <pool>` to verify access to custom Volumes.";
const INSTANCES_PANEL_ID: &str = "instances";
const VOLUMES_PANEL_ID: &str = "volumes";
const INFO_VIEW_ID: &str = "info";
const CONFIG_VIEW_ID: &str = "config";
const CONSOLE_LOG_VIEW_ID: &str = "console-log";

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

#[derive(Deserialize)]
struct StoragePoolRow {
    name: String,
}

#[derive(Deserialize)]
struct StorageVolumeRow {
    name: String,
    #[serde(rename = "type")]
    volume_type: String,
    content_type: String,
    description: String,
    project: String,
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
            let (remote, project) = tokio::join!(
                cli.run(ProcessSpec::new("incus", &["remote", "get-default"])),
                cli.run(ProcessSpec::new("incus", &["project", "get-current"])),
            );
            let remote = match remote {
                Err(ProcessError::ExecutableNotFound) => return None,
                Err(error) => {
                    return Some(discovery_error(
                        provider_cli_error(
                            PROVIDER_NAME,
                            &error,
                            "Incus could not report its default remote",
                        ),
                        "incus remote get-default",
                    ));
                }
                Ok(output) => output.stdout.trim().to_owned(),
            };
            let project = match project {
                Err(error) => {
                    return Some(discovery_error(
                        provider_cli_error(
                            PROVIDER_NAME,
                            &error,
                            "Incus could not report the current project",
                        ),
                        "incus project get-current",
                    ));
                }
                Ok(output) => output.stdout.trim().to_owned(),
            };

            Some(ProviderDiscovery::new(
                Provider::new(
                    self.id(),
                    PROVIDER_NAME,
                    Some(TargetEnvironment::new(format!("{remote} / {project}"))),
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
            let (output, pools_output) = tokio::try_join!(
                async {
                    cli.run(ProcessSpec::new("incus", &["list", "--format=json"]))
                        .await
                        .map_err(refresh_failure)
                },
                async {
                    cli.run(ProcessSpec::new(
                        "incus",
                        &["storage", "list", "--format=json"],
                    ))
                    .await
                    .map_err(|error| {
                        volume_refresh_failure(error, "Incus could not list storage pools")
                    })
                }
            )?;
            let rows: Vec<InstanceRow> = serde_json::from_str(&output.stdout)
                .map_err(|error| WorkspaceError::new(error.to_string()))?;
            let resources = rows
                .into_iter()
                .map(|row| {
                    let state = incus_resource_state(&row.status);
                    let available_commands =
                        lifecycle_commands(state, LifecycleCommandPolicy::RestartAndResume);
                    let shell = instance_shell(state, &row.name);
                    Resource {
                        id: ResourceId::new(&row.name),
                        name: row.name,
                        secondary_text: None,
                        status: Some(row.status),
                        state: Some(state),
                        fields: vec![
                            ("Type", row.instance_type),
                            ("Architecture", row.architecture),
                            ("Location", row.location),
                        ],
                        snapshot_details: Vec::new(),
                        available_commands,
                        shell,
                    }
                })
                .collect();

            let pools: Vec<StoragePoolRow> =
                serde_json::from_str(&pools_output.stdout).map_err(|error| {
                    volume_refresh_error(format!(
                        "Incus returned malformed storage pool data: {error}"
                    ))
                })?;
            let mut volume_resources = Vec::new();
            for pool in pools {
                let output = cli
                    .run(ProcessSpec::new(
                        "incus",
                        &[
                            "storage",
                            "volume",
                            "list",
                            pool.name.as_str(),
                            "type=custom",
                            "--format=json",
                        ],
                    ))
                    .await
                    .map_err(|error| {
                        volume_refresh_failure(
                            error,
                            &format!("Incus could not list custom Volumes in pool {}", pool.name),
                        )
                    })?;
                let rows: Vec<StorageVolumeRow> =
                    serde_json::from_str(&output.stdout).map_err(|error| {
                        volume_refresh_error(format!(
                            "Incus returned malformed Volume data for pool {}: {error}",
                            pool.name
                        ))
                    })?;
                volume_resources.extend(rows.into_iter().filter_map(|row| {
                    if row.volume_type != "custom" || row.name.contains('/') {
                        return None;
                    }
                    Some(Resource {
                        id: ResourceId::new(format!("{}/{}", pool.name, row.name)),
                        name: row.name,
                        secondary_text: Some(pool.name.clone()),
                        status: None,
                        state: None,
                        fields: vec![
                            ("Pool", pool.name.clone()),
                            ("Content Type", row.content_type),
                            ("Description", row.description),
                            ("Project", row.project),
                            ("Location", row.location),
                        ],
                        snapshot_details: Vec::new(),
                        available_commands: &[ResourceCommand::Delete],
                        shell: None,
                    })
                }));
            }
            volume_resources.sort_by(|left, right| {
                left.secondary_text
                    .cmp(&right.secondary_text)
                    .then_with(|| left.name.cmp(&right.name))
            });

            Ok(WorkspaceSnapshot {
                panels: vec![
                    ResourcePanel {
                        id: ResourcePanelId::new(INSTANCES_PANEL_ID),
                        title: "Instances".to_owned(),
                        detail_views: instance_detail_views(),
                        resources,
                    },
                    ResourcePanel {
                        id: ResourcePanelId::new(VOLUMES_PANEL_ID),
                        title: "Volumes".to_owned(),
                        detail_views: vec![
                            DetailView::new(INFO_VIEW_ID, "Info"),
                            DetailView::new(CONFIG_VIEW_ID, "Config"),
                        ],
                        resources: volume_resources,
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
        state: Option<ResourceState>,
    ) -> Pin<Box<dyn Future<Output = Result<(), WorkspaceError>> + Send + 'a>> {
        Box::pin(async move {
            let panel_id = target.panel_id();
            let resource_id = target.resource_id();
            if panel_id.0 == VOLUMES_PANEL_ID && command == ResourceCommand::Delete {
                let (pool, name) = volume_address(resource_id)?;
                cli.run(ProcessSpec::new(
                    "incus",
                    &["storage", "volume", "delete", pool, name],
                ))
                .await
                .map_err(|error| {
                    WorkspaceError::new(provider_cli_error(
                        PROVIDER_NAME,
                        &error,
                        &format!("Incus could not delete Volume {name} in pool {pool}"),
                    ))
                })?;
                return Ok(());
            }
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
            let state =
                require_resource_state(state, PROVIDER_NAME, "instance", command, resource_id)?;
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
            if panel_id.0 == VOLUMES_PANEL_ID {
                let (pool, name) = volume_address(resource_id)?;
                let operation = match view_id.0.as_str() {
                    INFO_VIEW_ID => "info",
                    CONFIG_VIEW_ID => "show",
                    _ => {
                        return Err(WorkspaceError::new(format!(
                            "Incus has no {view_id} view for Volume {name} in pool {pool}"
                        )));
                    }
                };
                let output = cli
                    .run(ProcessSpec::new(
                        "incus",
                        &["storage", "volume", operation, pool, name],
                    ))
                    .await
                    .map_err(|error| {
                        WorkspaceError::new(provider_cli_error(
                            PROVIDER_NAME,
                            &error,
                            &format!(
                                "Incus could not load {view_id} for Volume {name} in pool {pool}"
                            ),
                        ))
                    })?;
                return Ok(ResourceDetails::from_output(&output.stdout));
            }
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
        INFO_VIEW_ID => Some(vec!["info", resource_id]),
        CONFIG_VIEW_ID => Some(vec!["config", "show", resource_id]),
        CONSOLE_LOG_VIEW_ID => Some(vec!["console", "--show-log", resource_id]),
        _ => None,
    }
}

/// The views Incus itself offers for an instance, in the order the user moves
/// through them.
fn instance_detail_views() -> Vec<DetailView> {
    vec![
        DetailView::new(INFO_VIEW_ID, "Info"),
        DetailView::new(CONFIG_VIEW_ID, "Config"),
        DetailView::new(CONSOLE_LOG_VIEW_ID, "Console Log"),
    ]
}

fn volume_address(resource_id: &ResourceId) -> Result<(&str, &str), WorkspaceError> {
    resource_id
        .0
        .split_once('/')
        .filter(|(pool, name)| !pool.is_empty() && !name.is_empty())
        .ok_or_else(|| {
            WorkspaceError::new(format!(
                "Incus Volume identity {resource_id} does not contain a pool and name"
            ))
        })
}

/// Maps an Incus instance status onto the shared vocabulary.
///
/// Incus reports settled statuses (`Running`, `Stopped`, `Frozen`, `Error`)
/// alongside the transitional ones an operation passes through; anything else
/// is deliberately left `Unknown` rather than assumed to be stopped.
fn incus_resource_state(status: &str) -> ResourceState {
    if status.eq_ignore_ascii_case("running") {
        ResourceState::Running
    } else if status.eq_ignore_ascii_case("stopped") {
        ResourceState::Stopped
    } else if status.eq_ignore_ascii_case("frozen") {
        ResourceState::Paused
    } else if ["starting", "stopping", "freezing", "thawing"]
        .iter()
        .any(|transition| status.eq_ignore_ascii_case(transition))
    {
        ResourceState::Transitioning
    } else if status.eq_ignore_ascii_case("error") {
        ResourceState::Broken
    } else {
        ResourceState::Unknown
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
        Provider::new(ProviderId::new(PROVIDER_ID), PROVIDER_NAME, None, None),
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

fn volume_refresh_failure(error: ProcessError, fallback: &str) -> WorkspaceError {
    let message = provider_cli_error(PROVIDER_NAME, &error, fallback);
    match error {
        ProcessError::Exited(_) => volume_refresh_error(message),
        _ => WorkspaceError::new(message),
    }
}

fn volume_refresh_error(message: impl AsRef<str>) -> WorkspaceError {
    WorkspaceError::with_help(message, VOLUME_REFRESH_HELP)
}
