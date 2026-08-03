use crate::domain::{
    DetailViewId, ProviderId, ResourceId, ResourcePanelId, ResourceState, ResourceTarget,
};

use super::{InteractiveShellProcess, ResourceCommand};

const RUNNING_RESTARTABLE: &[ResourceCommand] = &[
    ResourceCommand::Stop,
    ResourceCommand::Restart,
    ResourceCommand::Delete,
];
const RUNNING_START_STOP: &[ResourceCommand] = &[ResourceCommand::Stop, ResourceCommand::Delete];
const STOPPED: &[ResourceCommand] = &[ResourceCommand::Start, ResourceCommand::Delete];
const PAUSED: &[ResourceCommand] = &[ResourceCommand::Resume, ResourceCommand::Delete];
const UNSETTLED: &[ResourceCommand] = &[ResourceCommand::Delete];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// The real lifecycle difference between Provider command sets.
pub enum LifecycleCommandPolicy {
    RestartAndResume,
    StartStop,
}

/// Derives allocation-free Resource Commands from shared lifecycle policy.
pub fn lifecycle_commands(
    state: ResourceState,
    policy: LifecycleCommandPolicy,
) -> &'static [ResourceCommand] {
    match state {
        ResourceState::Running if policy == LifecycleCommandPolicy::RestartAndResume => {
            RUNNING_RESTARTABLE
        }
        ResourceState::Running => RUNNING_START_STOP,
        ResourceState::Stopped => STOPPED,
        ResourceState::Paused if policy == LifecycleCommandPolicy::RestartAndResume => PAUSED,
        ResourceState::Paused => UNSETTLED,
        ResourceState::Transitioning | ResourceState::Broken | ResourceState::Unknown => UNSETTLED,
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
/// Identifies one asynchronous request sent to a Provider Workspace.
///
/// The application allocates these IDs and accepts a completion only while its
/// ID remains pending for its Provider. That prevents stale provider results
/// from overwriting newer application state.
pub struct ProviderRequestId(pub(crate) u64);

impl ProviderRequestId {
    /// Reconstructs an application-allocated request identity at a state seam.
    /// The application remains responsible for choosing unique values.
    pub fn new(value: u64) -> Self {
        Self(value)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// A specific request the application asks a Provider Workspace to perform.
///
/// Infrastructure executes requests outside the single-owner application state.
pub enum ProviderRequest {
    RefreshWorkspace {
        request_id: ProviderRequestId,
        provider_id: ProviderId,
    },
    ExecuteResourceCommand {
        request_id: ProviderRequestId,
        provider_id: ProviderId,
        target: ResourceTarget,
        command: ResourceCommand,
        /// What the last refresh reported for this Resource, carried here so
        /// the Provider Workspace never re-queries it while dispatching.
        state: ResourceState,
    },
    /// Loads one detail view for one Resource.
    ///
    /// The application asks for exactly the view on screen, so a Provider never
    /// runs work for a detail the user is not looking at.
    LoadResourceDetails {
        request_id: ProviderRequestId,
        provider_id: ProviderId,
        target: ResourceTarget,
        view_id: DetailViewId,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// One selectable native resource in a Provider Workspace.
pub struct Resource {
    pub id: ResourceId,
    pub name: String,
    /// Provider-defined status text shown next to the resource in the list.
    pub status: Option<String>,
    /// The provider-neutral reading of `status` that application policy uses.
    pub state: Option<ResourceState>,
    /// Provider-defined label/value fields for the selected-resource details panel.
    pub fields: Vec<(&'static str, String)>,
    /// Detail content already carried by this application-owned snapshot.
    pub snapshot_details: Vec<(DetailViewId, ResourceDetails)>,
    /// Lifecycle Commands currently available for this provider Resource.
    pub available_commands: &'static [ResourceCommand],
    /// The Interactive Shell this Provider offers inside the Resource now.
    pub shell: Option<InteractiveShellProcess>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// One provider-native way of inspecting a selected Resource.
pub struct DetailView {
    pub id: DetailViewId,
    pub title: String,
    source: DetailViewSource,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum DetailViewSource {
    Provider,
    Snapshot,
}

impl DetailView {
    pub fn new(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            id: DetailViewId::new(id),
            title: title.into(),
            source: DetailViewSource::Provider,
        }
    }

    /// Declares Detail content that the Resource snapshot already carries.
    pub fn from_snapshot(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            id: DetailViewId::new(id),
            title: title.into(),
            source: DetailViewSource::Snapshot,
        }
    }

    pub(crate) fn loads_from_snapshot(&self) -> bool {
        self.source == DetailViewSource::Snapshot
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// A provider-defined group of resources, such as Docker Containers.
pub struct ResourcePanel {
    pub id: ResourcePanelId,
    pub title: String,
    /// Detail views offered for every Resource in this panel, in display order.
    pub detail_views: Vec<DetailView>,
    pub resources: Vec<Resource>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// Presentation-neutral data returned by a successful provider refresh.
pub struct WorkspaceSnapshot {
    pub panels: Vec<ResourcePanel>,
}

impl WorkspaceSnapshot {
    /// Iterates every Resource across all panels in provider-defined order.
    pub fn resources(&self) -> impl Iterator<Item = &Resource> {
        self.panels.iter().flat_map(|panel| panel.resources.iter())
    }

    /// Iterates every Resource with its panel-qualified target.
    pub fn targets(&self) -> impl Iterator<Item = (ResourceTarget, &Resource)> {
        self.panels.iter().flat_map(|panel| {
            panel.resources.iter().map(|resource| {
                (
                    ResourceTarget::new(panel.id.clone(), resource.id.clone()),
                    resource,
                )
            })
        })
    }

    pub fn panel(&self, panel_id: &ResourcePanelId) -> Option<&ResourcePanel> {
        self.panels.iter().find(|panel| &panel.id == panel_id)
    }

    pub(crate) fn panel_for(&self, target: &ResourceTarget) -> Option<&ResourcePanel> {
        self.panel(target.panel_id())
    }

    pub fn resource(&self, target: &ResourceTarget) -> Option<&Resource> {
        self.panel_for(target)?
            .resources
            .iter()
            .find(|resource| &resource.id == target.resource_id())
    }

    /// Resolves application-owned Detail content without Provider work.
    ///
    /// `None` means the declared view is Provider-backed. A snapshot-backed
    /// view whose Resource is absent is present but empty.
    pub fn snapshot_detail(
        &self,
        target: &ResourceTarget,
        view_id: &DetailViewId,
    ) -> Option<ResourceDetails> {
        let panel = self.panel_for(target)?;
        let view = panel.detail_views.iter().find(|view| &view.id == view_id)?;
        if !view.loads_from_snapshot() {
            return None;
        }
        Some(
            self.resource(target)
                .and_then(|resource| {
                    resource
                        .snapshot_details
                        .iter()
                        .find(|(id, _)| id == view_id)
                        .map(|(_, details)| details.clone())
                })
                .unwrap_or_default(),
        )
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
/// Presentation-neutral output of one detail view for one Resource.
pub struct ResourceDetails {
    pub lines: Vec<String>,
}

impl ResourceDetails {
    pub fn from_lines<I, S>(lines: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            lines: lines.into_iter().map(Into::into).collect(),
        }
    }

    /// Splits provider output into displayable lines, dropping a trailing blank.
    pub fn from_output(output: &str) -> Self {
        Self::from_lines(output.trim_end_matches('\n').lines())
    }

    pub fn is_empty(&self) -> bool {
        self.lines.iter().all(|line| line.trim().is_empty())
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

    pub fn with_help(message: impl AsRef<str>, help: &str) -> Self {
        Self::new(format!("{}. {help}", message.as_ref()))
    }
}
