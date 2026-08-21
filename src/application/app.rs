use std::collections::HashMap;

use super::workspace::{DetailCompletion, ProviderWorkspaceState};
use super::{
    Command, CommandRegistry, CommandScope, Key, NUMBERED_RESOURCE_PANEL_CAPACITY, PaneBoundary,
    ProviderRequest, ProviderRequestId, ResourceCommand, ResourceDetails, ResourceShellEffect,
    ResourceShellSession, ResourceShellSessionId, ResourceShellSessionLifecycle, WorkspaceError,
    WorkspaceSnapshot,
};
use crate::domain::{DetailViewId, Provider, ProviderId, ResourceState, ResourceTarget};

/// Facts the application receives: provider discovery, the refresh clock, and
/// asynchronous completions.
///
/// User intentions are [`Command`]s, resolved from keys, not events. Keeping
/// the two separate means a keypress never looks like a completed refresh.
pub enum AppEvent {
    ProviderDiscovered {
        provider: Provider,
        error: Option<WorkspaceError>,
    },
    RefreshTimerElapsed,
    /// The result of an earlier [`ProviderRequest::RefreshWorkspace`].
    ///
    /// The application verifies the request is still pending before accepting
    /// this event.
    RefreshCompleted {
        request_id: ProviderRequestId,
        provider_id: ProviderId,
        result: Result<WorkspaceSnapshot, WorkspaceError>,
    },
    /// The host created the live PTY runtime for this application-owned
    /// Resource Shell Session identity.
    ResourceShellStarted {
        session_id: ResourceShellSessionId,
    },
    /// The host could not create the PTY or provider process. The session
    /// remains visible so Enter can retry it.
    ResourceShellStartFailed {
        session_id: ResourceShellSessionId,
        reason: String,
    },
    /// The private child process ended after it had started.
    ResourceShellExited {
        session_id: ResourceShellSessionId,
    },
    ResourceCommandCompleted {
        request_id: ProviderRequestId,
        provider_id: ProviderId,
        target: ResourceTarget,
        command: ResourceCommand,
        result: Result<(), WorkspaceError>,
    },
    /// The result of an earlier [`ProviderRequest::LoadResourceDetails`].
    ///
    /// Accepted only while its request is still the pending one for the visible
    /// Resource and view, so a result the user has navigated away from is
    /// dropped rather than rendered.
    ResourceDetailsCompleted {
        request_id: ProviderRequestId,
        provider_id: ProviderId,
        target: ResourceTarget,
        view_id: DetailViewId,
        result: Result<ResourceDetails, WorkspaceError>,
    },
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum FocusedPane {
    Providers,
    /// The focused Resource Panel is owned by the Active Workspace.
    #[default]
    Resources,
    Details,
}

/// How far one scroll Command moves through a detail view. Rendering owns the
/// layout, so a fixed step is the honest one: the application has no viewport
/// height to take a page from.
const DETAIL_SCROLL_LINES: isize = 10;

/// One sentence for every operation Tuivir asked a Provider for and did not
/// get.
///
/// Lifecycle Commands and Interactive Shells fail in different places and are
/// worded by different code, but they identify their target the same way, so
/// the user reads one sentence rather than two dialects of one.
fn operation_failure(
    provider_name: &str,
    operation: &str,
    resource_name: &str,
    target: &ResourceTarget,
    reason: &str,
) -> String {
    format!("{provider_name} {operation} failed for {resource_name} ({target}): {reason}")
}

#[derive(Default)]
pub struct AppState {
    pub providers: Vec<ProviderWorkspaceState>,
    pub focused_pane: FocusedPane,
    /// Index into `providers` of the currently active provider — the one
    /// whose Provider Workspace is visible and being refreshed.
    ///
    /// `None` represents startup before any installed provider is discovered.
    pub active_provider: Option<usize>,
    /// Where the Resource Panels give way to the Details Pane.
    ///
    /// One share for the whole run, so moving between Provider Workspaces finds
    /// the Panes the size the user left them.
    pub pane_boundary: PaneBoundary,
    pub help_overlay: Option<HelpOverlay>,
    pub confirmation: Option<ResourceCommandInvocation>,
    pub command_error: Option<String>,
    /// Stable Resource Shell Session identities and user-visible lifecycles.
    /// The host owns all live terminal and process objects keyed by these IDs.
    pub resource_shell_sessions: Vec<ResourceShellSession>,
    /// Dispatched Resource Commands that have not completed yet, in dispatch
    /// order.
    ///
    /// Each entry stays attached to the Provider and Resource it was
    /// dispatched for until completion, so navigating to another Resource or
    /// Provider Workspace never discards one, and the shell can show a global
    /// progress status that identifies the original target.
    pub running_commands: Vec<RunningResourceCommand>,
    /// The first effective binding for each Command whose key is shown inline,
    /// derived from the same registry that drives dispatch and help so the
    /// rendered hints cannot drift. `None` means the Command is unbound and its
    /// hint is omitted.
    pub hints: KeyHints,
    /// A compact, scope-correct subset of Commands shown at the bottom of the
    /// screen. The help overlay remains the complete reference.
    pub command_bar: Vec<HelpEntry>,
    #[doc(hidden)]
    pub pending_details_copy: Option<String>,
}

impl AppState {
    /// Returns the single Provider Workspace currently visible to the user.
    pub fn active_workspace(&self) -> Option<&ProviderWorkspaceState> {
        self.active_provider
            .and_then(|active| self.providers.get(active))
    }

    /// The session whose Shell Detail View Tab is currently visible, if it has
    /// already been explicitly started.
    pub fn visible_resource_shell_session(&self) -> Option<&ResourceShellSession> {
        let workspace = self.active_workspace()?;
        if !workspace.selected_resource_shell_tab() {
            return None;
        }
        let target = workspace.selected_resource_target()?;
        self.resource_shell_sessions
            .iter()
            .find(|session| session.provider_id == *workspace.id() && session.target == target)
    }

    fn active_workspace_mut(&mut self) -> Option<&mut ProviderWorkspaceState> {
        self.active_provider
            .and_then(|active| self.providers.get_mut(active))
    }
}

/// First effective bindings projected for inline display.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct KeyHints {
    pub focus_providers: Option<String>,
    pub focus_resource_panels: Vec<Option<String>>,
    pub focus_details: Option<String>,
}

impl KeyHints {
    fn from_registry(registry: &CommandRegistry) -> Self {
        Self {
            focus_providers: registry
                .first_key(Command::FocusProviders)
                .map(|key| key.to_string()),
            focus_resource_panels: (0..NUMBERED_RESOURCE_PANEL_CAPACITY)
                .map(|index| {
                    registry
                        .first_key(Command::FocusResourcePanel(index))
                        .map(|key| key.to_string())
                })
                .collect(),
            focus_details: registry
                .first_key(Command::FocusDetails)
                .map(|key| key.to_string()),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// One dispatched Resource Command awaiting its completion.
pub struct RunningResourceCommand {
    pub request_id: ProviderRequestId,
    pub provider_id: ProviderId,
    pub provider_name: String,
    pub target: ResourceTarget,
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
pub struct ResourceCommandInvocation {
    pub provider_id: ProviderId,
    pub provider_name: String,
    pub target: ResourceTarget,
    pub resource_name: String,
    pub command: ResourceCommand,
    /// What the Resource was doing when the Command was invoked, so the prompt
    /// can say what confirming will really do and the request can carry it on.
    pub state: Option<ResourceState>,
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
    /// The preference at the start of the current drag, if any.
    pane_boundary_before_drag: Option<PaneBoundary>,
    /// A completed user resize for the host to persist outside application
    /// state, following the same effect-taking pattern as clipboard work.
    pending_pane_boundary_save: Option<PaneBoundary>,
    next_resource_shell_session_id: u64,
    pending_resource_shell_effects: Vec<ResourceShellEffect>,
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl App {
    pub fn new() -> Self {
        Self::with_registry(CommandRegistry::default())
    }

    /// Builds the application around an effective registry, projecting its
    /// first bindings into the state the renderer reads.
    pub fn with_registry(commands: CommandRegistry) -> Self {
        Self::with_registry_and_pane_boundary(commands, PaneBoundary::default())
    }

    /// Builds the application with the durable Pane Boundary restored by the
    /// host before it owns the terminal.
    pub fn with_registry_and_pane_boundary(
        commands: CommandRegistry,
        pane_boundary: PaneBoundary,
    ) -> Self {
        let hints = KeyHints::from_registry(&commands);
        let state = AppState {
            hints,
            pane_boundary,
            ..AppState::default()
        };
        Self {
            state,
            commands,
            next_request_id: 1,
            pending_refreshes: HashMap::new(),
            pane_boundary_before_drag: None,
            pending_pane_boundary_save: None,
            next_resource_shell_session_id: 1,
            pending_resource_shell_effects: Vec::new(),
        }
    }

    pub fn state(&self) -> &AppState {
        &self.state
    }

    /// Confirms that a debounced Detail View Tab request still describes the
    /// visible load before the host starts Provider work for it.
    pub fn detail_request_is_current(&self, request: &ProviderRequest) -> bool {
        let ProviderRequest::LoadResourceDetails {
            request_id,
            provider_id,
            target,
            view_id,
        } = request
        else {
            return false;
        };
        self.state.active_workspace().is_some_and(|workspace| {
            workspace.is_loading_detail(*request_id, provider_id, target, view_id)
        })
    }

    /// Applies one application event and returns any provider work to run.
    ///
    /// This method performs no I/O; the runtime executes returned requests and
    /// feeds their completions back as events.
    pub fn update(&mut self, event: AppEvent) -> Vec<ProviderRequest> {
        let mut requests = self.apply(event);
        requests.extend(self.sync_details());
        self.update_command_bar();
        requests
    }

    fn apply(&mut self, event: AppEvent) -> Vec<ProviderRequest> {
        match event {
            AppEvent::ProviderDiscovered { provider, error } => {
                self.handle_provider_discovered(provider, error)
            }
            AppEvent::RefreshTimerElapsed => self.refresh_active_provider(),
            AppEvent::ResourceShellStarted { session_id } => {
                self.apply_resource_shell_started(session_id);
                Vec::new()
            }
            AppEvent::ResourceShellStartFailed { session_id, reason } => {
                self.apply_resource_shell_start_failed(session_id, reason);
                Vec::new()
            }
            AppEvent::ResourceShellExited { session_id } => {
                self.apply_resource_shell_exited(session_id)
            }
            AppEvent::RefreshCompleted {
                request_id,
                provider_id,
                result,
            } => self.apply_refresh_completed(request_id, provider_id, result),
            AppEvent::ResourceCommandCompleted {
                request_id,
                provider_id,
                target,
                command,
                result,
            } => {
                self.apply_resource_command_result(request_id, provider_id, target, command, result)
            }
            AppEvent::ResourceDetailsCompleted {
                request_id,
                provider_id,
                target,
                view_id,
                result,
            } => {
                self.apply_details_completed(request_id, provider_id, target, view_id, result);
                Vec::new()
            }
        }
    }

    /// The structural Command Scope the interface is in right now.
    ///
    /// A modal replaces the ordinary workspace scope; otherwise the focused
    /// panel selects the workspace scope.
    pub fn active_scope(&self) -> CommandScope {
        if self.state.confirmation.is_some() {
            CommandScope::Confirmation
        } else if self.state.command_error.is_some() {
            CommandScope::CommandFailure
        } else if self.state.help_overlay.is_some() {
            CommandScope::HelpOverlay
        } else {
            match &self.state.focused_pane {
                FocusedPane::Providers => CommandScope::ProviderSelector,
                FocusedPane::Resources => self
                    .state
                    .active_workspace()
                    .and_then(ProviderWorkspaceState::focused_resource_panel_index)
                    .map_or(CommandScope::ResourceView, CommandScope::ResourcePanel),
                FocusedPane::Details => CommandScope::Details,
            }
        }
    }

    /// Resolves one pressed key against the effective registry in the active
    /// scope. The caller normalizes the terminal event into the registry's
    /// [`Key`] type, so this never sees a crossterm event.
    pub fn resolve_command(&self, key: Key) -> Option<Command> {
        self.commands.resolve(self.active_scope(), key)
    }

    /// Resolves a key the registry reserves no matter the scope or the
    /// configuration — only the emergency Quit.
    pub fn reserved(&self, key: Key) -> Option<Command> {
        self.commands.reserved(key)
    }

    /// Takes the exact selected text a Details Copy Command asked the host to
    /// place on the clipboard. The application never performs clipboard I/O.
    pub fn take_pending_details_copy(&mut self) -> Option<String> {
        self.state.pending_details_copy.take()
    }

    /// Takes a completed Pane Boundary preference for the host to persist.
    pub fn take_pending_pane_boundary_save(&mut self) -> Option<PaneBoundary> {
        self.pending_pane_boundary_save.take()
    }

    /// Takes all host work requested by application-owned Resource Shell
    /// Session transitions.
    pub fn take_resource_shell_effects(&mut self) -> Vec<ResourceShellEffect> {
        std::mem::take(&mut self.pending_resource_shell_effects)
    }

    /// Records a clipboard-adapter failure through the ordinary UI error path.
    pub fn report_details_copy_failure(&mut self, reason: String) {
        self.state.command_error = Some(format!("copy selected Details failed: {reason}"));
    }

    /// Records a state-write failure through the ordinary in-app error path.
    pub fn report_pane_boundary_persistence_failure(&mut self, reason: String) {
        self.state.command_error = Some(format!("saving Pane Boundary failed: {reason}"));
    }

    /// Carries out one resolved user intention and returns any provider work.
    pub fn invoke(&mut self, command: Command) -> Vec<ProviderRequest> {
        let mut requests = self.dispatch(command);
        requests.extend(self.sync_details());
        self.update_command_bar();
        requests
    }

    fn update_command_bar(&mut self) {
        let scope = self.active_scope();
        let resource = self
            .state
            .active_workspace()
            .and_then(|workspace| workspace.selected_resource());
        let commands =
            self.commands
                .in_scope(scope)
                .filter(|entry| match entry.command {
                    Command::Resource(command) => resource
                        .is_some_and(|resource| resource.available_commands.contains(&command)),
                    _ => true,
                })
                .filter(|entry| entry.command != Command::ToggleHelp)
                .collect::<Vec<_>>();
        self.state.command_bar = commands
            .iter()
            .filter(|entry| matches!(entry.command, Command::Resource(_)))
            .chain(
                commands
                    .iter()
                    .filter(|entry| !matches!(entry.command, Command::Resource(_))),
            )
            .take(4)
            .filter_map(|entry| {
                entry.keys.first().map(|key| HelpEntry {
                    key: key.to_string(),
                    description: entry.description.to_owned(),
                })
            })
            .collect();
    }

    /// Makes one Provider Workspace active.
    ///
    /// The Workspace being left keeps its navigation, but its in-flight detail
    /// work is abandoned: a result for a Workspace the user has left is refused
    /// rather than shown.
    fn activate_provider(&mut self, index: usize) -> Vec<ProviderRequest> {
        if self.state.active_provider == Some(index) {
            return Vec::new();
        }
        if let Some(active) = self.state.active_provider {
            let previous = self.state.providers[active].id().clone();
            self.state.providers[active].invalidate_pending_detail();
            self.pending_refreshes
                .retain(|_, provider_id| provider_id != &previous);
        }
        self.state.active_provider = Some(index);
        self.refresh_active_provider()
    }

    /// Resizes the Panes and nothing else. No Provider has work to do: the
    /// Panes are drawn from state the shell already holds.
    fn move_pane_boundary(
        &mut self,
        moved: impl FnOnce(PaneBoundary) -> PaneBoundary,
    ) -> Vec<ProviderRequest> {
        self.state.pane_boundary = moved(self.state.pane_boundary);
        Vec::new()
    }

    fn resize_pane_boundary(
        &mut self,
        moved: impl FnOnce(PaneBoundary) -> PaneBoundary,
    ) -> Vec<ProviderRequest> {
        let before = self.state.pane_boundary.resources_percent();
        let requests = self.move_pane_boundary(moved);
        if before != self.state.pane_boundary.resources_percent() {
            self.pending_pane_boundary_save = Some(self.state.pane_boundary);
        }
        requests
    }

    fn scroll_resource_panel(&mut self, panel: usize, delta: isize) -> Vec<ProviderRequest> {
        if let Some(workspace) = self.state.active_workspace_mut() {
            workspace.move_resource_selection_at(panel, delta);
        }
        Vec::new()
    }

    fn dispatch(&mut self, command: Command) -> Vec<ProviderRequest> {
        if !matches!(
            command,
            Command::CopyDetails
                | Command::BeginDetailsSelection { .. }
                | Command::ExtendDetailsSelection { .. }
                | Command::ExtendDetailsSelectionAtEdge { .. }
        ) && let Some(workspace) = self.state.active_workspace_mut()
        {
            workspace.clear_detail_selection();
        }
        match command {
            Command::Quit => Vec::new(),
            Command::ToggleHelp => {
                self.toggle_help();
                Vec::new()
            }
            Command::Refresh => self.refresh_active_provider(),
            Command::MovePaneBoundaryLeft => self.resize_pane_boundary(PaneBoundary::moved_left),
            Command::MovePaneBoundaryRight => self.resize_pane_boundary(PaneBoundary::moved_right),
            Command::GrabPaneBoundary(column) => {
                self.pane_boundary_before_drag = Some(self.state.pane_boundary);
                self.move_pane_boundary(|boundary| boundary.grabbed_at(column))
            }
            Command::SetPaneBoundary(share) => {
                self.move_pane_boundary(|boundary| boundary.dragged_to(share))
            }
            Command::ReleasePaneBoundary => {
                let requests = self.move_pane_boundary(PaneBoundary::released);
                if self.pane_boundary_before_drag.take().is_some_and(|before| {
                    before.resources_percent() != self.state.pane_boundary.resources_percent()
                }) {
                    self.pending_pane_boundary_save = Some(self.state.pane_boundary);
                }
                requests
            }
            Command::FocusProviders => {
                self.state.focused_pane = FocusedPane::Providers;
                Vec::new()
            }
            Command::FocusResourcePanel(index) => {
                self.focus_resource_panel(index);
                Vec::new()
            }
            Command::FocusDetails => {
                self.state.focused_pane = FocusedPane::Details;
                Vec::new()
            }
            Command::FocusNextPane => {
                self.cycle_focus(1);
                Vec::new()
            }
            Command::FocusPreviousPane => {
                self.cycle_focus(-1);
                Vec::new()
            }
            Command::SelectNext => self.select_by_focus(1),
            Command::SelectPrevious => self.select_by_focus(-1),
            Command::SelectNextFast => {
                self.move_resource_selection(5);
                Vec::new()
            }
            Command::SelectPreviousFast => {
                self.move_resource_selection(-5);
                Vec::new()
            }
            Command::NextWorkspace => self.move_provider_selection(1),
            Command::PreviousWorkspace => self.move_provider_selection(-1),
            Command::NextDetailView => {
                self.move_detail_view(1);
                Vec::new()
            }
            Command::PreviousDetailView => {
                self.move_detail_view(-1);
                Vec::new()
            }
            Command::ScrollDetailsDown => {
                self.scroll_details(DETAIL_SCROLL_LINES);
                Vec::new()
            }
            Command::ScrollDetailsUp => {
                self.scroll_details(-DETAIL_SCROLL_LINES);
                Vec::new()
            }
            // A Details selection arrives through the mouse path. Until one is
            // present, Copy deliberately has nothing to send to the host.
            Command::CopyDetails => {
                self.state.pending_details_copy = self
                    .state
                    .active_workspace()
                    .and_then(ProviderWorkspaceState::selected_detail_text);
                Vec::new()
            }
            Command::BeginDetailsSelection { line, column } => {
                self.state.focused_pane = FocusedPane::Details;
                if let Some(workspace) = self.state.active_workspace_mut() {
                    workspace.begin_detail_selection(line, column);
                }
                Vec::new()
            }
            Command::ExtendDetailsSelection { line, column } => {
                if let Some(workspace) = self.state.active_workspace_mut() {
                    workspace.extend_detail_selection(line, column);
                }
                Vec::new()
            }
            Command::ExtendDetailsSelectionAtEdge {
                above,
                column,
                visible_rows,
            } => {
                if let Some(workspace) = self.state.active_workspace_mut() {
                    workspace.scroll_details(if above { -1 } else { 1 });
                    workspace.extend_detail_selection(
                        if above {
                            0
                        } else {
                            visible_rows.saturating_sub(1)
                        },
                        column,
                    );
                }
                Vec::new()
            }
            Command::OpenShell => {
                self.open_shell();
                Vec::new()
            }
            Command::StartResourceShell => {
                self.start_selected_resource_shell();
                Vec::new()
            }
            Command::Confirm => self.confirm_or_dismiss(),
            Command::Cancel => {
                self.cancel_or_dismiss();
                Vec::new()
            }
            Command::Resource(command) => self.handle_resource_command(command),
            Command::ActivateProviderWorkspace(index) => {
                if index >= self.state.providers.len() {
                    return Vec::new();
                }
                self.state.focused_pane = FocusedPane::Providers;
                self.activate_provider(index)
            }
            Command::SelectResource { panel, resource } => {
                if let Some(workspace) = self.state.active_workspace_mut()
                    && workspace.focus_resource_panel_at(panel)
                {
                    workspace.select_resource_at(resource);
                    self.state.focused_pane = FocusedPane::Resources;
                }
                Vec::new()
            }
            Command::ActivateDetailView(index) => {
                self.state.focused_pane = FocusedPane::Details;
                if let Some(workspace) = self.state.active_workspace_mut() {
                    workspace.select_detail_view_at(index);
                }
                Vec::new()
            }
            // The wheel scrolls what is under the pointer, so it never touches
            // the focused Pane.
            Command::ScrollResourcePanelUp(panel) => self.scroll_resource_panel(panel, -1),
            Command::ScrollResourcePanelDown(panel) => self.scroll_resource_panel(panel, 1),
        }
    }

    /// Moves the selection in whichever panel has focus.
    fn select_by_focus(&mut self, delta: isize) -> Vec<ProviderRequest> {
        match &self.state.focused_pane {
            FocusedPane::Providers => self.move_provider_selection(delta),
            FocusedPane::Resources => {
                self.move_resource_selection(delta);
                Vec::new()
            }
            FocusedPane::Details => Vec::new(),
        }
    }

    /// Accepts the open modal: confirms a Resource Command, or dismisses a
    /// reported failure.
    fn confirm_or_dismiss(&mut self) -> Vec<ProviderRequest> {
        if self.state.confirmation.is_some() {
            self.confirm_resource_command()
        } else {
            self.state.command_error = None;
            Vec::new()
        }
    }

    /// Cancels or returns from the open modal.
    fn cancel_or_dismiss(&mut self) {
        if self.state.confirmation.is_some() {
            self.state.confirmation = None;
        } else if self.state.command_error.is_some() {
            self.state.command_error = None;
        } else if self.state.help_overlay.is_some() {
            self.state.help_overlay = None;
        }
    }

    fn handle_provider_discovered(
        &mut self,
        provider: Provider,
        error: Option<WorkspaceError>,
    ) -> Vec<ProviderRequest> {
        let activates_provider = self.state.active_provider.is_none();
        let should_refresh_active_provider = activates_provider && error.is_none();
        let provider_id = provider.id().clone();
        self.state
            .providers
            .push(ProviderWorkspaceState::new(provider, error));
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
            .find(|provider| provider.id() == &provider_id)
        else {
            return Vec::new();
        };
        match result {
            Ok(snapshot) if snapshot.panels.len() > NUMBERED_RESOURCE_PANEL_CAPACITY => {
                provider.reject_snapshot(WorkspaceError::new(format!(
                    "{} returned {} Resource Panels; Tuivir supports at most \
                     {NUMBERED_RESOURCE_PANEL_CAPACITY} Resource Panels so each retains a \
                     numbered focus Command",
                    provider.name(),
                    snapshot.panels.len(),
                )));
            }
            Ok(snapshot) => {
                provider.reconcile_snapshot(snapshot);
            }
            Err(error) => provider.record_load_error(error),
        }
        Vec::new()
    }

    /// Brings the loaded detail view into line with what is on screen.
    ///
    /// Called after every event and Command, this is the single place a detail
    /// load starts: a target that already matches asks for nothing, and a
    /// target that changed replaces the pending request, which is what makes
    /// the previous one's result unwelcome.
    fn sync_details(&mut self) -> Vec<ProviderRequest> {
        let request_id = ProviderRequestId::new(self.next_request_id);
        let Some(provider) = self.state.active_workspace_mut() else {
            return Vec::new();
        };
        let Some(load) = provider.start_visible_detail_load(request_id) else {
            return Vec::new();
        };
        self.next_request_id += 1;
        let (request_id, provider_id, target, view_id) = load.into_request_parts();
        vec![ProviderRequest::LoadResourceDetails {
            request_id,
            provider_id,
            target,
            view_id,
        }]
    }

    fn apply_details_completed(
        &mut self,
        request_id: ProviderRequestId,
        provider_id: ProviderId,
        target: ResourceTarget,
        view_id: DetailViewId,
        result: Result<ResourceDetails, WorkspaceError>,
    ) {
        let Some(provider) = self
            .state
            .providers
            .iter_mut()
            .find(|provider| provider.id() == &provider_id)
        else {
            return;
        };
        provider.complete_detail_load(DetailCompletion::new(
            request_id,
            provider_id,
            target,
            view_id,
            result,
        ));
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
        target: ResourceTarget,
        command: ResourceCommand,
        result: Result<(), WorkspaceError>,
    ) -> Vec<ProviderRequest> {
        let Some(running) = self
            .state
            .running_commands
            .iter()
            .position(|running| {
                running.request_id == request_id
                    && running.provider_id == provider_id
                    && running.target == target
                    && running.command == command
            })
            .map(|index| self.state.running_commands.remove(index))
        else {
            return Vec::new();
        };
        if let Err(error) = result {
            self.state.command_error = Some(operation_failure(
                &running.provider_name,
                &running.command.to_string(),
                &running.resource_name,
                &running.target,
                &error.message,
            ));
            return Vec::new();
        }
        self.state.command_error = None;
        if !self.is_active_provider(&running.provider_id) {
            return Vec::new();
        }
        self.refresh_active_provider()
    }

    fn apply_resource_shell_started(&mut self, session_id: ResourceShellSessionId) {
        if let Some(session) = self
            .state
            .resource_shell_sessions
            .iter_mut()
            .find(|session| session.id == session_id)
            && session.lifecycle == ResourceShellSessionLifecycle::Starting
        {
            session.lifecycle = ResourceShellSessionLifecycle::Running;
        }
    }

    fn apply_resource_shell_start_failed(
        &mut self,
        session_id: ResourceShellSessionId,
        reason: String,
    ) {
        if let Some(session) = self
            .state
            .resource_shell_sessions
            .iter_mut()
            .find(|session| session.id == session_id)
            && session.lifecycle == ResourceShellSessionLifecycle::Starting
        {
            session.lifecycle = ResourceShellSessionLifecycle::StartFailed(reason);
        }
    }

    fn apply_resource_shell_exited(
        &mut self,
        session_id: ResourceShellSessionId,
    ) -> Vec<ProviderRequest> {
        let Some(session) = self
            .state
            .resource_shell_sessions
            .iter_mut()
            .find(|session| session.id == session_id)
        else {
            return Vec::new();
        };
        let provider_id = session.provider_id.clone();
        session.lifecycle = ResourceShellSessionLifecycle::Exited;
        if self.is_active_provider(&provider_id) {
            self.refresh_active_provider()
        } else {
            Vec::new()
        }
    }

    fn is_active_provider(&self, provider_id: &ProviderId) -> bool {
        self.state
            .active_workspace()
            .is_some_and(|provider| provider.id() == provider_id)
    }

    fn handle_resource_command(&mut self, command: ResourceCommand) -> Vec<ProviderRequest> {
        let Some(provider) = self.state.active_workspace() else {
            return Vec::new();
        };
        let Some(target) = provider.selected_resource_target() else {
            return Vec::new();
        };
        let Some(resource) = provider.selected_resource() else {
            return Vec::new();
        };
        if !resource.available_commands.contains(&command) {
            return Vec::new();
        }
        let provider_id = provider.id().clone();
        let provider_name = provider.name().to_owned();
        let resource_name = resource.name.clone();
        let target = ResourceCommandInvocation {
            provider_id,
            provider_name,
            target,
            resource_name,
            command,
            state: resource.state,
        };
        if command == ResourceCommand::Delete {
            self.state.confirmation = Some(target);
            return Vec::new();
        }
        self.dispatch_resource_command(target)
    }

    fn confirm_resource_command(&mut self) -> Vec<ProviderRequest> {
        let Some(confirmation) = self.state.confirmation.take() else {
            return Vec::new();
        };
        self.dispatch_resource_command(confirmation)
    }

    fn dispatch_resource_command(
        &mut self,
        target: ResourceCommandInvocation,
    ) -> Vec<ProviderRequest> {
        self.state.command_error = None;
        let request_id = ProviderRequestId(self.next_request_id);
        self.next_request_id += 1;
        self.state.running_commands.push(RunningResourceCommand {
            request_id,
            provider_id: target.provider_id.clone(),
            provider_name: target.provider_name,
            target: target.target.clone(),
            resource_name: target.resource_name.clone(),
            command: target.command,
        });

        vec![ProviderRequest::ExecuteResourceCommand {
            request_id,
            provider_id: target.provider_id,
            target: target.target,
            command: target.command,
            state: target.state,
        }]
    }

    /// Asks for the terminal on behalf of the selected Resource's Interactive
    /// Shell.
    ///
    /// A Resource whose Provider offers no shell asks for nothing, so an
    /// unsupported operation stays unsupported rather than being attempted and
    /// refused.
    fn open_shell(&mut self) {
        let started = self
            .state
            .active_workspace_mut()
            .is_some_and(ProviderWorkspaceState::select_resource_shell_tab);
        if started {
            self.state.focused_pane = FocusedPane::Details;
            self.start_selected_resource_shell();
        }
    }

    fn start_selected_resource_shell(&mut self) {
        let Some(provider) = self.state.active_workspace() else {
            return;
        };
        if !provider.selected_resource_shell_tab() {
            return;
        }
        let provider_id = provider.id().clone();
        let provider_name = provider.name().to_owned();
        let Some(target) = provider.selected_resource_target() else {
            return;
        };
        let Some(resource) = provider.selected_resource() else {
            return;
        };
        let Some(process) = resource.shell.clone() else {
            return;
        };
        let resource_name = resource.name.clone();
        if self.state.resource_shell_sessions.iter().any(|session| {
            session.provider_id == provider_id
                && session.target == target
                && matches!(
                    session.lifecycle,
                    ResourceShellSessionLifecycle::Starting
                        | ResourceShellSessionLifecycle::Running
                )
        }) {
            return;
        }
        self.state
            .resource_shell_sessions
            .retain(|session| session.provider_id != provider_id || session.target != target);
        let session = ResourceShellSession {
            id: ResourceShellSessionId(self.next_resource_shell_session_id),
            provider_id,
            provider_name,
            target,
            resource_name,
            lifecycle: ResourceShellSessionLifecycle::Starting,
        };
        self.next_resource_shell_session_id += 1;
        self.state.resource_shell_sessions.push(session.clone());
        self.pending_resource_shell_effects
            .push(ResourceShellEffect::Start { session, process });
    }

    fn toggle_help(&mut self) {
        if self.state.help_overlay.take().is_some() {
            return;
        }
        let scope = match &self.state.focused_pane {
            FocusedPane::Resources => CommandScope::ResourceView,
            FocusedPane::Details => CommandScope::Details,
            FocusedPane::Providers => return,
        };
        let Some(resource) = self.selected_resource() else {
            return;
        };
        let target = resource.name.clone();
        let available_commands = resource.available_commands;
        let offers_a_shell = resource.shell.is_some();
        let panel_count = self
            .state
            .active_workspace()
            .and_then(ProviderWorkspaceState::resource_panel_count)
            .unwrap_or(0);
        self.state.help_overlay = Some(HelpOverlay {
            target,
            entries: self
                .commands
                .in_scope(scope)
                .filter(|registered| {
                    !matches!(
                        registered.command,
                        Command::FocusResourcePanel(index) if index >= panel_count
                    )
                })
                .map(|registered| HelpEntry {
                    key: registered
                        .keys
                        .first()
                        .expect("a Command in scope is bound")
                        .to_string(),
                    description: match registered.command {
                        Command::Resource(command) if !available_commands.contains(&command) => {
                            format!("{} (unavailable)", registered.description)
                        }
                        Command::OpenShell if !offers_a_shell => {
                            format!("{} (unavailable)", registered.description)
                        }
                        _ => registered.description.to_owned(),
                    },
                })
                .collect(),
        });
    }

    fn selected_resource(&self) -> Option<&super::Resource> {
        self.state.active_workspace()?.selected_resource()
    }

    fn refresh_active_provider(&mut self) -> Vec<ProviderRequest> {
        let Some(provider_id) = self
            .state
            .active_workspace()
            .map(|provider| provider.id().clone())
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

    fn focus_resource_panel(&mut self, index: usize) {
        let Some(provider) = self.state.active_workspace_mut() else {
            return;
        };
        if provider.focus_resource_panel_at(index) {
            self.state.focused_pane = FocusedPane::Resources;
        }
    }

    fn cycle_focus(&mut self, delta: isize) {
        let Some(provider) = self.state.active_workspace() else {
            self.state.focused_pane = FocusedPane::Providers;
            return;
        };
        let Some(panel_count) = provider.resource_panel_count() else {
            self.state.focused_pane = FocusedPane::Providers;
            return;
        };
        let current = match self.state.focused_pane {
            FocusedPane::Providers => 0,
            FocusedPane::Resources => provider
                .focused_resource_panel_index()
                .map_or(0, |index| index + 1),
            FocusedPane::Details => panel_count + 1,
        };
        let pane_count = panel_count + 2;
        let next = (current as isize + delta).rem_euclid(pane_count as isize) as usize;
        match next {
            0 => self.state.focused_pane = FocusedPane::Providers,
            next if next <= panel_count => self.focus_resource_panel(next - 1),
            _ => self.state.focused_pane = FocusedPane::Details,
        }
    }

    fn move_resource_selection(&mut self, delta: isize) {
        if let Some(provider) = self.state.active_workspace_mut() {
            provider.move_resource_selection(delta);
        }
    }

    /// Moves the detail view's first visible line, keeping at least the last
    /// line on screen.
    ///
    /// Rendering owns the layout, so the application cannot know how tall the
    /// panel is; clamping to the last line is the strongest promise it can keep
    /// without one, and it is enough that scrolling never runs into blank space.
    fn scroll_details(&mut self, delta: isize) {
        if let Some(provider) = self.state.active_workspace_mut() {
            provider.scroll_details(delta);
        }
    }

    /// Moves through the views the selected Resource's panel offers.
    ///
    /// The views are a ring: three tabs are few enough that walking off one end
    /// is a request for the other, not a mistake to clamp.
    fn move_detail_view(&mut self, delta: isize) {
        if let Some(provider) = self.state.active_workspace_mut() {
            provider.move_detail_view(delta);
        }
    }

    fn move_provider_selection(&mut self, delta: isize) -> Vec<ProviderRequest> {
        let provider_count = self.state.providers.len();
        if provider_count < 2 {
            return Vec::new();
        }
        let Some(active_provider) = self.state.active_provider else {
            return Vec::new();
        };
        let next = (active_provider as isize + delta).rem_euclid(provider_count as isize) as usize;
        self.activate_provider(next)
    }
}
