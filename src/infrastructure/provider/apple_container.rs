use std::{future::Future, pin::Pin};

use serde::{Deserialize, Serialize};

use super::{
    DetailView, DetailViewId, Provider, ProviderDiscovery, ProviderId, ProviderVersion,
    ProviderWorkspace, Resource, ResourceCommand, ResourceDetails, ResourceId, ResourcePanel,
    ResourcePanelId, ResourceState, ResourceTarget, WorkspaceError, WorkspaceSnapshot,
    provider_cli_error, require_resource_state,
};
use crate::application::{
    ConfigurationField, ConfigurationUpdate, FieldConstraint, ResourceConfiguration,
    ResourceShellProcess,
};
use crate::infrastructure::process::{CliRunner, ProcessError, ProcessSpec};

pub struct AppleContainerWorkspace;

const PROVIDER_NAME: &str = "Apple Container";
const RECOVERY: &str = "Run `container system start` outside Tuivir, then refresh the Workspace. Verify the CLI installation with `container system version --format json`; the tested baseline is CLI 1.5.0 on Apple Silicon with macOS 26.";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct VersionRow {
    app_name: String,
    version: String,
}

#[derive(Deserialize)]
struct ContainerRow {
    configuration: ContainerConfiguration,
    status: ContainerStatus,
}

#[derive(Deserialize)]
struct ContainerConfiguration {
    id: String,
    image: ContainerImage,
    resources: ContainerResources,
}

#[derive(Deserialize)]
struct ContainerImage {
    reference: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ContainerResources {
    cpus: u64,
    memory_in_bytes: u64,
}

#[derive(Deserialize)]
struct ContainerStatus {
    state: String,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct StatsRow {
    id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    memory_usage_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    memory_limit_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cpu_usage_usec: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    network_rx_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    network_tx_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    block_read_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    block_write_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    num_processes: Option<u64>,
    #[serde(flatten)]
    additional_fields: serde_json::Map<String, serde_json::Value>,
}

#[derive(Deserialize)]
struct ImageRow {
    configuration: ImageConfiguration,
    variants: Vec<ImageVariant>,
}

#[derive(Deserialize)]
struct ImageConfiguration {
    name: String,
    descriptor: ImageDescriptor,
}

#[derive(Deserialize)]
struct ImageDescriptor {
    digest: String,
}

#[derive(Deserialize)]
struct ImageVariant {
    size: u64,
    platform: ImagePlatform,
}

#[derive(Deserialize)]
struct ImagePlatform {
    os: String,
    architecture: String,
    variant: Option<String>,
}

fn image_resource(row: ImageRow) -> Resource {
    let platforms = row
        .variants
        .iter()
        .map(|variant| {
            let platform = &variant.platform;
            let mut name = format!("{}/{}", platform.os, platform.architecture);
            if let Some(variant) = &platform.variant {
                name.push('/');
                name.push_str(variant);
            }
            name
        })
        .collect::<Vec<_>>()
        .join(", ");
    let size: u64 = row.variants.iter().map(|variant| variant.size).sum();
    Resource {
        // A digest can have several references; Delete and Inspect operate on
        // the requested reference, so each must retain its own identity.
        id: ResourceId::new(&row.configuration.name),
        name: row.configuration.name,
        secondary_text: Some(row.configuration.descriptor.digest.clone()),
        status: None,
        state: None,
        fields: vec![
            ("Digest", row.configuration.descriptor.digest),
            ("Size", format!("{} MiB", size / (1024 * 1024))),
            ("Platforms", platforms),
        ],
        snapshot_details: vec![],
        available_commands: &[ResourceCommand::Delete],
        shell: None,
    }
}

#[derive(Deserialize)]
struct NetworkRow {
    configuration: NetworkConfiguration,
    status: NetworkStatus,
}

#[derive(Deserialize)]
struct NetworkConfiguration {
    name: String,
    mode: String,
    labels: std::collections::HashMap<String, String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NetworkStatus {
    ipv4_subnet: String,
    ipv4_gateway: String,
    ipv6_subnet: Option<String>,
}

fn network_resource(row: NetworkRow) -> Resource {
    let builtin = row
        .configuration
        .labels
        .get("com.apple.container.resource.role")
        .is_some_and(|role| role == "builtin");
    let mut fields = vec![
        ("Mode", row.configuration.mode),
        ("IPv4 Subnet", row.status.ipv4_subnet),
        ("IPv4 Gateway", row.status.ipv4_gateway),
        ("Built-in", builtin.to_string()),
    ];
    if let Some(subnet) = row.status.ipv6_subnet {
        fields.push(("IPv6 Subnet", subnet));
    }
    Resource {
        id: ResourceId::new(&row.configuration.name),
        name: row.configuration.name,
        secondary_text: None,
        status: None,
        state: None,
        fields,
        snapshot_details: vec![],
        available_commands: if builtin {
            &[]
        } else {
            &[ResourceCommand::Delete]
        },
        shell: None,
    }
}

#[derive(Deserialize)]
struct VolumeRow {
    configuration: VolumeConfiguration,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct VolumeConfiguration {
    name: String,
    driver: String,
    format: String,
    source: String,
    size_in_bytes: Option<u64>,
}

enum NativeRows {
    Containers(Vec<ContainerRow>),
    Images(Vec<ImageRow>),
    Networks(Vec<NetworkRow>),
    Volumes(Vec<VolumeRow>),
}

impl NativeRows {
    fn parse<'de>(
        panel_id: &str,
        deserializer: impl serde::Deserializer<'de>,
    ) -> Result<Self, WorkspaceError> {
        let rows = match panel_id {
            "containers" => Vec::<ContainerRow>::deserialize(deserializer).map(Self::Containers),
            "images" => Vec::<ImageRow>::deserialize(deserializer).map(Self::Images),
            "networks" => Vec::<NetworkRow>::deserialize(deserializer).map(Self::Networks),
            "volumes" => Vec::<VolumeRow>::deserialize(deserializer).map(Self::Volumes),
            _ => {
                return Err(WorkspaceError::new(format!(
                    "Apple Container has no Resource Panel {panel_id}"
                )));
            }
        };
        rows.map_err(malformed)
    }

    fn into_resources(self) -> Vec<Resource> {
        match self {
            Self::Containers(rows) => rows.into_iter().map(container_resource).collect(),
            Self::Images(rows) => rows.into_iter().map(image_resource).collect(),
            Self::Networks(rows) => rows.into_iter().map(network_resource).collect(),
            Self::Volumes(rows) => rows.into_iter().map(volume_resource).collect(),
        }
    }
}

fn volume_resource(row: VolumeRow) -> Resource {
    let mut fields = vec![
        ("Driver", row.configuration.driver),
        ("Format", row.configuration.format),
        ("Source", row.configuration.source),
    ];
    if let Some(size) = row.configuration.size_in_bytes {
        fields.push(("Size", format!("{} MiB", size / (1024 * 1024))));
    }
    Resource {
        id: ResourceId::new(&row.configuration.name),
        name: row.configuration.name,
        secondary_text: None,
        status: None,
        state: None,
        fields,
        snapshot_details: vec![],
        available_commands: &[ResourceCommand::Delete],
        shell: None,
    }
}

fn container_commands(state: ResourceState) -> &'static [ResourceCommand] {
    match state {
        ResourceState::Running => &[
            ResourceCommand::Stop,
            ResourceCommand::Restart,
            ResourceCommand::Delete,
        ],
        ResourceState::Stopped => &[ResourceCommand::Start, ResourceCommand::Delete],
        _ => &[ResourceCommand::Delete],
    }
}

fn container_state(state: &str) -> ResourceState {
    match state {
        "running" => ResourceState::Running,
        "stopped" => ResourceState::Stopped,
        "stopping" => ResourceState::Transitioning,
        _ => ResourceState::Unknown,
    }
}

fn container_resource(row: ContainerRow) -> Resource {
    let state = container_state(&row.status.state);
    let shell = (state == ResourceState::Running).then(|| {
        ResourceShellProcess::new(
            "container",
            &[
                "exec",
                "--interactive",
                "--tty",
                "--",
                &row.configuration.id,
                "/bin/sh",
            ],
        )
    });
    Resource {
        id: ResourceId::new(&row.configuration.id),
        name: row.configuration.id,
        secondary_text: Some(row.configuration.image.reference.clone()),
        status: Some(row.status.state),
        state: Some(state),
        fields: vec![
            ("Image", row.configuration.image.reference),
            ("CPUs", row.configuration.resources.cpus.to_string()),
            (
                "Memory",
                format!(
                    "{} MiB",
                    row.configuration.resources.memory_in_bytes / (1024 * 1024)
                ),
            ),
        ],
        snapshot_details: vec![],
        available_commands: container_commands(state),
        shell,
    }
}

fn malformed(error: impl std::fmt::Display) -> WorkspaceError {
    WorkspaceError::with_help(
        format!("Apple Container returned malformed or incompatible data: {error}"),
        RECOVERY,
    )
}

impl ProviderWorkspace for AppleContainerWorkspace {
    fn load_configuration<'a>(
        &'a self,
        cli: &'a dyn CliRunner,
        target: &'a ResourceTarget,
    ) -> Pin<Box<dyn Future<Output = Result<ResourceConfiguration, WorkspaceError>> + Send + 'a>>
    {
        Box::pin(async move {
            if target.panel_id().0 != "containers" {
                return Err(WorkspaceError::new(
                    "Apple Container configuration is only available for Containers",
                ));
            }
            let output = cli
                .run(ProcessSpec::new(
                    "container",
                    &["inspect", "--", &target.resource_id().0],
                ))
                .await
                .map_err(|error| {
                    WorkspaceError::new(provider_cli_error(
                        PROVIDER_NAME,
                        &error,
                        "Apple Container could not load Container configuration",
                    ))
                })?;
            let rows: Vec<ContainerRow> =
                serde_json::from_str(&output.stdout).map_err(malformed)?;
            let row = rows
                .into_iter()
                .find(|row| row.configuration.id == target.resource_id().0)
                .ok_or_else(|| {
                    WorkspaceError::new("Apple Container did not report the selected Container")
                })?;
            Ok(ResourceConfiguration {
                fields: vec![
                    ConfigurationField { id: "cpu".into(), label: "CPU count".into(), value: row.configuration.resources.cpus.to_string(), constraint: FieldConstraint::ReadOnly, update: ConfigurationUpdate::Live },
                    ConfigurationField { id: "memory".into(), label: "Memory (bytes)".into(), value: row.configuration.resources.memory_in_bytes.to_string(), constraint: FieldConstraint::ReadOnly, update: ConfigurationUpdate::Live },
                ],
                state: container_state(&row.status.state),
                notice: "Apple Container: in-place CPU/memory resize is unsupported by the CLI; values are read-only. Set resources when creating a Container outside Tuivir.".into(),
                stop_preserves_resource: true,
            })
        })
    }

    fn id(&self) -> ProviderId {
        ProviderId::new("apple-container")
    }

    fn discover<'a>(
        &'a self,
        cli: &'a dyn CliRunner,
    ) -> Pin<Box<dyn Future<Output = Option<ProviderDiscovery>> + Send + 'a>> {
        Box::pin(async move {
            let result = cli
                .run(ProcessSpec::new(
                    "container",
                    &["system", "version", "--format", "json"],
                ))
                .await;
            let (version, error) = match result {
                Err(ProcessError::ExecutableNotFound) => return None,
                Err(error) => (
                    None,
                    Some(WorkspaceError::with_help(
                        provider_cli_error(
                            PROVIDER_NAME,
                            &error,
                            "Apple Container could not report its version",
                        ),
                        RECOVERY,
                    )),
                ),
                Ok(output) => match serde_json::from_str::<Vec<VersionRow>>(&output.stdout) {
                    Ok(rows) => match rows.into_iter().find(|row| row.app_name == "container") {
                        Some(row) if !row.version.trim().is_empty() => {
                            (Some(ProviderVersion::new(row.version)), None)
                        }
                        _ => (None, Some(malformed("missing container CLI version"))),
                    },
                    Err(error) => (None, Some(malformed(error))),
                },
            };
            Some(ProviderDiscovery::new(
                Provider::new(self.id(), PROVIDER_NAME, None, version),
                error,
            ))
        })
    }

    fn refresh<'a>(
        &'a self,
        cli: &'a dyn CliRunner,
    ) -> Pin<Box<dyn Future<Output = Result<WorkspaceSnapshot, WorkspaceError>> + Send + 'a>> {
        Box::pin(async move {
            // Containers establish availability first. The remaining native
            // listings are independent, so their CLI work can overlap.
            let containers = load_panel(
                cli,
                "containers",
                "Containers",
                &["list", "--all", "--format", "json"],
            )
            .await?;
            let (images, networks, volumes) = tokio::join!(
                load_panel(
                    cli,
                    "images",
                    "Images",
                    &["image", "list", "--format", "json"]
                ),
                load_panel(
                    cli,
                    "networks",
                    "Networks",
                    &["network", "list", "--format", "json"]
                ),
                load_panel(
                    cli,
                    "volumes",
                    "Volumes",
                    &["volume", "list", "--format", "json"]
                ),
            );
            Ok(WorkspaceSnapshot {
                panels: vec![containers, images?, networks?, volumes?],
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
            if target.panel_id().0 == "images" && command == ResourceCommand::Delete {
                run_command(cli, &["image", "delete", "--", name], command, name).await?;
                // Both native list and inspect filter infrastructure images.
                // Inspect resolves the reference first: a successful empty
                // array means retention, never absence. Only the exact native
                // missing-reference diagnostic can prove deletion.
                return match cli
                    .run(ProcessSpec::new(
                        "container",
                        &["image", "inspect", "--", name],
                    ))
                    .await
                {
                    Err(ProcessError::Exited(failure))
                        if failure.exit_code == Some(1)
                            && failure.stdout.trim().is_empty()
                            && failure.stderr.trim()
                                == format!("Error: image not found: {name}") =>
                    {
                        Ok(())
                    }
                    Err(error) => Err(WorkspaceError::new(format!(
                        "Apple Container image deletion could not be verified for {name}: {}",
                        provider_cli_error(PROVIDER_NAME, &error, "inspection failed")
                    ))),
                    Ok(output) => {
                        match serde_json::from_str::<Vec<serde_json::Value>>(&output.stdout) {
                            Ok(_) => Err(WorkspaceError::new(format!(
                                "Apple Container retained image reference {name}; configured infrastructure images cannot be deleted"
                            ))),
                            Err(error) => Err(WorkspaceError::new(format!(
                                "Apple Container image deletion could not be verified for {name}: malformed inspection data: {error}"
                            ))),
                        }
                    }
                };
            }
            if command == ResourceCommand::Delete {
                let kind = match target.panel_id().0.as_str() {
                    "networks" => Some("network"),
                    "volumes" => Some("volume"),
                    _ => None,
                };
                if let Some(kind) = kind {
                    return run_command(cli, &[kind, "delete", "--", name], command, name).await;
                }
            }
            if target.panel_id().0 != "containers" {
                return Err(WorkspaceError::new(format!(
                    "Apple Container has no {command} Command for Resource Panel {}",
                    target.panel_id()
                )));
            }
            let state = require_resource_state(
                state,
                PROVIDER_NAME,
                "Container",
                command,
                target.resource_id(),
            )?;
            if !container_commands(state).contains(&command) {
                return Err(WorkspaceError::new(format!(
                    "Apple Container cannot {command} Container {name} in Resource State {state:?}"
                )));
            }
            let verb = match command {
                ResourceCommand::Delete => {
                    let args = if state == ResourceState::Stopped {
                        vec!["delete", "--", name]
                    } else {
                        vec!["delete", "--force", "--", name]
                    };
                    return run_command(cli, &args, command, name).await;
                }
                ResourceCommand::Restart => {
                    run_command(cli, &["stop", "--", name], ResourceCommand::Stop, name)
                        .await
                        .map_err(|error| {
                            WorkspaceError::new(format!(
                                "Apple Container Restart Stop failed: {}",
                                error.message
                            ))
                        })?;
                    return run_command(cli, &["start", "--", name], ResourceCommand::Start, name)
                        .await
                        .map_err(|error| {
                            WorkspaceError::new(format!(
                                "Apple Container Restart Start failed: {}",
                                error.message
                            ))
                        });
                }
                ResourceCommand::Start => "start",
                ResourceCommand::Stop => "stop",
                _ => {
                    return Err(WorkspaceError::new(format!(
                        "Apple Container cannot {command} Container {name}"
                    )));
                }
            };
            run_command(cli, &[verb, "--", name], command, name).await
        })
    }

    fn load_details<'a>(
        &'a self,
        cli: &'a dyn CliRunner,
        target: &'a ResourceTarget,
        view_id: &'a DetailViewId,
    ) -> Pin<Box<dyn Future<Output = Result<ResourceDetails, WorkspaceError>> + Send + 'a>> {
        Box::pin(async move {
            let name = target.resource_id().0.as_str();
            let mut args = match (target.panel_id().0.as_str(), view_id.0.as_str()) {
                ("containers", "logs") => vec!["logs", "-n", "200"],
                ("containers", "stats") => vec!["stats", "--no-stream", "--format", "json"],
                ("containers", "boot-logs") => vec!["logs", "--boot", "-n", "200"],
                ("containers", "inspect") => vec!["inspect"],
                ("images", "inspect") => vec!["image", "inspect"],
                ("networks", "inspect") => vec!["network", "inspect"],
                ("volumes", "inspect") => vec!["volume", "inspect"],
                _ => {
                    return Err(WorkspaceError::new(format!(
                        "Apple Container has no {view_id} view for Resource Panel {}",
                        target.panel_id()
                    )));
                }
            };
            args.extend(["--", name]);
            let output = cli
                .run(ProcessSpec::new("container", &args))
                .await
                .map_err(|error| {
                    WorkspaceError::new(provider_cli_error(
                        PROVIDER_NAME,
                        &error,
                        &format!("Apple Container could not load {view_id} for Resource {name}"),
                    ))
                })?;
            if view_id.0 == "logs" || view_id.0 == "boot-logs" {
                let mut details = ResourceDetails::from_output(&output.stdout);
                details
                    .lines
                    .extend(ResourceDetails::from_output(&output.stderr).lines);
                return Ok(details);
            }
            if view_id.0 == "stats" {
                let rows: Vec<StatsRow> =
                    serde_json::from_str(&output.stdout).map_err(malformed)?;
                if rows.is_empty() {
                    return Ok(ResourceDetails::from_lines([format!(
                        "Statistics unavailable for Container {name}."
                    )]));
                }
                return Ok(ResourceDetails::from_output(
                    &serde_json::to_string_pretty(&rows).expect("Stats rows serialize"),
                ));
            }
            // Inspect is the only remaining view. Validate its native model
            // from the parsed tree while retaining all unsummarized fields.
            let value: serde_json::Value =
                serde_json::from_str(&output.stdout).map_err(malformed)?;
            NativeRows::parse(&target.panel_id().0, &value)?;
            Ok(ResourceDetails::from_output(
                &serde_json::to_string_pretty(&value).expect("JSON values serialize"),
            ))
        })
    }
}

async fn run_command(
    cli: &dyn CliRunner,
    args: &[&str],
    command: ResourceCommand,
    name: &str,
) -> Result<(), WorkspaceError> {
    cli.run(ProcessSpec::new("container", args))
        .await
        .map_err(|error| {
            WorkspaceError::new(provider_cli_error(
                PROVIDER_NAME,
                &error,
                &format!("Apple Container could not {command} Resource {name}"),
            ))
        })?;
    Ok(())
}

async fn load_panel(
    cli: &dyn CliRunner,
    id: &str,
    title: &str,
    args: &[&str],
) -> Result<ResourcePanel, WorkspaceError> {
    let output = cli
        .run(ProcessSpec::new("container", args))
        .await
        .map_err(|error| {
            WorkspaceError::with_help(
                provider_cli_error(
                    PROVIDER_NAME,
                    &error,
                    &format!("Apple Container could not list {title}"),
                ),
                RECOVERY,
            )
        })?;
    // Listings need only summary fields. Inspect can use the same schema
    // selection with its borrowed JSON tree, without retaining one here.
    let mut deserializer = serde_json::Deserializer::from_str(&output.stdout);
    let resources = NativeRows::parse(id, &mut deserializer)?.into_resources();
    deserializer.end().map_err(malformed)?;
    Ok(ResourcePanel {
        id: ResourcePanelId::new(id),
        title: title.into(),
        detail_views: if id == "containers" {
            vec![
                DetailView::new("logs", "Logs"),
                DetailView::new("boot-logs", "Boot Logs"),
                DetailView::new("stats", "Stats"),
                DetailView::new("inspect", "Inspect"),
                DetailView::configuration(),
            ]
        } else {
            vec![DetailView::new("inspect", "Inspect")]
        },
        resources,
    })
}
