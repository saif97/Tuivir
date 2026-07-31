use crate::provider::{
    DetailView, DetailViewId, ProviderDiscovery, ProviderId, ProviderRequestId, ProviderVersion,
    Resource, ResourceDetails, ResourceId, ResourcePanel, ResourcePanelId, ResourceTarget,
    TargetEnvironment, WorkspaceError, WorkspaceSnapshot,
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
pub enum DetailContent {
    Loading,
    Ready(ResourceDetails),
    Error(WorkspaceError),
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ResourceDetailsState {
    panel_id: ResourcePanelId,
    resource_id: ResourceId,
    resource_name: String,
    view_id: DetailViewId,
    title: String,
    content: DetailContent,
    scroll: u16,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DetailLoad {
    pub request_id: ProviderRequestId,
    pub provider_id: ProviderId,
    pub panel_id: ResourcePanelId,
    pub resource_id: ResourceId,
    pub view_id: DetailViewId,
}

impl DetailLoad {
    pub fn completion(self, result: Result<ResourceDetails, WorkspaceError>) -> DetailCompletion {
        DetailCompletion { load: self, result }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DetailCompletion {
    load: DetailLoad,
    result: Result<ResourceDetails, WorkspaceError>,
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
    details: Option<ResourceDetailsState>,
    pending_detail: Option<DetailLoad>,
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
            details: None,
            pending_detail: None,
        }
    }

    pub fn load_state(&self) -> &WorkspaceLoadState {
        &self.load_state
    }

    /// Focuses an existing Resource Panel and restores its remembered Resource
    /// selection. A stale panel identity changes nothing.
    pub fn focus_resource_panel(&mut self, panel_id: &ResourcePanelId) -> bool {
        let WorkspaceLoadState::Ready(snapshot) = &self.load_state else {
            return false;
        };
        let Some(panel) = snapshot.panel(panel_id) else {
            return false;
        };
        let offered = panel
            .detail_views
            .iter()
            .map(|view| view.id.clone())
            .collect::<Vec<_>>();
        self.focused_resource_panel = Some(panel_id.clone());
        let still_offered = self
            .selected_detail_view
            .as_ref()
            .is_some_and(|selected| offered.contains(selected));
        if !still_offered {
            self.selected_detail_view = offered.into_iter().next();
        }
        true
    }

    /// Moves the selected Resource within the focused panel and clamps at its
    /// ends. Every other panel keeps its own selection and scroll.
    pub fn move_resource_selection(&mut self, delta: isize) {
        let WorkspaceLoadState::Ready(snapshot) = &self.load_state else {
            return;
        };
        let Some(panel_id) = self.focused_resource_panel.as_ref() else {
            return;
        };
        let Some(panel) = snapshot.panel(panel_id) else {
            return;
        };
        let Some(navigation) = self
            .panel_navigation
            .iter_mut()
            .find(|navigation| &navigation.panel_id == panel_id)
        else {
            return;
        };
        let Some(current) = navigation.selected_resource.as_ref().and_then(|selected| {
            panel
                .resources
                .iter()
                .position(|resource| &resource.id == selected)
        }) else {
            navigation.selected_resource =
                panel.resources.first().map(|resource| resource.id.clone());
            navigation.scroll = 0;
            return;
        };
        let next = current
            .saturating_add_signed(delta)
            .min(panel.resources.len().saturating_sub(1));
        navigation.selected_resource = panel
            .resources
            .get(next)
            .map(|resource| resource.id.clone());
        navigation.scroll = next;
    }

    /// Moves through the focused panel's Detail Views as a ring.
    pub fn move_detail_view(&mut self, delta: isize) {
        let WorkspaceLoadState::Ready(snapshot) = &self.load_state else {
            return;
        };
        let Some(panel_id) = self.focused_resource_panel.as_ref() else {
            return;
        };
        let Some(panel) = snapshot.panel(panel_id) else {
            return;
        };
        if panel.detail_views.is_empty() {
            return;
        }
        let current = self
            .selected_detail_view
            .as_ref()
            .and_then(|selected| {
                panel
                    .detail_views
                    .iter()
                    .position(|view| &view.id == selected)
            })
            .unwrap_or(0);
        let next =
            (current as isize + delta).rem_euclid(panel.detail_views.len() as isize) as usize;
        self.selected_detail_view = panel.detail_views.get(next).map(|view| view.id.clone());
    }

    /// Starts a load only when the visible Resource and Detail View are not
    /// already loaded or pending. The application supplies the request ID and
    /// dispatches the returned work.
    pub fn start_visible_detail_load(
        &mut self,
        request_id: ProviderRequestId,
    ) -> Option<DetailLoad> {
        let (target, resource_name, view) = self.detail_target()?;
        let pending_for_target = self.pending_detail.as_ref().is_some_and(|pending| {
            pending.panel_id == target.panel_id
                && pending.resource_id == target.resource_id
                && pending.view_id == view.id
        });
        if !pending_for_target {
            self.pending_detail = None;
        }
        let describes_target = self.details.as_ref().is_some_and(|details| {
            details.panel_id == target.panel_id
                && details.resource_id == target.resource_id
                && details.view_id == view.id
        });
        if pending_for_target || describes_target {
            return None;
        }

        let load = DetailLoad {
            request_id,
            provider_id: self.id.clone(),
            panel_id: target.panel_id.clone(),
            resource_id: target.resource_id.clone(),
            view_id: view.id.clone(),
        };
        self.details = Some(ResourceDetailsState {
            panel_id: target.panel_id,
            resource_id: target.resource_id,
            resource_name,
            view_id: view.id,
            title: view.title,
            content: DetailContent::Loading,
            scroll: 0,
        });
        self.pending_detail = Some(load.clone());
        Some(load)
    }

    /// Accepts a detail completion only while its full request identity still
    /// describes the visible load.
    pub fn complete_detail_load(&mut self, completion: DetailCompletion) {
        if self.pending_detail.as_ref() != Some(&completion.load) {
            return;
        }
        self.pending_detail = None;
        let Some(details) = self.details.as_mut() else {
            return;
        };
        details.content = match completion.result {
            Ok(loaded) => DetailContent::Ready(loaded),
            Err(error) => DetailContent::Error(error),
        };
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
        let details = selected_target.as_ref().and_then(|selected| {
            self.details.as_ref().filter(|details| {
                details.panel_id == selected.panel_id
                    && details.resource_id == selected.resource_id
                    && Some(&details.view_id) == self.selected_detail_view.as_ref()
            })
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
            details: details.map(ResourceDetailsView::from),
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

    fn detail_target(&self) -> Option<(ResourceTarget, String, DetailView)> {
        let WorkspaceLoadState::Ready(snapshot) = &self.load_state else {
            return None;
        };
        let target = self.selected_resource_target()?;
        let resource = snapshot.resource(&target.panel_id, &target.resource_id)?;
        let view_id = self.selected_detail_view.as_ref()?;
        let view = snapshot
            .panel(&target.panel_id)?
            .detail_views
            .iter()
            .find(|view| &view.id == view_id)?
            .clone();
        Some((target, resource.name.clone(), view))
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
    pub details: Option<ResourceDetailsView<'a>>,
}

#[derive(Clone, Copy)]
pub struct ResourceDetailsView<'a> {
    pub resource_name: &'a str,
    pub title: &'a str,
    pub content: &'a DetailContent,
    pub scroll: u16,
}

impl<'a> From<&'a ResourceDetailsState> for ResourceDetailsView<'a> {
    fn from(details: &'a ResourceDetailsState) -> Self {
        Self {
            resource_name: &details.resource_name,
            title: &details.title,
            content: &details.content,
            scroll: details.scroll,
        }
    }
}

/// One Resource Panel paired with its private navigation projection.
pub struct ResourcePanelView<'a> {
    pub panel: &'a ResourcePanel,
    pub selected_resource: Option<&'a ResourceId>,
    pub scroll: usize,
}
