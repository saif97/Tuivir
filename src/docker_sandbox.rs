use std::{future::Future, pin::Pin};

use crate::{
    cli::{CliRunner, ProcessError, ProcessSpec},
    provider::{
        DetailViewId, ProviderDiscovery, ProviderId, ProviderWorkspace, ResourceCommand,
        ResourceDetails, ResourceId, ResourceState, WorkspaceError, WorkspaceSnapshot,
    },
};

const PROVIDER_ID: &str = "docker-sandbox";

pub struct DockerSandboxWorkspace;

impl ProviderWorkspace for DockerSandboxWorkspace {
    fn id(&self) -> ProviderId {
        ProviderId::new(PROVIDER_ID)
    }

    fn discover<'a>(
        &'a self,
        cli: &'a dyn CliRunner,
    ) -> Pin<Box<dyn Future<Output = Option<ProviderDiscovery>> + Send + 'a>> {
        Box::pin(async move {
            match cli.run(ProcessSpec::new("sbx", &["version"])).await {
                Err(ProcessError::ExecutableNotFound) => None,
                _ => None,
            }
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
