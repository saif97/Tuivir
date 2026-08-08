use crate::{
    application::{
        DetailView, ProviderRequestId, Resource, ResourceDetails, ResourcePanel, WorkspaceError,
        WorkspaceSnapshot,
    },
    domain::{
        DetailViewId, Provider, ProviderId, ProviderVersion, ResourceId, ResourcePanelId,
        ResourceTarget, TargetEnvironment,
    },
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
    /// The selected Resource's index in the latest snapshot.
    selected_index: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DetailContent {
    Loading,
    Ready(ResourceDetails),
    Error(WorkspaceError),
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ResourceDetailsState {
    request_id: Option<ProviderRequestId>,
    provider_id: ProviderId,
    target: DetailTarget,
    resource_name: String,
    title: String,
    content: DetailContent,
    scroll: u16,
    selection: Option<DetailSelection>,
}

/// A half-open source range inside loaded Details text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DetailSelection {
    pub start: DetailPosition,
    pub end: DetailPosition,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct DetailPosition {
    pub line: u16,
    pub column: u16,
}

impl ResourceDetailsState {
    fn is_loading_request(
        &self,
        request_id: ProviderRequestId,
        provider_id: &ProviderId,
        target: &ResourceTarget,
        view_id: &DetailViewId,
    ) -> bool {
        self.content == DetailContent::Loading
            && self.request_id == Some(request_id)
            && &self.provider_id == provider_id
            && &self.target.resource == target
            && &self.target.view_id == view_id
    }

    fn is_loading(&self, load: &DetailLoad) -> bool {
        self.is_loading_request(
            load.request_id,
            &load.provider_id,
            &load.target.resource,
            &load.target.view_id,
        )
    }
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
/// All presentation-neutral state for one Provider Workspace.
pub struct ProviderWorkspaceState {
    provider: Provider,
    load_state: WorkspaceLoadState,
    focused_resource_panel: Option<ResourcePanelId>,
    panel_navigation: Vec<ResourcePanelNavigation>,
    selected_detail_view: Option<DetailViewId>,
    details: Option<ResourceDetailsState>,
}

impl ProviderWorkspaceState {
    pub fn new(provider: Provider, error: Option<WorkspaceError>) -> Self {
        let load_state = error.map_or(WorkspaceLoadState::Loading, WorkspaceLoadState::Error);
        Self {
            provider,
            load_state,
            focused_resource_panel: None,
            panel_navigation: Vec::new(),
            selected_detail_view: None,
            details: None,
        }
    }

    pub fn load_state(&self) -> &WorkspaceLoadState {
        &self.load_state
    }

    pub fn id(&self) -> &ProviderId {
        self.provider.id()
    }

    pub fn name(&self) -> &str {
        self.provider.name()
    }

    pub fn target_environment(&self) -> Option<&TargetEnvironment> {
        self.provider.target_environment()
    }

    pub fn version(&self) -> Option<&ProviderVersion> {
        self.provider.version()
    }

    /// Checks the private, authoritative Detail load identity without exposing
    /// a second pending-request state to the host.
    pub(crate) fn is_loading_detail(
        &self,
        request_id: ProviderRequestId,
        provider_id: &ProviderId,
        target: &ResourceTarget,
        view_id: &DetailViewId,
    ) -> bool {
        self.details.as_ref().is_some_and(|details| {
            details.is_loading_request(request_id, provider_id, target, view_id)
        })
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
    }

    /// Refuses the pending detail result after its Provider Workspace is left.
    pub fn invalidate_pending_detail(&mut self) {
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
            workspace.focused_resource_panel = Some(panel_id.clone());
            let still_offered = workspace
                .selected_detail_view
                .as_ref()
                .is_some_and(|selected| panel.detail_views.iter().any(|view| &view.id == selected));
            if !still_offered {
                workspace.selected_detail_view =
                    panel.detail_views.first().map(|view| view.id.clone());
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
        let Some(panel_id) = self.focused_resource_panel.clone() else {
            return;
        };
        self.move_selection_within(&panel_id, delta);
    }

    /// Moves the selection inside one named Resource Panel.
    ///
    /// The panel is named rather than taken from focus, so moving a Panel the
    /// user is only pointing at cannot be mistaken for moving the focused one —
    /// and so cannot abandon the focused Resource's detail load.
    fn move_selection_within(&mut self, panel_id: &ResourcePanelId, delta: isize) {
        self.invalidate_detail_when_target_changes(|workspace| {
            let WorkspaceLoadState::Ready(snapshot) = &workspace.load_state else {
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
                // Nothing valid was selected, so start at the top.
                navigation.selected_resource =
                    panel.resources.first().map(|resource| resource.id.clone());
                navigation.selected_index = 0;
                return;
            };
            let next = current
                .saturating_add_signed(delta)
                .min(panel.resources.len().saturating_sub(1));
            navigation.selected_resource = panel
                .resources
                .get(next)
                .map(|resource| resource.id.clone());
            navigation.selected_index = next;
        });
    }

    /// Selects a Resource by its current Provider order without changing any
    /// other panel's remembered selection or scroll position.
    pub fn select_resource_at(&mut self, index: usize) {
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
            let Some(resource) = panel.resources.get(index) else {
                return;
            };
            let Some(navigation) = workspace
                .panel_navigation
                .iter_mut()
                .find(|navigation| &navigation.panel_id == panel_id)
            else {
                return;
            };
            navigation.selected_resource = Some(resource.id.clone());
            navigation.selected_index = index;
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

    pub fn select_detail_view_at(&mut self, index: usize) {
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
            workspace.selected_detail_view =
                panel.detail_views.get(index).map(|view| view.id.clone());
        });
    }

    /// Moves the selection inside the Resource Panel at one provider-defined
    /// position, leaving focus where the keyboard left it.
    pub fn move_resource_selection_at(&mut self, panel_index: usize, delta: isize) {
        let Some(panel_id) = (match &self.load_state {
            WorkspaceLoadState::Ready(snapshot) => snapshot
                .panels
                .get(panel_index)
                .map(|panel| panel.id.clone()),
            WorkspaceLoadState::Loading | WorkspaceLoadState::Error(_) => None,
        }) else {
            return;
        };
        self.move_selection_within(&panel_id, delta);
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
        let describes_target = self
            .details
            .as_ref()
            .is_some_and(|details| details.target == detail_target);
        if describes_target {
            return None;
        }

        let snapshot_content = match &self.load_state {
            WorkspaceLoadState::Ready(snapshot) => {
                snapshot.snapshot_detail(&detail_target.resource, &detail_target.view_id)
            }
            WorkspaceLoadState::Loading | WorkspaceLoadState::Error(_) => None,
        };
        if let Some(content) = snapshot_content {
            self.details = Some(ResourceDetailsState {
                request_id: None,
                provider_id: self.provider.id().clone(),
                target: detail_target,
                resource_name,
                title: view.title,
                content: DetailContent::Ready(content),
                scroll: 0,
                selection: None,
            });
            return None;
        }

        let provider_id = self.provider.id().clone();
        let load = DetailLoad {
            request_id,
            provider_id: provider_id.clone(),
            target: detail_target.clone(),
        };
        self.details = Some(ResourceDetailsState {
            request_id: Some(request_id),
            provider_id,
            target: detail_target,
            resource_name,
            title: view.title,
            content: DetailContent::Loading,
            scroll: 0,
            selection: None,
        });
        Some(load)
    }

    /// Accepts a detail completion only while its full request identity still
    /// describes the visible load.
    pub fn complete_detail_load(&mut self, completion: DetailCompletion) {
        let Some(details) = self.details.as_mut() else {
            return;
        };
        if !details.is_loading(&completion.load) {
            return;
        }
        details.request_id = None;
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

    pub fn begin_detail_selection(&mut self, line: u16, column: u16) {
        let Some(details) = self.details.as_mut() else { return };
        let line = details.scroll.saturating_add(line);
        let Some(position) = detail_position(&details.content, line, column) else { return };
        details.selection = Some(DetailSelection { start: position, end: position });
    }

    pub fn extend_detail_selection(&mut self, line: u16, column: u16) {
        let Some(details) = self.details.as_mut() else { return };
        let line = details.scroll.saturating_add(line);
        let Some(position) = detail_position(&details.content, line, column) else { return };
        if let Some(selection) = &mut details.selection {
            selection.end = position;
        }
    }

    pub fn clear_detail_selection(&mut self) {
        if let Some(details) = self.details.as_mut() {
            details.selection = None;
        }
    }

    pub fn selected_detail_text(&self) -> Option<String> {
        let details = self.details.as_ref()?;
        let DetailContent::Ready(loaded) = &details.content else { return None };
        let selection = details.selection.as_ref()?;
        selected_text(&loaded.lines, selection)
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
                        selected_index,
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
            .and_then(|selected| snapshot.resource(selected));
        let selected_panel = selected_target
            .as_ref()
            .and_then(|selected| snapshot.panel_for(selected));
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
            id: self.provider.id(),
            name: self.provider.name(),
            target_environment: self.provider.target_environment(),
            version: self.provider.version(),
            focused_resource_panel: self.focused_resource_panel.as_ref(),
            snapshot,
            panel_navigation: &self.panel_navigation,
            selected_resource,
            detail_views,
            selected_detail_view,
            details: details.map(ResourceDetailsView::from),
        }
    }

    /// Projects the Provider Workspace into one coherent presentation state.
    pub fn presentation(&self) -> WorkspacePresentation<'_> {
        match &self.load_state {
            WorkspaceLoadState::Loading => WorkspacePresentation::Loading {
                name: self.provider.name(),
            },
            WorkspaceLoadState::Ready(snapshot) => {
                WorkspacePresentation::Ready(self.view(snapshot))
            }
            WorkspaceLoadState::Error(error) => WorkspacePresentation::Error {
                name: self.provider.name(),
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
        snapshot.resource(&target)
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
            .and_then(|selected| snapshot.panel_for(selected))
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
        let resource = snapshot.resource(&target)?;
        let view_id = self.selected_detail_view.as_ref()?;
        let view = snapshot
            .panel_for(&target)?
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
    pub target_environment: Option<&'a TargetEnvironment>,
    pub version: Option<&'a ProviderVersion>,
    pub focused_resource_panel: Option<&'a ResourcePanelId>,
    snapshot: &'a WorkspaceSnapshot,
    panel_navigation: &'a [ResourcePanelNavigation],
    pub selected_resource: Option<&'a Resource>,
    pub detail_views: &'a [DetailView],
    pub selected_detail_view: Option<&'a DetailView>,
    pub details: Option<ResourceDetailsView<'a>>,
}

impl<'a> WorkspaceView<'a> {
    /// Projects panels lazily so every frame borrows application state without
    /// allocating an intermediate collection.
    pub fn panels(&self) -> impl ExactSizeIterator<Item = ResourcePanelView<'a>> + '_ {
        self.snapshot.panels.iter().map(|panel| {
            let navigation = self
                .panel_navigation
                .iter()
                .find(|navigation| navigation.panel_id == panel.id);
            ResourcePanelView {
                panel,
                selected_resource: navigation
                    .and_then(|navigation| navigation.selected_resource.as_ref()),
                selected_index: navigation.map_or(0, |navigation| navigation.selected_index),
            }
        })
    }
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
    pub selection: Option<&'a DetailSelection>,
}

impl<'a> From<&'a ResourceDetailsState> for ResourceDetailsView<'a> {
    fn from(details: &'a ResourceDetailsState) -> Self {
        Self {
            resource_name: &details.resource_name,
            title: &details.title,
            content: &details.content,
            scroll: details.scroll,
            selection: details.selection.as_ref(),
        }
    }
}

fn detail_position(content: &DetailContent, line: u16, column: u16) -> Option<DetailPosition> {
    let DetailContent::Ready(loaded) = content else { return None };
    let text = loaded.lines.get(line as usize)?;
    Some(DetailPosition { line, column: column.min(text.chars().count() as u16) })
}

fn selected_text(lines: &[String], selection: &DetailSelection) -> Option<String> {
    let (start, end) = if selection.start <= selection.end {
        (selection.start, selection.end)
    } else {
        (selection.end, selection.start)
    };
    if start == end || end.line as usize >= lines.len() { return None }
    Some((start.line..=end.line).enumerate().map(|(offset, line)| {
        let text = &lines[line as usize];
        let from = if offset == 0 { start.column as usize } else { 0 };
        let to = if line == end.line { end.column as usize } else { text.chars().count() };
        text.chars().skip(from).take(to.saturating_sub(from)).collect::<String>()
    }).collect::<Vec<_>>().join("\n"))
}

/// One Resource Panel paired with its private navigation projection.
#[derive(Clone, Copy)]
pub struct ResourcePanelView<'a> {
    pub panel: &'a ResourcePanel,
    pub selected_resource: Option<&'a ResourceId>,
    pub selected_index: usize,
}
