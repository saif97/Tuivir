use std::{future::Future, pin::Pin};

use serde::Deserialize;

use crate::{
    cli::{CliRunner, ProcessError, ProcessSpec},
    provider::{
        DetailViewId, ProviderDiscovery, ProviderId, ProviderWorkspace, Resource, ResourceCommand,
        ResourceDetails, ResourceId, ResourcePanel, ResourceState, WorkspaceError,
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
        Some(rest) => rest.split_whitespace().next().unwrap_or(reported).to_owned(),
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
                    detail_views: Vec::new(),
                    resources,
                }],
            })
        })
    }

    fn execute_command<'a>(
        &'a self,
        _cli: &'a dyn CliRunner,
        _resource_id: &'a ResourceId,
        _command: ResourceCommand,
        _state: ResourceState,
    ) -> Pin<Box<dyn Future<Output = Result<(), WorkspaceError>> + Send + 'a>> {
        Box::pin(async move { Err(WorkspaceError::new("not implemented")) })
    }

    fn load_details<'a>(
        &'a self,
        _cli: &'a dyn CliRunner,
        _resource_id: &'a ResourceId,
        _view_id: &'a DetailViewId,
    ) -> Pin<Box<dyn Future<Output = Result<ResourceDetails, WorkspaceError>> + Send + 'a>> {
        Box::pin(async move { Err(WorkspaceError::new("not implemented")) })
    }
}
