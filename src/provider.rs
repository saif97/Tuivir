use std::{fmt, future::Future, pin::Pin};

use crate::cli::CliRunner;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ProviderId(pub String);

impl ProviderId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

impl fmt::Display for ProviderId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
/// Identifies one asynchronous request sent to a Provider Workspace.
///
/// The application allocates these IDs and accepts a completion only while its
/// ID remains pending for its Provider. That prevents stale provider results
/// from overwriting newer application state.
pub struct ProviderRequestId(pub(crate) u64);

#[derive(Clone, Debug, Eq, PartialEq)]
/// A request from the application to one Provider Workspace.
pub struct ProviderRequest {
    pub id: ProviderRequestId,
    pub provider_id: ProviderId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// A specific operation the application asks a Provider Workspace to perform.
///
/// The runtime executes actions outside the single-owner application state.
pub enum ProviderAction {
    RefreshWorkspace(ProviderRequest),
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ResourceId(pub String);

impl ResourceId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

impl fmt::Display for ResourceId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// One selectable native resource in a Provider Workspace.
pub struct Resource {
    pub id: ResourceId,
    pub name: String,
    /// Provider-defined status text shown next to the resource in the list,
    /// such as a Docker container's running/exited state.
    pub status: Option<String>,
    /// Provider-defined label/value fields for the selected-resource details panel.
    pub fields: Vec<(String, String)>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// A provider-defined group of resources, such as Docker Containers.
pub struct ResourcePanel {
    pub title: String,
    pub resources: Vec<Resource>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// UI-neutral data returned by a successful provider refresh.
///
/// Provider workspaces populate snapshots; the shared shell renders them.
pub struct WorkspaceSnapshot {
    pub panels: Vec<ResourcePanel>,
}

impl WorkspaceSnapshot {
    /// Iterates every resource across all panels, in panel order.
    pub fn resources(&self) -> impl Iterator<Item = &Resource> {
        self.panels.iter().flat_map(|panel| &panel.resources)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// A user-facing failure from discovering or refreshing a Provider Workspace.
pub struct WorkspaceError {
    pub message: String,
}

impl WorkspaceError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// The presence and initial state of an installed provider.
///
/// `error` distinguishes an installed-but-unreachable provider from an absent
/// CLI, which is represented by `None` from [`ProviderWorkspace::discover`].
pub struct ProviderDiscovery {
    pub id: ProviderId,
    pub name: String,
    pub target_environment: String,
    pub error: Option<WorkspaceError>,
}

/// A provider-specific source of UI-neutral workspace data.
///
/// Implementations own CLI commands and parsing. They never render Ratatui
/// widgets, keeping provider knowledge out of the shared shell.
pub trait ProviderWorkspace: Send + Sync {
    fn id(&self) -> ProviderId;

    /// Returns `None` when the provider CLI is absent; otherwise returns an
    /// available provider or one with an actionable connection error.
    fn discover<'a>(
        &'a self,
        cli: &'a dyn CliRunner,
    ) -> Pin<Box<dyn Future<Output = Option<ProviderDiscovery>> + Send + 'a>>;

    /// Refreshes the provider's native resource panels through the CLI seam.
    fn refresh<'a>(
        &'a self,
        cli: &'a dyn CliRunner,
    ) -> Pin<Box<dyn Future<Output = Result<WorkspaceSnapshot, WorkspaceError>> + Send + 'a>>;
}
