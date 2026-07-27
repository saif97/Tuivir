use std::{future::Future, pin::Pin};

use serde::Deserialize;

use crate::{
    cli::{CliRunner, ProcessError, ProcessSpec},
    provider::{
        DetailView, DetailViewId, ProviderDiscovery, ProviderId, ProviderWorkspace, Resource,
        ResourceCommand, ResourceDetails, ResourceId, ResourcePanel, ResourceState, WorkspaceError,
        WorkspaceSnapshot,
    },
};

const PROVIDER_ID: &str = "docker-sandbox";
const PROVIDER_NAME: &str = "Docker Sandbox";

pub struct DockerSandboxWorkspace;

#[derive(Deserialize)]
struct SandboxListing {
    sandboxes: Vec<SandboxRow>,
}

#[derive(Deserialize)]
struct SandboxRow {
    name: String,
    /// The UUID sbx assigns. It addresses no sbx command, so it is shown but
    /// never used as the Resource identity.
    id: String,
    agent: String,
    status: String,
    /// Host paths mounted into the sandbox.
    #[serde(default)]
    workspaces: Vec<String>,
    /// Published ports. sbx reports these only for a running sandbox, so a
    /// stopped row omits the key entirely.
    #[serde(default)]
    ports: Vec<SandboxPort>,
}

#[derive(Deserialize)]
struct SandboxPort {
    host_ip: String,
    host_port: u16,
    sandbox_port: u16,
    protocol: String,
}

/// Lays a sandbox's `sbx ls --json` row out for reading.
///
/// sbx has no per-sandbox inspect command, so this row is everything it knows;
/// a section it has nothing for is left out rather than shown empty.
fn sandbox_info(row: &SandboxRow) -> Vec<String> {
    let mut lines = vec![
        format!("Name: {}", row.name),
        format!("ID: {}", row.id),
        format!("Agent: {}", row.agent),
        format!("Status: {}", row.status),
    ];
    if !row.workspaces.is_empty() {
        lines.push("Workspaces:".to_owned());
        lines.extend(row.workspaces.iter().map(|path| format!("  {path}")));
    }
    if !row.ports.is_empty() {
        lines.push("Ports:".to_owned());
        lines.extend(row.ports.iter().map(|port| {
            format!(
                "  {}:{} -> {}/{}",
                port.host_ip, port.host_port, port.sandbox_port, port.protocol
            )
        }));
    }
    lines
}

/// Runs `sbx ls --json` and parses the one object it wraps its rows in.
///
/// sbx starts sandboxd on demand and narrates that on stderr, so only stdout
/// is ever parsed.
async fn list_sandboxes(cli: &dyn CliRunner) -> Result<SandboxListing, WorkspaceError> {
    let output = cli
        .run(ProcessSpec::new("sbx", &["ls", "--json"]))
        .await
        .map_err(|error| refresh_error(listing_failure(error)))?;
    serde_json::from_str(&output.stdout)
        .map_err(|error| refresh_error(format!("Docker Sandbox returned malformed data: {error}")))
}

/// The views Docker Sandbox itself offers for a sandbox.
///
/// sbx has no logs, stats, or console command, so Info is the only view it can
/// answer — and borrowing Docker's names for diagnostics sbx does not have
/// would promise the user something that does not exist.
fn sandbox_detail_views() -> Vec<DetailView> {
    vec![DetailView::new("info", "Info")]
}

/// Maps an sbx sandbox status onto the shared vocabulary.
///
/// sbx reports `running` and `stopped`; it offers no pause, so nothing maps to
/// `Paused`. Anything else is deliberately left `Unknown` rather than assumed
/// to be stopped.
fn sandbox_resource_state(status: &str) -> ResourceState {
    match status.to_ascii_lowercase().as_str() {
        "running" => ResourceState::Running,
        "stopped" => ResourceState::Stopped,
        _ => ResourceState::Unknown,
    }
}

fn refresh_error(message: impl AsRef<str>) -> WorkspaceError {
    WorkspaceError::new(format!(
        "{}. Run `sbx ls` to verify access to the current Target Environment.",
        message.as_ref()
    ))
}

fn discovery_error(message: impl AsRef<str>) -> ProviderDiscovery {
    ProviderDiscovery {
        id: ProviderId::new(PROVIDER_ID),
        name: PROVIDER_NAME.to_owned(),
        target_environment: "unavailable".to_owned(),
        error: Some(WorkspaceError::new(format!(
            "{}. Run `sbx ls` to verify sandboxd is running and you are signed in to Docker.",
            message.as_ref(),
        ))),
    }
}

/// What `sbx ls` left behind when it could not list sandboxes.
fn listing_failure(error: ProcessError) -> String {
    match error {
        ProcessError::ExecutableNotFound => "Docker Sandbox CLI is no longer available".to_owned(),
        ProcessError::SpawnFailed(message) => not_started(&message),
        ProcessError::Exited(failure) => {
            failure.message_or("Docker Sandbox could not list sandboxes")
        }
    }
}

/// The sbx command behind each lifecycle Command, or `None` for one sbx has no
/// way to perform.
///
/// Start goes through `sbx exec` rather than the `sbx run --name` sbx
/// documents for reattaching: `run` opens an interactive agent session that
/// never exits, which would leave the request pending forever. `exec` starts a
/// stopped sandbox before running its command, and `-d` returns as soon as it
/// has.
///
/// Deletion always forces. Unlike Docker and Incus, the flag is not about a
/// running Resource: `sbx rm` prompts for confirmation it reads from a
/// terminal Virtui does not give it, and `--force` is what skips that prompt.
/// The user has already confirmed through Virtui's own.
fn sandbox_command(command: ResourceCommand, resource_id: &str) -> Option<Vec<&str>> {
    match command {
        ResourceCommand::Start => Some(vec!["exec", "-d", resource_id, "true"]),
        ResourceCommand::Stop => Some(vec!["stop", resource_id]),
        ResourceCommand::Delete => Some(vec!["rm", "--force", resource_id]),
        // sbx has no restart, and no pause to resume from.
        ResourceCommand::Restart | ResourceCommand::Resume => None,
    }
}

fn command_error(error: ProcessError, fallback: &str) -> WorkspaceError {
    let message = match error {
        ProcessError::ExecutableNotFound => "Docker Sandbox CLI is no longer available".to_owned(),
        ProcessError::SpawnFailed(message) => not_started(&message),
        ProcessError::Exited(failure) => failure.message_or(fallback),
    };
    WorkspaceError::new(message)
}

fn not_started(reason: &str) -> String {
    format!("Docker Sandbox CLI could not be started: {reason}")
}

/// The version out of `sbx version: v0.37.0 <commit>`.
///
/// The build commit identifies nothing the user is targeting, so only the
/// version reaches the Target Environment. An unrecognised line is shown whole
/// rather than guessed at.
fn sbx_version(output: &str) -> String {
    let reported = output.trim();
    match reported.strip_prefix("sbx version:") {
        Some(rest) => rest
            .split_whitespace()
            .next()
            .unwrap_or(reported)
            .to_owned(),
        None => reported.to_owned(),
    }
}

impl ProviderWorkspace for DockerSandboxWorkspace {
    fn id(&self) -> ProviderId {
        ProviderId::new(PROVIDER_ID)
    }

    fn discover<'a>(
        &'a self,
        cli: &'a dyn CliRunner,
    ) -> Pin<Box<dyn Future<Output = Option<ProviderDiscovery>> + Send + 'a>> {
        Box::pin(async move {
            // `sbx version` answers "installed?" on its own: it never contacts
            // sandboxd, so an absent CLI is distinguishable from an installed
            // one whose daemon is down or whose login has lapsed.
            let version = match cli.run(ProcessSpec::new("sbx", &["version"])).await {
                Err(ProcessError::ExecutableNotFound) => return None,
                Err(ProcessError::SpawnFailed(message)) => {
                    return Some(discovery_error(not_started(&message)));
                }
                Err(ProcessError::Exited(failure)) => {
                    return Some(discovery_error(
                        failure.message_or("Docker Sandbox could not report its version"),
                    ));
                }
                Ok(output) => sbx_version(&output.stdout),
            };
            // Listing is what proves sbx is usable, and it fails for the two
            // reasons the user can act on: sandboxd is down, or the Docker
            // login has lapsed.
            if let Err(error) = cli.run(ProcessSpec::new("sbx", &["ls", "--json"])).await {
                return Some(discovery_error(listing_failure(error)));
            }

            Some(ProviderDiscovery {
                id: self.id(),
                name: PROVIDER_NAME.to_owned(),
                target_environment: version,
                error: None,
            })
        })
    }

    fn refresh<'a>(
        &'a self,
        cli: &'a dyn CliRunner,
    ) -> Pin<Box<dyn Future<Output = Result<WorkspaceSnapshot, WorkspaceError>> + Send + 'a>> {
        Box::pin(async move {
            let listing = list_sandboxes(cli).await?;
            let resources = listing
                .sandboxes
                .into_iter()
                .map(|row| {
                    let state = sandbox_resource_state(&row.status);
                    Resource {
                        id: ResourceId::new(&row.name),
                        name: row.name,
                        status: Some(row.status),
                        state,
                        fields: vec![
                            ("Agent".to_owned(), row.agent),
                            ("ID".to_owned(), row.id),
                            ("Workspaces".to_owned(), row.workspaces.join(", ")),
                        ],
                        available_commands: Vec::new(),
                    }
                })
                .collect();

            Ok(WorkspaceSnapshot {
                panels: vec![ResourcePanel {
                    title: "Sandboxes".to_owned(),
                    detail_views: sandbox_detail_views(),
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
        _state: ResourceState,
    ) -> Pin<Box<dyn Future<Output = Result<(), WorkspaceError>> + Send + 'a>> {
        Box::pin(async move {
            let Some(args) = sandbox_command(command, resource_id.0.as_str()) else {
                return Err(WorkspaceError::new(format!(
                    "Docker Sandbox cannot {command} sandbox {resource_id}"
                )));
            };
            cli.run(ProcessSpec::new("sbx", &args))
                .await
                .map_err(|error| {
                    command_error(
                        error,
                        &format!("Docker Sandbox could not {command} sandbox {resource_id}"),
                    )
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
            if view_id.0 != "info" {
                return Err(WorkspaceError::new(format!(
                    "Docker Sandbox has no {view_id} view for sandbox {resource_id}"
                )));
            }
            let listing = list_sandboxes(cli).await?;
            let Some(row) = listing
                .sandboxes
                .into_iter()
                .find(|row| row.name == resource_id.0)
            else {
                // The sandbox was listed at the last refresh and is gone now.
                // That is an empty view, not a failure: the panel is about to
                // drop it anyway.
                return Ok(ResourceDetails::default());
            };
            Ok(ResourceDetails::from_lines(sandbox_info(&row)))
        })
    }
}
