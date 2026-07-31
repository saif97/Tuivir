use crate::provider::{
    DetailView, DetailViewId, ProviderDiscovery, ProviderId, ProviderVersion, Resource, ResourceId,
    ResourcePanel, ResourcePanelId, ResourceTarget, TargetEnvironment, WorkspaceError,
    WorkspaceSnapshot,
};

#[derive(Clone, Debug, Eq, PartialEq)]
/// Whether a Provider Workspace is loading, ready to present, or unavailable.
///
/// This load status is one part of [`ProviderWorkspaceState`], not the overall
/// state of the Provider Workspace.
pub enum WorkspaceLoadState {
    Loading,
    Ready(WorkspaceSnapshot),
    Error(WorkspaceError),
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ResourcePanelNavigation {
    panel_id: ResourcePanelId,
    selected_resource: Option<ResourceId>,
    /// The first Resource row on screen.
    scroll: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// All UI-neutral presentation state for one Provider Workspace.
pub struct ProviderWorkspaceState {
    id: ProviderId,
    name: String,
    target_environment: TargetEnvironment,
    version: Option<ProviderVersion>,
    load_state: WorkspaceLoadState,
    focused_resource_panel: Option<ResourcePanelId>,
    panel_navigation: Vec<ResourcePanelNavigation>,
    selected_detail_view: Option<DetailViewId>,
}

impl ProviderWorkspaceState {
    pub fn new(discovery: ProviderDiscovery) -> Self {
        let load_state = discovery
            .error
            .map_or(WorkspaceLoadState::Loading, WorkspaceLoadState::Error);
        Self {
            id: discovery.id,
            name: discovery.name,
            target_environment: discovery.target_environment,
            version: discovery.version,
            load_state,
            focused_resource_panel: None,
            panel_navigation: Vec::new(),
            selected_detail_view: None,
        }
    }

    pub fn load_state(&self) -> &WorkspaceLoadState {
        &self.load_state
    }

    /// Replaces Provider data while preserving every still-valid presentation
    /// choice by stable Provider identity.
    pub fn reconcile_snapshot(&mut self, snapshot: WorkspaceSnapshot) {
        let previous = std::mem::take(&mut self.panel_navigation);
        self.panel_navigation = snapshot
            .panels
            .iter()
            .map(|panel| {
                let remembered = previous
                    .iter()
                    .find(|navigation| navigation.panel_id == panel.id);
                let selected_resource = remembered
                    .and_then(|navigation| navigation.selected_resource.as_ref())
                    .filter(|selected| {
                        panel
                            .resources
                            .iter()
                            .any(|resource| &resource.id == *selected)
                    })
                    .cloned()
                    .or_else(|| panel.resources.first().map(|resource| resource.id.clone()));
                let selected_index = selected_resource
                    .as_ref()
                    .and_then(|selected| {
                        panel
                            .resources
                            .iter()
                            .position(|resource| &resource.id == selected)
                    })
                    .unwrap_or(0);
                ResourcePanelNavigation {
                    panel_id: panel.id.clone(),
                    selected_resource,
                    scroll: remembered
                        .map_or(0, |navigation| navigation.scroll)
                        .min(selected_index),
                }
            })
            .collect();

        let focused_still_exists = self
            .focused_resource_panel
            .as_ref()
            .is_some_and(|focused| snapshot.panel(focused).is_some());
        if !focused_still_exists {
            self.focused_resource_panel = snapshot.panels.first().map(|panel| panel.id.clone());
        }
        self.reconcile_detail_view(&snapshot);
        self.load_state = WorkspaceLoadState::Ready(snapshot);
    }

    /// Projects the private presentation state against the Provider snapshot
    /// that supplied its Resource data.
    pub fn view<'a>(&'a self, snapshot: &'a WorkspaceSnapshot) -> WorkspaceView<'a> {
        let selected_target = self.selected_resource_target();
        let selected_resource = selected_target
            .as_ref()
            .and_then(|selected| snapshot.resource(&selected.panel_id, &selected.resource_id));
        let selected_panel = selected_target
            .as_ref()
            .and_then(|selected| snapshot.panel(&selected.panel_id));
        let selected_detail_view = self.selected_detail_view.as_ref().and_then(|selected| {
            selected_panel?
                .detail_views
                .iter()
                .find(|view| &view.id == selected)
        });
        WorkspaceView {
            id: &self.id,
            name: &self.name,
            target_environment: &self.target_environment,
            version: self.version.as_ref(),
            focused_resource_panel: self.focused_resource_panel.as_ref(),
            panels: snapshot
                .panels
                .iter()
                .map(|panel| {
                    let navigation = self
                        .panel_navigation
                        .iter()
                        .find(|navigation| navigation.panel_id == panel.id);
                    ResourcePanelView {
                        panel,
                        selected_resource: navigation
                            .and_then(|navigation| navigation.selected_resource.as_ref()),
                        scroll: navigation.map_or(0, |navigation| navigation.scroll),
                    }
                })
                .collect(),
            selected_resource,
            selected_detail_view,
        }
    }

    fn selected_resource_target(&self) -> Option<ResourceTarget> {
        let panel_id = self.focused_resource_panel.as_ref()?;
        let resource_id = self
            .panel_navigation
            .iter()
            .find(|navigation| &navigation.panel_id == panel_id)?
            .selected_resource
            .as_ref()?;
        Some(ResourceTarget::new(panel_id.clone(), resource_id.clone()))
    }

    fn reconcile_detail_view(&mut self, snapshot: &WorkspaceSnapshot) {
        let offered = self
            .selected_resource_target()
            .as_ref()
            .and_then(|selected| snapshot.panel(&selected.panel_id))
            .map_or(&[][..], |panel| panel.detail_views.as_slice());
        let still_offered = self
            .selected_detail_view
            .as_ref()
            .is_some_and(|selected| offered.iter().any(|view| &view.id == selected));
        if !still_offered {
            self.selected_detail_view = offered.first().map(|view| view.id.clone());
        }
    }
}

/// The read-only projection consumed by presentation and application callers.
pub struct WorkspaceView<'a> {
    pub id: &'a ProviderId,
    pub name: &'a str,
    pub target_environment: &'a TargetEnvironment,
    pub version: Option<&'a ProviderVersion>,
    pub focused_resource_panel: Option<&'a ResourcePanelId>,
    pub panels: Vec<ResourcePanelView<'a>>,
    pub selected_resource: Option<&'a Resource>,
    pub selected_detail_view: Option<&'a DetailView>,
}

/// One Resource Panel paired with its private navigation projection.
pub struct ResourcePanelView<'a> {
    pub panel: &'a ResourcePanel,
    pub selected_resource: Option<&'a ResourceId>,
    pub scroll: usize,
}
