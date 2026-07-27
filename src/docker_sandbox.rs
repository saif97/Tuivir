use std::{future::Future, pin::Pin};

use crate::{
    cli::{CliRunner, ProcessError, ProcessSpec},
    provider::{
        DetailViewId, ProviderDiscovery, ProviderId, ProviderWorkspace, ResourceCommand,
        ResourceDetails, ResourceId, ResourceState, WorkspaceError, WorkspaceSnapshot,
    },
};

const PROVIDER_ID: &str = "docker-sandbox";
const PROVIDER_NAME: &str = "Docker Sandbox";

pub struct DockerSandboxWorkspace;

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
                Ok(output) => sbx_version(&output.stdout),
                Err(_) => return None,
            };
            cli.run(ProcessSpec::new("sbx", &["ls", "--json"]))
                .await
                .ok()?;

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
        _cli: &'a dyn CliRunner,
    ) -> Pin<Box<dyn Future<Output = Result<WorkspaceSnapshot, WorkspaceError>> + Send + 'a>> {
        Box::pin(async move { Err(WorkspaceError::new("not implemented")) })
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
