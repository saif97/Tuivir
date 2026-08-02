//! Compatibility facade for provider infrastructure and shared contracts.

pub use crate::{
    application::{
        DetailView, InteractiveShellProcess, ProviderRequest, ProviderRequestId, Resource,
        ResourceCommand, ResourceDetails, ResourcePanel, WorkspaceError, WorkspaceSnapshot,
    },
    domain::{
        DetailViewId, Provider, ProviderId, ProviderVersion, ResourceId, ResourcePanelId,
        ResourceState, ResourceTarget, TargetEnvironment,
    },
    infrastructure::provider::{ProviderDiscovery, ProviderWorkspace, provider_cli_error},
};
