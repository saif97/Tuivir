use std::collections::HashMap;

use crossterm::event::KeyEvent;

use crate::command::{Command, CommandRegistry, CommandScope};
use crate::keys::Key;
use crate::provider::{
    ProviderDiscovery, ProviderId, ProviderRequest, ProviderRequestId, ResourceCommand, ResourceId,
    ResourceState, WorkspaceError, WorkspaceSnapshot,
};

pub enum AppEvent {
    ProviderDiscovered(ProviderDiscovery),
    FocusProviders,
    FocusResources,
    ManualRefresh,
    RefreshTimerElapsed,
    SelectNextResource,
    SelectPreviousResource,
    SelectNextProvider,
    SelectPreviousProvider,
    ResourceCommandInvoked(ResourceCommand),
    ToggleHelp,
    ConfirmResourceCommand,
    CancelConfirmation,
    DismissCommandError,
    /// The result of an earlier [`ProviderRequest::RefreshWorkspace`].
    ///
    /// The application verifies the request is still pending before accepting
    /// this event.
    RefreshCompleted {
        request_id: ProviderRequestId,
        provider_id: ProviderId,
        result: Result<WorkspaceSnapshot, WorkspaceError>,
    },
    ResourceCommandCompleted {
        request_id: ProviderRequestId,
        provider_id: ProviderId,
        resource_id: ResourceId,
        resource_name: String,
        command: ResourceCommand,
        result: Result<(), WorkspaceError>,
    },
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum FocusedPanel {
    Providers,
    #[default]
    Resources,
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
    pub workspace_state: WorkspaceState,
    pub selected_resource: Option<ResourceId>,
}

#[derive(Default)]
pub struct AppState {
    pub providers: Vec<ProviderState>,
    pub focused_panel: FocusedPanel,
    /// Index into `providers` of the currently active provider — the one
    /// whose Provider Workspace is visible and being refreshed.
    ///
    /// `None` represents startup before any installed provider is discovered.
    pub active_provider: Option<usize>,
    pub help_overlay: Option<HelpOverlay>,
    pub confirmation: Option<ResourceCommandConfirmation>,
    pub command_error: Option<String>,
    /// Dispatched Resource Commands that have not completed yet, in dispatch
    /// order.
    ///
    /// Each entry stays attached to the Provider and Resource it was
    /// dispatched for until completion, so navigating to another Resource or
    /// Provider Workspace never discards one, and the shell can show a global
    /// progress status that identifies the original target.
    pub running_commands: Vec<RunningResourceCommand>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// One dispatched Resource Command awaiting its completion.
pub struct RunningResourceCommand {
    pub request_id: ProviderRequestId,
    pub provider_id: ProviderId,
    pub provider_name: String,
    pub resource_id: ResourceId,
    pub resource_name: String,
    pub command: ResourceCommand,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HelpEntry {
    pub key: String,
    pub description: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HelpOverlay {
    pub target: String,
    pub entries: Vec<HelpEntry>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResourceCommandConfirmation {
    pub provider_id: ProviderId,
    pub provider_name: String,
    pub resource_id: ResourceId,
    pub resource_name: String,
    pub command: ResourceCommand,
    /// What the Resource was doing when the Command was invoked, so the prompt
    /// can say what confirming will really do and the request can carry it on.
    pub state: ResourceState,
}

pub struct App {
    state: AppState,
    commands: CommandRegistry,
    /// Monotonically allocates request IDs in the main event loop.
    next_request_id: u64,
    /// Refresh requests whose snapshots may still update their Provider
    /// Workspace.
    ///
    /// Removing an entry invalidates a background snapshot without needing
    /// shared mutable state or cancellation handles in `AppState`. Navigating
    /// away from a Provider Workspace drops its entries so a stale snapshot
    /// cannot overwrite newer application state.
    pending_refreshes: HashMap<ProviderRequestId, ProviderId>,
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
            commands: CommandRegistry::default(),
            next_request_id: 1,
            pending_refreshes: HashMap::new(),
        }
    }

    pub fn state(&self) -> &AppState {
        &self.state
    }

    pub fn resource_command_for_key(&self, key: &KeyEvent) -> Option<ResourceCommand> {
        if self.state.focused_panel != FocusedPanel::Resources || self.state.help_overlay.is_some()
        {
            return None;
        }
        let Some(Command::Resource(command)) = self
            .commands
            .resolve(CommandScope::ResourceView, Key::from_event(*key)?)
        else {
            return None;
        };
        self.selected_resource()?
            .available_commands
            .contains(&command)
            .then_some(command)
    }

    /// Applies one application event and returns any provider work to run.
    ///
    /// This method performs no I/O; the runtime executes returned requests and
    /// feeds their completions back as events.
    pub fn update(&mut self, event: AppEvent) -> Vec<ProviderRequest> {
        match event {
            AppEvent::ProviderDiscovered(discovery) => self.handle_provider_discovered(discovery),
            AppEvent::FocusProviders => {
                self.state.focused_panel = FocusedPanel::Providers;
                Vec::new()
            }
            AppEvent::FocusResources => {
                self.state.focused_panel = FocusedPanel::Resources;
                Vec::new()
            }
            AppEvent::ManualRefresh | AppEvent::RefreshTimerElapsed => {
                self.refresh_active_provider()
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
            AppEvent::ResourceCommandInvoked(command) => self.handle_resource_command(command),
            AppEvent::ToggleHelp => {
                self.toggle_help();
                Vec::new()
            }
            AppEvent::ConfirmResourceCommand => self.confirm_resource_command(),
            AppEvent::CancelConfirmation => {
                self.state.confirmation = None;
                Vec::new()
            }
            AppEvent::DismissCommandError => {
                self.state.command_error = None;
                Vec::new()
            }
            AppEvent::RefreshCompleted {
                request_id,
                provider_id,
                result,
            } => self.apply_refresh_completed(request_id, provider_id, result),
            AppEvent::ResourceCommandCompleted {
                request_id,
                provider_id,
                resource_id,
                resource_name,
                command,
                result,
            } => self.apply_resource_command_result(
                request_id,
                provider_id,
                resource_id,
                resource_name,
                command,
                result,
            ),
        }
    }

    fn handle_provider_discovered(&mut self, discovery: ProviderDiscovery) -> Vec<ProviderRequest> {
        let activates_provider = self.state.active_provider.is_none();
        let should_refresh_active_provider = activates_provider && discovery.error.is_none();
        let initial_workspace_state = match discovery.error {
            Some(error) => WorkspaceState::Error(error),
            None => WorkspaceState::Loading,
        };
        let provider_id = discovery.id.clone();
        self.state.providers.push(ProviderState {
            id: discovery.id,
            name: discovery.name,
            target_environment: discovery.target_environment,
            workspace_state: initial_workspace_state,
            selected_resource: None,
        });
        if activates_provider {
            self.state.active_provider = Some(self.state.providers.len() - 1);
        }

        if should_refresh_active_provider {
            vec![self.start_refresh(provider_id)]
        } else {
            Vec::new()
        }
    }

    fn apply_refresh_completed(
        &mut self,
        request_id: ProviderRequestId,
        provider_id: ProviderId,
        result: Result<WorkspaceSnapshot, WorkspaceError>,
    ) -> Vec<ProviderRequest> {
        if self.pending_refreshes.remove(&request_id) != Some(provider_id.clone()) {
            return Vec::new();
        }
        let Some(provider) = self
            .state
            .providers
            .iter_mut()
            .find(|provider| provider.id == provider_id)
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
                provider.workspace_state = WorkspaceState::Ready(snapshot);
            }
            Err(error) => provider.workspace_state = WorkspaceState::Error(error),
        }
        Vec::new()
    }

    fn start_refresh(&mut self, provider_id: ProviderId) -> ProviderRequest {
        let request_id = ProviderRequestId(self.next_request_id);
        self.next_request_id += 1;
        self.pending_refreshes
            .insert(request_id, provider_id.clone());
        ProviderRequest::RefreshWorkspace {
            request_id,
            provider_id,
        }
    }

    fn apply_resource_command_result(
        &mut self,
        request_id: ProviderRequestId,
        provider_id: ProviderId,
        resource_id: ResourceId,
        resource_name: String,
        command: ResourceCommand,
        result: Result<(), WorkspaceError>,
    ) -> Vec<ProviderRequest> {
        let Some(running) = self
            .state
            .running_commands
            .iter()
            .position(|running| {
                running.request_id == request_id && running.provider_id == provider_id
            })
            .map(|index| self.state.running_commands.remove(index))
        else {
            return Vec::new();
        };
        let provider_name = running.provider_name;
        if let Err(error) = result {
            self.state.command_error = Some(format!(
                "{provider_name} {command} failed for {resource_name} ({resource_id}): {}",
                error.message
            ));
            return Vec::new();
        }
        self.state.command_error = None;
        if !self.is_active_provider(&provider_id) {
            return Vec::new();
        }
        self.refresh_active_provider()
    }

    fn is_active_provider(&self, provider_id: &ProviderId) -> bool {
        self.state
            .active_provider
            .and_then(|active| self.state.providers.get(active))
            .is_some_and(|provider| &provider.id == provider_id)
    }

    fn handle_resource_command(&mut self, command: ResourceCommand) -> Vec<ProviderRequest> {
        let Some(provider) = self
            .state
            .active_provider
            .and_then(|active| self.state.providers.get(active))
        else {
            return Vec::new();
        };
        let Some(resource_id) = provider.selected_resource.clone() else {
            return Vec::new();
        };
        let WorkspaceState::Ready(snapshot) = &provider.workspace_state else {
            return Vec::new();
        };
        let Some(resource) = snapshot
            .resources()
            .find(|resource| resource.id == resource_id)
        else {
            return Vec::new();
        };
        if !resource.available_commands.contains(&command) {
            return Vec::new();
        }
        let provider_id = provider.id.clone();
        let provider_name = provider.name.clone();
        let resource_name = resource.name.clone();
        let state = resource.state;
        if command == ResourceCommand::Delete {
            self.state.confirmation = Some(ResourceCommandConfirmation {
                provider_id,
                provider_name,
                resource_id,
                resource_name,
                command,
                state,
            });
            return Vec::new();
        }
        self.dispatch_resource_command(
            provider_id,
            provider_name,
            resource_id,
            resource_name,
            command,
            state,
        )
    }

    fn confirm_resource_command(&mut self) -> Vec<ProviderRequest> {
        let Some(confirmation) = self.state.confirmation.take() else {
            return Vec::new();
        };
        self.dispatch_resource_command(
            confirmation.provider_id,
            confirmation.provider_name,
            confirmation.resource_id,
            confirmation.resource_name,
            confirmation.command,
            confirmation.state,
        )
    }

    fn dispatch_resource_command(
        &mut self,
        provider_id: ProviderId,
        provider_name: String,
        resource_id: ResourceId,
        resource_name: String,
        command: ResourceCommand,
        state: ResourceState,
    ) -> Vec<ProviderRequest> {
        self.state.command_error = None;
        let request_id = ProviderRequestId(self.next_request_id);
        self.next_request_id += 1;
        self.state.running_commands.push(RunningResourceCommand {
            request_id,
            provider_id: provider_id.clone(),
            provider_name,
            resource_id: resource_id.clone(),
            resource_name: resource_name.clone(),
            command,
        });

        vec![ProviderRequest::ExecuteResourceCommand {
            request_id,
            provider_id,
            resource_id,
            resource_name,
            command,
            state,
        }]
    }

    fn toggle_help(&mut self) {
        if self.state.help_overlay.take().is_some() {
            return;
        }
        if self.state.focused_panel != FocusedPanel::Resources {
            return;
        }
        let Some(resource) = self.selected_resource() else {
            return;
        };
        let target = resource.name.clone();
        let available_commands = resource.available_commands.clone();
        self.state.help_overlay = Some(HelpOverlay {
            target,
            entries: self
                .commands
                .in_scope(CommandScope::ResourceView)
                .map(|registered| HelpEntry {
                    key: registered
                        .keys
                        .first()
                        .expect("a Command in scope is bound")
                        .to_string(),
                    description: match registered.command {
                        Command::Resource(command)
                            if !available_commands.contains(&command) =>
                        {
                            format!("{} (unavailable)", registered.description)
                        }
                        _ => registered.description.to_owned(),
                    },
                })
                .collect(),
        });
    }

    fn selected_resource(&self) -> Option<&crate::provider::Resource> {
        let provider = self
            .state
            .active_provider
            .and_then(|active| self.state.providers.get(active))?;
        let selected = provider.selected_resource.as_ref()?;
        let WorkspaceState::Ready(snapshot) = &provider.workspace_state else {
            return None;
        };
        snapshot
            .resources()
            .find(|resource| &resource.id == selected)
    }

    fn refresh_active_provider(&mut self) -> Vec<ProviderRequest> {
        let Some(active_provider) = self.state.active_provider else {
            return Vec::new();
        };
        let Some(provider_id) = self
            .state
            .providers
            .get(active_provider)
            .map(|provider| provider.id.clone())
        else {
            return Vec::new();
        };
        if self
            .pending_refreshes
            .values()
            .any(|pending| pending == &provider_id)
        {
            return Vec::new();
        }
        vec![self.start_refresh(provider_id)]
    }

    fn move_resource_selection(&mut self, delta: isize) {
        let Some(active_provider) = self.state.active_provider else {
            return;
        };
        let Some(provider) = self.state.providers.get_mut(active_provider) else {
            return;
        };
        let WorkspaceState::Ready(snapshot) = &provider.workspace_state else {
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

    fn move_provider_selection(&mut self, delta: isize) -> Vec<ProviderRequest> {
        let provider_count = self.state.providers.len();
        if provider_count < 2 {
            return Vec::new();
        }
        let Some(active_provider) = self.state.active_provider else {
            return Vec::new();
        };
        let previous_provider = self.state.providers[active_provider].id.clone();
        self.pending_refreshes
            .retain(|_, provider_id| provider_id != &previous_provider);
        self.state.active_provider =
            Some((active_provider as isize + delta).rem_euclid(provider_count as isize) as usize);
        self.refresh_active_provider()
    }
}
