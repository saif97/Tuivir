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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResourceCommand {
    Start,
    Stop,
    Restart,
    /// Returns a suspended Resource to running — Docker `unpause`, Incus
    /// `unfreeze`.
    Resume,
    Delete,
}

impl fmt::Display for ResourceCommand {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Start => "start",
            Self::Stop => "stop",
            Self::Restart => "restart",
            Self::Resume => "resume",
            Self::Delete => "delete",
        };
        formatter.write_str(name)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// What a Provider reported a Resource to be doing at the last refresh.
///
/// This is a provider-neutral vocabulary that each Provider Workspace maps its
/// own status words into, so the shell can act on a Resource's state without
/// branching on Provider identity. [`Resource::status`] keeps the Provider's
/// own word for display.
///
/// Only [`ResourceState::Stopped`] is ever positively determined. Every other
/// variant, `Unknown` included, means "not settled and stopped", which is what
/// makes forcing a deletion the safe default: an unrecognised status can never
/// masquerade as a stopped Resource.
pub enum ResourceState {
    Running,
    /// Settled and not running: safe to remove without stopping anything first.
    Stopped,
    /// Suspended but still resident — Docker `paused`, Incus `Frozen`.
    Paused,
    /// Moving between states — Docker `restarting`/`removing`, Incus
    /// `Starting`/`Stopping`/`Freezing`/`Thawing`.
    Transitioning,
    /// The Provider reports the Resource as unusable — Docker `dead`, Incus
    /// `Error`.
    Broken,
    /// A status word this Provider Workspace does not recognise.
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// A specific request the application asks a Provider Workspace to perform.
///
/// The runtime executes requests outside the single-owner application state.
pub enum ProviderRequest {
    RefreshWorkspace {
        request_id: ProviderRequestId,
        provider_id: ProviderId,
    },
    ExecuteResourceCommand {
        request_id: ProviderRequestId,
        provider_id: ProviderId,
        resource_id: ResourceId,
        resource_name: String,
        command: ResourceCommand,
        /// What the last refresh reported for this Resource, carried here so
        /// the Provider Workspace never re-queries it while dispatching.
        state: ResourceState,
    },
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
    /// The provider-neutral reading of `status` that the shell can act on.
    pub state: ResourceState,
    /// Provider-defined label/value fields for the selected-resource details panel.
    pub fields: Vec<(String, String)>,
    /// Lifecycle Commands currently available for this provider Resource.
    pub available_commands: Vec<ResourceCommand>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
/// Identifies one detail view a Provider Workspace offers for its Resources.
pub struct DetailViewId(pub String);

impl DetailViewId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

impl fmt::Display for DetailViewId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// One provider-native way of inspecting a selected Resource.
///
/// Views belong to the Provider Workspace that declared them and keep their
/// own names — Docker's Logs is not Incus's Console Log — so the shell can
/// offer them without knowing what either Provider inspects.
pub struct DetailView {
    pub id: DetailViewId,
    pub title: String,
}

impl DetailView {
    pub fn new(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            id: DetailViewId::new(id),
            title: title.into(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// A provider-defined group of resources, such as Docker Containers.
pub struct ResourcePanel {
    pub title: String,
    /// The detail views offered for every Resource in this panel, in the order
    /// the user moves through them. The first is shown when a Resource is
    /// selected.
    pub detail_views: Vec<DetailView>,
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

    /// The panel `resource_id` belongs to, and so the detail views offered for
    /// it.
    pub fn panel_of(&self, resource_id: &ResourceId) -> Option<&ResourcePanel> {
        self.panels.iter().find(|panel| {
            panel
                .resources
                .iter()
                .any(|resource| &resource.id == resource_id)
        })
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

    /// Runs one lifecycle Command against a Resource.
    ///
    /// `state` is what the last refresh reported for that Resource, so a
    /// Command that must behave differently for a running Resource can do so
    /// without a second Provider CLI query.
    fn execute_command<'a>(
        &'a self,
        cli: &'a dyn CliRunner,
        resource_id: &'a ResourceId,
        command: ResourceCommand,
        state: ResourceState,
    ) -> Pin<Box<dyn Future<Output = Result<(), WorkspaceError>> + Send + 'a>>;
}
