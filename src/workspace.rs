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
    target: DetailTarget,
    resource_name: String,
    title: String,
    content: DetailContent,
    scroll: u16,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct DetailTarget {
    resource: ResourceTarget,
    view_id: DetailViewId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DetailLoad {
    request_id: ProviderRequestId,
    provider_id: ProviderId,
    target: DetailTarget,
}

impl DetailLoad {
    pub fn into_request_parts(
        self,
    ) -> (ProviderRequestId, ProviderId, ResourceTarget, DetailViewId) {
        (
            self.request_id,
            self.provider_id,
            self.target.resource,
            self.target.view_id,
        )
    }

    pub fn completion(self, result: Result<ResourceDetails, WorkspaceError>) -> DetailCompletion {
        DetailCompletion { load: self, result }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DetailCompletion {
    load: DetailLoad,
    result: Result<ResourceDetails, WorkspaceError>,
}

impl DetailCompletion {
    pub fn new(
        request_id: ProviderRequestId,
        provider_id: ProviderId,
        resource: ResourceTarget,
        view_id: DetailViewId,
        result: Result<ResourceDetails, WorkspaceError>,
    ) -> Self {
        Self {
            load: DetailLoad {
                request_id,
                provider_id,
                target: DetailTarget { resource, view_id },
            },
            result,
        }
    }
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

    pub fn id(&self) -> &ProviderId {
        &self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn target_environment(&self) -> &TargetEnvironment {
        &self.target_environment
    }

    pub fn version(&self) -> Option<&ProviderVersion> {
        self.version.as_ref()
    }

    pub fn focused_resource_panel(&self) -> Option<&ResourcePanelId> {
        self.focused_resource_panel.as_ref()
    }

    pub fn focused_resource_panel_index(&self) -> Option<usize> {
        let WorkspaceLoadState::Ready(snapshot) = &self.load_state else {
            return None;
        };
        let focused = self.focused_resource_panel.as_ref()?;
        snapshot
            .panels
            .iter()
            .position(|panel| &panel.id == focused)
    }

    /// Returns the number of Resource Panels when the workspace is ready.
    pub fn resource_panel_count(&self) -> Option<usize> {
        match &self.load_state {
            WorkspaceLoadState::Ready(snapshot) => Some(snapshot.panels.len()),
            WorkspaceLoadState::Loading | WorkspaceLoadState::Error(_) => None,
        }
    }

    /// Records a transient refresh failure while retaining stable presentation
    /// choices for the next successful reconciliation.
    pub fn record_load_error(&mut self, error: WorkspaceError) {
        self.load_state = WorkspaceLoadState::Error(error);
        self.invalidate_pending_detail();
    }

    /// Rejects a snapshot whose shape Virtui cannot represent.
    pub fn reject_snapshot(&mut self, error: WorkspaceError) {
        self.load_state = WorkspaceLoadState::Error(error);
        self.focused_resource_panel = None;
        self.panel_navigation.clear();
        self.selected_detail_view = None;
        self.details = None;
        self.pending_detail = None;
    }

    /// Refuses the pending detail result after its Provider Workspace is left.
    pub fn invalidate_pending_detail(&mut self) {
        self.pending_detail = None;
        if self
            .details
            .as_ref()
            .is_some_and(|details| details.content == DetailContent::Loading)
        {
            self.details = None;
        }
    }

    /// Focuses an existing Resource Panel and restores its remembered Resource
    /// selection. A stale panel identity changes nothing.
    pub fn focus_resource_panel(&mut self, panel_id: &ResourcePanelId) -> bool {
        self.invalidate_detail_when_target_changes(|workspace| {
            let WorkspaceLoadState::Ready(snapshot) = &workspace.load_state else {
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
            workspace.focused_resource_panel = Some(panel_id.clone());
            let still_offered = workspace
                .selected_detail_view
                .as_ref()
                .is_some_and(|selected| offered.contains(selected));
            if !still_offered {
                workspace.selected_detail_view = offered.into_iter().next();
            }
            true
        })
    }

    /// Focuses one Resource Panel by its Provider-defined order.
    pub fn focus_resource_panel_at(&mut self, index: usize) -> bool {
        let panel_id = match &self.load_state {
            WorkspaceLoadState::Ready(snapshot) => {
                snapshot.panels.get(index).map(|panel| panel.id.clone())
            }
            WorkspaceLoadState::Loading | WorkspaceLoadState::Error(_) => None,
        };
        panel_id.is_some_and(|panel_id| self.focus_resource_panel(&panel_id))
    }

    /// Moves the selected Resource within the focused panel and clamps at its
    /// ends. Every other panel keeps its own selection and scroll.
    pub fn move_resource_selection(&mut self, delta: isize) {
        self.invalidate_detail_when_target_changes(|workspace| {
            let WorkspaceLoadState::Ready(snapshot) = &workspace.load_state else {
                return;
            };
            let Some(panel_id) = workspace.focused_resource_panel.as_ref() else {
                return;
            };
            let Some(panel) = snapshot.panel(panel_id) else {
                return;
            };
            let Some(navigation) = workspace
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
        });
    }

    /// Moves through the focused panel's Detail Views as a ring.
    pub fn move_detail_view(&mut self, delta: isize) {
        self.invalidate_detail_when_target_changes(|workspace| {
            let WorkspaceLoadState::Ready(snapshot) = &workspace.load_state else {
                return;
            };
            let Some(panel_id) = workspace.focused_resource_panel.as_ref() else {
                return;
            };
            let Some(panel) = snapshot.panel(panel_id) else {
                return;
            };
            if panel.detail_views.is_empty() {
                return;
            }
            let current = workspace
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
            workspace.selected_detail_view =
                panel.detail_views.get(next).map(|view| view.id.clone());
        });
    }

    /// Starts a load only when the visible Resource and Detail View are not
    /// already loaded or pending. The application supplies the request ID and
    /// dispatches the returned work.
    pub fn start_visible_detail_load(
        &mut self,
        request_id: ProviderRequestId,
    ) -> Option<DetailLoad> {
        let Some((target, resource_name, view)) = self.detail_target() else {
            self.invalidate_pending_detail();
            self.details = None;
            return None;
        };
        let detail_target = DetailTarget {
            resource: target,
            view_id: view.id.clone(),
        };
        let pending_for_target = self
            .pending_detail
            .as_ref()
            .is_some_and(|pending| pending.target == detail_target);
        if !pending_for_target {
            self.pending_detail = None;
        }
        let describes_target = self
            .details
            .as_ref()
            .is_some_and(|details| details.target == detail_target);
        if pending_for_target || describes_target {
            return None;
        }

        let load = DetailLoad {
            request_id,
            provider_id: self.id.clone(),
            target: detail_target.clone(),
        };
        self.details = Some(ResourceDetailsState {
            target: detail_target,
            resource_name,
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

    /// Moves the first visible detail line without scrolling beyond content.
    pub fn scroll_details(&mut self, delta: isize) {
        let Some(details) = self.details.as_mut() else {
            return;
        };
        let last_line = match &details.content {
            DetailContent::Ready(loaded) => loaded.lines.len().saturating_sub(1),
            DetailContent::Loading | DetailContent::Error(_) => 0,
        };
        details.scroll = (details.scroll as usize)
            .saturating_add_signed(delta)
            .min(last_line) as u16;
    }

    /// Replaces Provider data while preserving every still-valid presentation
    /// choice by stable Provider identity.
    pub fn reconcile_snapshot(&mut self, snapshot: WorkspaceSnapshot) {
        self.invalidate_detail_when_target_changes(|workspace| {
            let previous = std::mem::take(&mut workspace.panel_navigation);
            workspace.panel_navigation = snapshot
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

            let focused_still_exists = workspace
                .focused_resource_panel
                .as_ref()
                .is_some_and(|focused| snapshot.panel(focused).is_some());
            if !focused_still_exists {
                workspace.focused_resource_panel =
                    snapshot.panels.first().map(|panel| panel.id.clone());
            }
            workspace.reconcile_detail_view(&snapshot);
            workspace.load_state = WorkspaceLoadState::Ready(snapshot);
        });
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
        let detail_views = selected_panel.map_or(&[][..], |panel| panel.detail_views.as_slice());
        let details = selected_target.as_ref().and_then(|selected| {
            self.details.as_ref().filter(|details| {
                details.target.resource == *selected
                    && Some(&details.target.view_id) == self.selected_detail_view.as_ref()
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
            detail_views,
            selected_detail_view,
            details: details.map(ResourceDetailsView::from),
        }
    }

    /// Projects the Provider Workspace into one coherent presentation state.
    pub fn presentation(&self) -> WorkspacePresentation<'_> {
        match &self.load_state {
            WorkspaceLoadState::Loading => WorkspacePresentation::Loading { name: &self.name },
            WorkspaceLoadState::Ready(snapshot) => {
                WorkspacePresentation::Ready(self.view(snapshot))
            }
            WorkspaceLoadState::Error(error) => WorkspacePresentation::Error {
                name: &self.name,
                error,
            },
        }
    }

    pub fn selected_resource_target(&self) -> Option<ResourceTarget> {
        let panel_id = self.focused_resource_panel.as_ref()?;
        let resource_id = self
            .panel_navigation
            .iter()
            .find(|navigation| &navigation.panel_id == panel_id)?
            .selected_resource
            .as_ref()?;
        Some(ResourceTarget::new(panel_id.clone(), resource_id.clone()))
    }

    pub fn selected_resource(&self) -> Option<&Resource> {
        let target = self.selected_resource_target()?;
        let WorkspaceLoadState::Ready(snapshot) = &self.load_state else {
            return None;
        };
        snapshot.resource(&target.panel_id, &target.resource_id)
    }

    fn invalidate_detail_when_target_changes<R>(
        &mut self,
        update: impl FnOnce(&mut Self) -> R,
    ) -> R {
        let previous_detail = self.visible_detail_identity();
        let result = update(self);
        if self.visible_detail_identity() != previous_detail {
            self.invalidate_pending_detail();
        }
        result
    }

    fn visible_detail_identity(&self) -> Option<DetailTarget> {
        Some(DetailTarget {
            resource: self.selected_resource_target()?,
            view_id: self.selected_detail_view.clone()?,
        })
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
    pub detail_views: &'a [DetailView],
    pub selected_detail_view: Option<&'a DetailView>,
    pub details: Option<ResourceDetailsView<'a>>,
}

pub enum WorkspacePresentation<'a> {
    Loading {
        name: &'a str,
    },
    Ready(WorkspaceView<'a>),
    Error {
        name: &'a str,
        error: &'a WorkspaceError,
    },
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
