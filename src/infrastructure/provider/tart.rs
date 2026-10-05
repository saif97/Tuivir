use std::{future::Future, pin::Pin};

use serde::Deserialize;

use super::{
    DetailView, DetailViewId, Provider, ProviderDiscovery, ProviderId, ProviderVersion,
    ProviderWorkspace, Resource, ResourceCommand, ResourceDetails, ResourceId, ResourcePanel,
    ResourcePanelId, ResourceState, ResourceTarget, WorkspaceError, WorkspaceSnapshot,
    provider_cli_error, require_resource_state,
};
use crate::{
    application::ResourceShellProcess,
    infrastructure::process::{CliRunner, ProcessError, ProcessSpec},
};

pub struct TartWorkspace;

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct VmRow {
    source: String,
    name: String,
    disk: Option<u64>,
    size: u64,
    accessed: String,
    state: Option<String>,
    running: Option<bool>,
}

fn resource_state(state: &str) -> ResourceState {
    match state.to_ascii_lowercase().as_str() {
        "running" => ResourceState::Running,
        "stopped" => ResourceState::Stopped,
        "suspended" => ResourceState::Paused,
        _ => ResourceState::Unknown,
    }
}

fn vm_commands(state: ResourceState) -> &'static [ResourceCommand] {
    match state {
        ResourceState::Running => &[ResourceCommand::Stop, ResourceCommand::Delete],
        ResourceState::Stopped => &[ResourceCommand::Start, ResourceCommand::Delete],
        ResourceState::Paused => &[ResourceCommand::Resume, ResourceCommand::Delete],
        _ => &[ResourceCommand::Delete],
    }
}

fn listing_error(message: impl AsRef<str>) -> WorkspaceError {
    WorkspaceError::with_help(
        message,
        "Run `tart list --format json` to verify access to Tart's local storage.",
    )
}

impl ProviderWorkspace for TartWorkspace {
    fn id(&self) -> ProviderId {
        ProviderId::new("tart")
    }

    fn discover<'a>(
        &'a self,
        cli: &'a dyn CliRunner,
    ) -> Pin<Box<dyn Future<Output = Option<ProviderDiscovery>> + Send + 'a>> {
        Box::pin(async move {
            let (version, error) = match cli.run(ProcessSpec::new("tart", &["--version"])).await {
                Ok(output) => (Some(ProviderVersion::new(output.stdout.trim())), None),
                Err(ProcessError::ExecutableNotFound) => return None,
                Err(error) => (
                    None,
                    Some(WorkspaceError::with_help(
                        provider_cli_error("Tart", &error, "Tart could not report its version"),
                        "Run `tart --version` to verify the Tart installation.",
                    )),
                ),
            };
            Some(ProviderDiscovery::new(
                Provider::new(self.id(), "Tart", None, version),
                error,
            ))
        })
    }

    fn refresh<'a>(
        &'a self,
        cli: &'a dyn CliRunner,
    ) -> Pin<Box<dyn Future<Output = Result<WorkspaceSnapshot, WorkspaceError>> + Send + 'a>> {
        Box::pin(async move {
            let output = cli
                .run(ProcessSpec::new("tart", &["list", "--format", "json"]))
                .await
                .map_err(|error| {
                    let message = provider_cli_error("Tart", &error, "Tart could not list VMs");
                    match error {
                        ProcessError::Exited(_) => listing_error(message),
                        _ => WorkspaceError::new(message),
                    }
                })?;
            let rows: Vec<VmRow> = serde_json::from_str(&output.stdout)
                .map_err(|error| listing_error(format!("Tart returned malformed data: {error}")))?;
            let mut vms = ResourcePanel {
                id: ResourcePanelId::new("vms"),
                title: "VMs".into(),
                detail_views: vec![DetailView::new("config", "Config")],
                resources: vec![],
            };
            let mut images = ResourcePanel {
                id: ResourcePanelId::new("images"),
                title: "Images".into(),
                detail_views: vec![DetailView::from_snapshot("info", "Info")],
                resources: vec![],
            };
            for row in rows {
                let local = row.source.eq_ignore_ascii_case("local");
                if !local && !row.source.eq_ignore_ascii_case("oci") {
                    return Err(listing_error(format!(
                        "Tart returned an unsupported VM source: {}",
                        row.source
                    )));
                }
                let mut fields = vec![];
                if let Some(disk) = row.disk {
                    fields.push(("Disk", format!("{disk} GB")));
                }
                fields.push(("Size", format!("{} GB", row.size)));
                fields.push(("Accessed", row.accessed));
                let snapshot_details = if local {
                    vec![]
                } else {
                    let mut lines = vec![
                        format!("Name: {}", row.name),
                        format!("Source: {}", row.source),
                    ];
                    lines.extend(
                        fields
                            .iter()
                            .map(|(label, value)| format!("{label}: {value}")),
                    );
                    vec![(
                        DetailViewId::new("info"),
                        ResourceDetails::from_lines(lines),
                    )]
                };
                // Older Tart releases expose only Running. An explicit State
                // always wins, including future states we cannot interpret.
                let status = row.state.unwrap_or_else(|| match row.running {
                    Some(true) => "running".into(),
                    Some(false) => "stopped".into(),
                    None => "unknown".into(),
                });
                let state = resource_state(&status);
                // `exec` requires macOS 14+ and the Tart Guest Agent in the
                // guest. Tart reports connection failures in the session.
                let shell = (local && state == ResourceState::Running).then(|| {
                    ResourceShellProcess::new(
                        "tart",
                        &["exec", "-i", "-t", "--", &row.name, "/bin/sh"],
                    )
                });
                let resource = Resource {
                    id: ResourceId::new(&row.name),
                    name: row.name,
                    secondary_text: None,
                    status: local.then_some(status),
                    state: local.then_some(state),
                    fields,
                    snapshot_details,
                    available_commands: if local {
                        vm_commands(state)
                    } else {
                        &[ResourceCommand::Delete]
                    },
                    shell,
                };
                if local {
                    vms.resources.push(resource);
                } else {
                    images.resources.push(resource);
                }
            }
            Ok(WorkspaceSnapshot {
                panels: vec![vms, images],
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
            let name = target.resource_id().0.as_str();
            match target.panel_id().0.as_str() {
                "vms" => {
                    let state =
                        require_resource_state(state, "Tart", "VM", command, target.resource_id())?;
                    if !vm_commands(state).contains(&command) {
                        return Err(WorkspaceError::new(format!(
                            "Tart cannot {command} VM {name} in Resource State {state:?}"
                        )));
                    }
                    if command == ResourceCommand::Delete && state != ResourceState::Stopped {
                        run_command(
                            cli,
                            ProcessSpec::new("tart", &["stop", "--", name]),
                            ResourceCommand::Stop,
                            name,
                        )
                        .await?;
                    }
                }
                "images" if command == ResourceCommand::Delete => {}
                _ => {
                    return Err(WorkspaceError::new(format!(
                        "Tart has no {command} command for Resource Panel {}",
                        target.panel_id()
                    )));
                }
            }
            let native_command = match command {
                ResourceCommand::Start | ResourceCommand::Resume => {
                    return run_command(
                        cli,
                        ProcessSpec::background("tart", &["run", "--no-graphics", "--", name]),
                        command,
                        name,
                    )
                    .await;
                }
                ResourceCommand::Stop => "stop",
                ResourceCommand::Delete => "delete",
                _ => {
                    return Err(WorkspaceError::new(format!(
                        "Tart cannot {command} VM {name}"
                    )));
                }
            };
            run_command(
                cli,
                ProcessSpec::new("tart", &[native_command, "--", name]),
                command,
                name,
            )
            .await
        })
    }

    fn load_details<'a>(
        &'a self,
        cli: &'a dyn CliRunner,
        target: &'a ResourceTarget,
        view_id: &'a DetailViewId,
    ) -> Pin<Box<dyn Future<Output = Result<ResourceDetails, WorkspaceError>> + Send + 'a>> {
        Box::pin(async move {
            match (target.panel_id().0.as_str(), view_id.0.as_str()) {
                ("images", "info") => Ok(ResourceDetails::default()),
                ("vms", "config") => {
                    let output = cli
                        .run(ProcessSpec::new(
                            "tart",
                            &["get", "--format", "json", "--", &target.resource_id().0],
                        ))
                        .await
                        .map_err(|error| {
                            WorkspaceError::new(provider_cli_error(
                                "Tart",
                                &error,
                                "Tart could not load the VM configuration",
                            ))
                        })?;
                    let value: serde_json::Map<String, serde_json::Value> =
                        serde_json::from_str(&output.stdout).map_err(|error| {
                            WorkspaceError::new(format!(
                                "Tart returned malformed configuration data: {error}"
                            ))
                        })?;
                    Ok(ResourceDetails::from_output(
                        &serde_json::to_string_pretty(&value).expect("JSON values serialize"),
                    ))
                }
                _ => Err(WorkspaceError::new(format!(
                    "Tart has no {view_id} view for Resource Panel {}",
                    target.panel_id()
                ))),
            }
        })
    }
}

async fn run_command(
    cli: &dyn CliRunner,
    process: ProcessSpec,
    command: ResourceCommand,
    name: &str,
) -> Result<(), WorkspaceError> {
    cli.run(process).await.map_err(|error| {
        WorkspaceError::new(provider_cli_error(
            "Tart",
            &error,
            &format!("Tart could not {command} Resource {name}"),
        ))
    })?;
    Ok(())
}
