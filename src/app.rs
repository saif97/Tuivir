use std::collections::HashMap;

use crate::provider::{
    ProviderAction, ProviderDiscovery, ProviderId, ProviderRequest, ProviderRequestId, ResourceId,
    WorkspaceError, WorkspaceSnapshot,
};

pub enum AppEvent {
    ProviderDiscovered(ProviderDiscovery),
    ManualRefresh,
    RefreshTimerElapsed,
    SelectNextResource,
    SelectPreviousResource,
    SelectNextProvider,
    SelectPreviousProvider,
    /// The result of an earlier [`ProviderAction::RefreshWorkspace`].
    ///
    /// The application verifies the request is still pending before accepting
    /// this event.
    RefreshCompleted {
        request: ProviderRequest,
        result: Result<WorkspaceSnapshot, WorkspaceError>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// The lifecycle state of one Provider Workspace.
pub enum WorkspaceState {
    Loading,
    Ready(WorkspaceSnapshot),
    Error(WorkspaceError),
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// UI-neutral state for one discovered provider.
pub struct ProviderState {
    pub id: ProviderId,
    pub name: String,
    pub target_environment: String,
    pub workspace: WorkspaceState,
    pub selected_resource: Option<ResourceId>,
}

#[derive(Default)]
pub struct AppState {
    pub providers: Vec<ProviderState>,
    /// The single Provider Workspace currently visible and being refreshed.
    ///
    /// `None` represents startup before any installed provider is discovered.
    pub active_workspace: Option<usize>,
}

pub struct App {
    state: AppState,
    /// Monotonically allocates request IDs in the main event loop.
    next_request_id: u64,
    /// Requests whose completions may still update their Provider Workspace.
    ///
    /// Removing an entry invalidates a background completion without needing
    /// shared mutable state or cancellation handles in `AppState`.
    pending: HashMap<ProviderRequestId, ProviderId>,
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl App {
    pub fn new() -> Self {
        Self {
            state: AppState::default(),
            next_request_id: 1,
            pending: HashMap::new(),
        }
    }

    pub fn state(&self) -> &AppState {
        &self.state
    }

    /// Applies one application event and returns any provider work to run.
    ///
    /// This method performs no I/O; the runtime executes returned actions and
    /// feeds their completions back as events.
    pub fn update(&mut self, event: AppEvent) -> Vec<ProviderAction> {
        match event {
            AppEvent::ProviderDiscovered(discovery) => self.handle_provider_discovered(discovery),
            AppEvent::ManualRefresh | AppEvent::RefreshTimerElapsed => {
                self.refresh_active_workspace()
            }
            AppEvent::SelectNextResource => {
                self.move_resource_selection(1);
                Vec::new()
            }
            AppEvent::SelectPreviousResource => {
                self.move_resource_selection(-1);
                Vec::new()
            }
            AppEvent::SelectNextProvider => self.move_provider_selection(1),
            AppEvent::SelectPreviousProvider => self.move_provider_selection(-1),
            AppEvent::RefreshCompleted { request, result } => {
                self.apply_refresh_completed(request, result)
            }
        }
    }

    fn handle_provider_discovered(&mut self, discovery: ProviderDiscovery) -> Vec<ProviderAction> {
        let ProviderDiscovery {
            id,
            name,
            target_environment,
            error,
        } = discovery;
        let activates_workspace = self.state.active_workspace.is_none();
        let should_refresh_active_workspace = activates_workspace && error.is_none();
        let initial_workspace_state = match error {
            Some(error) => WorkspaceState::Error(error),
            None => WorkspaceState::Loading,
        };
        self.state.providers.push(ProviderState {
            id: id.clone(),
            name,
            target_environment,
            workspace: initial_workspace_state,
            selected_resource: None,
        });
        if activates_workspace {
            self.state.active_workspace = Some(self.state.providers.len() - 1);
        }

        if should_refresh_active_workspace {
            vec![ProviderAction::RefreshWorkspace(self.start_refresh(id))]
        } else {
            Vec::new()
        }
    }

    fn apply_refresh_completed(
        &mut self,
        request: ProviderRequest,
        result: Result<WorkspaceSnapshot, WorkspaceError>,
    ) -> Vec<ProviderAction> {
        if self.pending.remove(&request.id) != Some(request.provider_id.clone()) {
            return Vec::new();
        }
        let Some(provider) = self
            .state
            .providers
            .iter_mut()
            .find(|provider| provider.id == request.provider_id)
        else {
            return Vec::new();
        };
        match result {
            Ok(snapshot) => {
                let selected_still_exists =
                    provider.selected_resource.as_ref().is_some_and(|selected| {
                        snapshot
                            .resources()
                            .any(|resource| &resource.id == selected)
                    });
                if !selected_still_exists {
                    provider.selected_resource = snapshot
                        .panels
                        .first()
                        .and_then(|panel| panel.resources.first())
                        .map(|resource| resource.id.clone());
                }
                provider.workspace = WorkspaceState::Ready(snapshot);
            }
            Err(error) => provider.workspace = WorkspaceState::Error(error),
        }
        Vec::new()
    }

    fn start_refresh(&mut self, provider_id: ProviderId) -> ProviderRequest {
        let id = ProviderRequestId(self.next_request_id);
        self.next_request_id += 1;
        self.pending.insert(id, provider_id.clone());
        ProviderRequest { id, provider_id }
    }

    fn refresh_active_workspace(&mut self) -> Vec<ProviderAction> {
        let Some(active_workspace) = self.state.active_workspace else {
            return Vec::new();
        };
        let Some(provider_id) = self
            .state
            .providers
            .get(active_workspace)
            .map(|provider| provider.id.clone())
        else {
            return Vec::new();
        };
        if self.pending.values().any(|pending| pending == &provider_id) {
            return Vec::new();
        }
        vec![ProviderAction::RefreshWorkspace(
            self.start_refresh(provider_id),
        )]
    }

    fn move_resource_selection(&mut self, delta: isize) {
        let Some(active_workspace) = self.state.active_workspace else {
            return;
        };
        let Some(provider) = self.state.providers.get_mut(active_workspace) else {
            return;
        };
        let WorkspaceState::Ready(snapshot) = &provider.workspace else {
            return;
        };
        let resources = snapshot.resources().collect::<Vec<_>>();
        let Some(current) = provider.selected_resource.as_ref().and_then(|selected| {
            resources
                .iter()
                .position(|resource| &resource.id == selected)
        }) else {
            provider.selected_resource = resources.first().map(|resource| resource.id.clone());
            return;
        };
        let next = current
            .saturating_add_signed(delta)
            .min(resources.len().saturating_sub(1));
        provider.selected_resource = resources.get(next).map(|resource| resource.id.clone());
    }

    fn move_provider_selection(&mut self, delta: isize) -> Vec<ProviderAction> {
        let provider_count = self.state.providers.len();
        if provider_count < 2 {
            return Vec::new();
        }
        let Some(active_workspace) = self.state.active_workspace else {
            return Vec::new();
        };
        let previous_provider = self.state.providers[active_workspace].id.clone();
        self.pending
            .retain(|_, provider_id| provider_id != &previous_provider);
        self.state.active_workspace =
            Some((active_workspace as isize + delta).rem_euclid(provider_count as isize) as usize);
        self.refresh_active_workspace()
    }
}
