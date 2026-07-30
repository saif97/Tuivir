use std::collections::HashMap;

use crate::cli::{ProcessError, ProcessSpec};
use crate::command::{Command, CommandRegistry, CommandScope, NUMBERED_RESOURCE_PANEL_CAPACITY};
use crate::keys::Key;
use crate::provider::{
    DetailView, DetailViewId, ProviderDiscovery, ProviderId, ProviderRequest, ProviderRequestId,
    Resource, ResourceCommand, ResourceDetails, ResourceId, ResourcePanelId, ResourceState,
    ResourceTarget, WorkspaceError, WorkspaceSnapshot,
};

/// Facts the application receives: provider discovery, the refresh clock, and
/// asynchronous completions.
///
/// User intentions are [`Command`]s, resolved from keys, not events. Keeping
/// the two separate means a keypress never looks like a completed refresh.
pub enum AppEvent {
    ProviderDiscovered(ProviderDiscovery),
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
    /// An Interactive Shell has given the terminal back, however it ended.
    ///
    /// The shell it was opened for travels with the event, so what happened can
    /// be reported against its Provider and Resource even though the pending
    /// shell is long gone by then.
    ShellClosed {
        shell: PendingShell,
        result: Result<(), ProcessError>,
    },
    ResourceCommandCompleted {
        request_id: ProviderRequestId,
        provider_id: ProviderId,
        resource_id: ResourceId,
        resource_name: String,
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
        panel_id: ResourcePanelId,
        resource_id: ResourceId,
        view_id: DetailViewId,
        result: Result<ResourceDetails, WorkspaceError>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FocusedPane {
    Providers,
    /// Resource focus requested before the Active Workspace has a snapshot.
    FirstResourcePanel,
    ResourcePanel(ResourcePanelId),
    Details,
}

impl Default for FocusedPane {
    fn default() -> Self {
        Self::FirstResourcePanel
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResourcePanelNavigation {
    pub panel_id: ResourcePanelId,
    pub selected_resource: Option<ResourceId>,
    /// The first Resource row on screen.
    pub scroll: usize,
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
    /// The Resource Panel this Provider Workspace returns to when Resource
    /// focus comes back from another Pane or Provider Workspace.
    pub focused_resource_panel: Option<ResourcePanelId>,
    /// Selection and scroll owned independently by every Resource Panel.
    pub panel_navigation: Vec<ResourcePanelNavigation>,
    /// The selected Resource in `focused_resource_panel`.
    ///
    /// Kept as a projection for request and detail routing; the panel-local
    /// records above are the navigation source of truth.
    pub selected_resource: Option<ResourceTarget>,
    /// Which of the selected Resource's provider-native views is visible.
    ///
    /// It survives moving between Resources of the same panel, so walking a
    /// list while reading one kind of detail does not keep resetting the view.
    /// `None` means no panel has declared any views yet.
    pub selected_detail_view: Option<DetailViewId>,
    /// The detail view this Provider Workspace last loaded or is loading, and
    /// what it was loaded for.
    ///
    /// Carrying the Resource and view alongside the content is what lets the
    /// shell tell "already loaded" from "needs loading" without a second copy
    /// of the selection.
    pub details: Option<ResourceDetailsState>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// One detail view being loaded or displayed, and the target it describes.
pub struct ResourceDetailsState {
    pub panel_id: ResourcePanelId,
    pub resource_id: ResourceId,
    /// The Resource's own name, kept here so an empty or failed view can say
    /// what it was loaded for without going back to the snapshot.
    pub resource_name: String,
    pub view_id: DetailViewId,
    pub title: String,
    pub content: DetailContent,
    /// The first line of output on screen. Every load starts at the top: a
    /// scrolled position belongs to the output it was scrolled through.
    pub scroll: u16,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DetailContent {
    Loading,
    Ready(ResourceDetails),
    Error(WorkspaceError),
}

impl ProviderState {
    /// The detail views offered for the selected Resource, which are the ones
    /// its panel declared.
    pub fn detail_views(&self) -> &[DetailView] {
        let WorkspaceState::Ready(snapshot) = &self.workspace_state else {
            return &[];
        };
        self.selected_resource
            .as_ref()
            .and_then(|selected| snapshot.panel(&selected.panel_id))
            .map_or(&[], |panel| panel.detail_views.as_slice())
    }

    /// What the detail panel should be describing right now: an existing
    /// Resource, the target that addresses it, and one of the views its panel
    /// offers.
    fn detail_target(&self) -> Option<(&ResourceTarget, &Resource, DetailView)> {
        let selected = self.selected_resource.as_ref()?;
        let view_id = self.selected_detail_view.as_ref()?;
        let view = self
            .detail_views()
            .iter()
            .find(|view| &view.id == view_id)?
            .clone();
        let WorkspaceState::Ready(snapshot) = &self.workspace_state else {
            return None;
        };
        let resource = snapshot.resource(&selected.panel_id, &selected.resource_id)?;
        Some((selected, resource, view))
    }
}

/// How far one scroll Command moves through a detail view. Rendering owns the
/// layout, so a fixed step is the honest one: the application has no viewport
/// height to take a page from.
const DETAIL_SCROLL_LINES: isize = 10;

/// One sentence for every operation Virtui asked a Provider for and did not
/// get.
///
/// Lifecycle Commands and Interactive Shells fail in different places and are
/// worded by different code, but they identify their target the same way, so
/// the user reads one sentence rather than two dialects of one.
fn operation_failure(
    provider_name: &str,
    operation: &str,
    resource_name: &str,
    resource_id: &ResourceId,
    reason: &str,
) -> String {
    format!("{provider_name} {operation} failed for {resource_name} ({resource_id}): {reason}")
}

/// Keeps the visible detail view among the ones currently offered, falling back
/// to the first when the selected Resource's panel does not declare it.
fn reconcile_detail_view(provider: &mut ProviderState) {
    let offered = provider
        .detail_views()
        .iter()
        .map(|view| view.id.clone())
        .collect::<Vec<_>>();
    let still_offered = provider
        .selected_detail_view
        .as_ref()
        .is_some_and(|selected| offered.contains(selected));
    if !still_offered {
        provider.selected_detail_view = offered.into_iter().next();
    }
}

fn reconcile_panel_navigation(provider: &mut ProviderState, snapshot: &WorkspaceSnapshot) {
    let previous = std::mem::take(&mut provider.panel_navigation);
    provider.panel_navigation = snapshot
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

    let focused_still_exists = provider
        .focused_resource_panel
        .as_ref()
        .is_some_and(|focused| snapshot.panel(focused).is_some());
    if !focused_still_exists {
        provider.focused_resource_panel = snapshot.panels.first().map(|panel| panel.id.clone());
    }
    provider.selected_resource = provider
        .focused_resource_panel
        .as_ref()
        .and_then(|panel_id| {
            provider
                .panel_navigation
                .iter()
                .find(|navigation| &navigation.panel_id == panel_id)
                .and_then(|navigation| {
                    navigation.selected_resource.as_ref().map(|resource_id| {
                        ResourceTarget::new(panel_id.clone(), resource_id.clone())
                    })
                })
        });
}

#[derive(Default)]
pub struct AppState {
    pub providers: Vec<ProviderState>,
    pub focused_pane: FocusedPane,
    /// Index into `providers` of the currently active provider — the one
    /// whose Provider Workspace is visible and being refreshed.
    ///
    /// `None` represents startup before any installed provider is discovered.
    pub active_provider: Option<usize>,
    pub help_overlay: Option<HelpOverlay>,
    pub confirmation: Option<ResourceCommandInvocation>,
    pub command_error: Option<String>,
    /// The Interactive Shell waiting to be given the terminal.
    ///
    /// The application cannot suspend a screen or run a process, so this is how
    /// it asks: the host takes the shell, hands the terminal over, and reports
    /// back with [`AppEvent::ShellClosed`].
    pub pending_shell: Option<PendingShell>,
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
    pub resource_id: ResourceId,
    pub resource_name: String,
    pub command: ResourceCommand,
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// One Interactive Shell the application wants the terminal for.
///
/// The Provider and Resource travel with the process because the application
/// state cannot be consulted for them: by the time the shell ends, the shell
/// has been taken out of the state and the refreshed snapshot may no longer
/// contain the Resource at all.
pub struct PendingShell {
    pub provider_id: ProviderId,
    pub provider_name: String,
    pub resource_id: ResourceId,
    pub resource_name: String,
    /// The Provider CLI process that takes over the terminal.
    pub process: ProcessSpec,
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
    pub panel_id: ResourcePanelId,
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
    /// The single detail load whose result may still reach the screen.
    ///
    /// Only the visible Resource and view can have one, so replacing or
    /// clearing this is what invalidates a request the user has navigated away
    /// from — its completion no longer matches anything and is dropped.
    pending_details: Option<PendingDetails>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// The target a pending detail load was issued for.
struct PendingDetails {
    request_id: ProviderRequestId,
    provider_id: ProviderId,
    panel_id: ResourcePanelId,
    resource_id: ResourceId,
    view_id: DetailViewId,
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
        let hints = KeyHints::from_registry(&commands);
        let state = AppState {
            hints,
            ..AppState::default()
        };
        Self {
            state,
            commands,
            next_request_id: 1,
            pending_refreshes: HashMap::new(),
            pending_details: None,
        }
    }

    pub fn state(&self) -> &AppState {
        &self.state
    }

    /// Applies one application event and returns any provider work to run.
    ///
    /// This method performs no I/O; the runtime executes returned requests and
    /// feeds their completions back as events.
    pub fn update(&mut self, event: AppEvent) -> Vec<ProviderRequest> {
        let mut requests = self.apply(event);
        requests.extend(self.sync_details());
        requests
    }

    fn apply(&mut self, event: AppEvent) -> Vec<ProviderRequest> {
        match event {
            AppEvent::ProviderDiscovered(discovery) => self.handle_provider_discovered(discovery),
            AppEvent::RefreshTimerElapsed => self.refresh_active_provider(),
            AppEvent::ShellClosed { shell, result } => self.apply_shell_closed(shell, result),
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
            AppEvent::ResourceDetailsCompleted {
                request_id,
                provider_id,
                panel_id,
                resource_id,
                view_id,
                result,
            } => {
                self.apply_details_completed(
                    request_id,
                    provider_id,
                    panel_id,
                    resource_id,
                    view_id,
                    result,
                );
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
                FocusedPane::FirstResourcePanel => CommandScope::ResourceView,
                FocusedPane::ResourcePanel(panel_id) => self
                    .state
                    .active_provider
                    .and_then(|active| self.state.providers.get(active))
                    .and_then(|provider| match &provider.workspace_state {
                        WorkspaceState::Ready(snapshot) => snapshot
                            .panels
                            .iter()
                            .position(|panel| &panel.id == panel_id),
                        WorkspaceState::Loading | WorkspaceState::Error(_) => None,
                    })
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

    /// Carries out one resolved user intention and returns any provider work.
    pub fn invoke(&mut self, command: Command) -> Vec<ProviderRequest> {
        let mut requests = self.dispatch(command);
        requests.extend(self.sync_details());
        requests
    }

    fn dispatch(&mut self, command: Command) -> Vec<ProviderRequest> {
        match command {
            Command::Quit => Vec::new(),
            Command::ToggleHelp => {
                self.toggle_help();
                Vec::new()
            }
            Command::Refresh => self.refresh_active_provider(),
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
            Command::OpenShell => {
                self.open_shell();
                Vec::new()
            }
            Command::Confirm => self.confirm_or_dismiss(),
            Command::Cancel => {
                self.cancel_or_dismiss();
                Vec::new()
            }
            Command::Resource(command) => self.handle_resource_command(command),
        }
    }

    /// Moves the selection in whichever panel has focus.
    fn select_by_focus(&mut self, delta: isize) -> Vec<ProviderRequest> {
        match &self.state.focused_pane {
            FocusedPane::Providers => self.move_provider_selection(delta),
            FocusedPane::FirstResourcePanel | FocusedPane::ResourcePanel(_) => {
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
            focused_resource_panel: None,
            panel_navigation: Vec::new(),
            selected_resource: None,
            selected_detail_view: None,
            details: None,
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
            Ok(snapshot) if snapshot.panels.len() > NUMBERED_RESOURCE_PANEL_CAPACITY => {
                provider.panel_navigation.clear();
                provider.focused_resource_panel = None;
                provider.selected_resource = None;
                provider.workspace_state = WorkspaceState::Error(WorkspaceError::new(format!(
                    "{} returned {} Resource Panels; Virtui supports at most \
                     {NUMBERED_RESOURCE_PANEL_CAPACITY} Resource Panels so each retains a \
                     numbered focus Command",
                    provider.name,
                    snapshot.panels.len(),
                )));
            }
            Ok(snapshot) => {
                reconcile_panel_navigation(provider, &snapshot);
                provider.workspace_state = WorkspaceState::Ready(snapshot);
                reconcile_detail_view(provider);
                if self.state.focused_pane == FocusedPane::FirstResourcePanel {
                    self.state.focused_pane = provider
                        .focused_resource_panel
                        .clone()
                        .map_or(FocusedPane::Providers, FocusedPane::ResourcePanel);
                }
            }
            Err(error) => provider.workspace_state = WorkspaceState::Error(error),
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
        let Some(provider) = self
            .state
            .active_provider
            .and_then(|active| self.state.providers.get_mut(active))
        else {
            self.pending_details = None;
            return Vec::new();
        };
        let provider_id = provider.id.clone();
        let Some((target, resource, view)) = provider.detail_target() else {
            provider.details = None;
            self.pending_details = None;
            return Vec::new();
        };
        let panel_id = target.panel_id.clone();
        let resource_id = resource.id.clone();
        let resource_name = resource.name.clone();
        let describes_target = provider.details.as_ref().is_some_and(|details| {
            details.panel_id == panel_id
                && details.resource_id == resource_id
                && details.view_id == view.id
        });
        // A request still in flight for this very target stays welcome; anything
        // else pending belongs to a target the user left.
        let pending_for_target = self.pending_details.as_ref().is_some_and(|pending| {
            pending.provider_id == provider_id
                && pending.panel_id == panel_id
                && pending.resource_id == resource_id
                && pending.view_id == view.id
        });
        if !pending_for_target {
            self.pending_details = None;
        }
        // Details still loading with nothing pending were abandoned when the
        // user navigated away, and their result will now be refused — so coming
        // back has to ask again rather than wait for it.
        let awaiting_a_refused_result = !pending_for_target
            && provider
                .details
                .as_ref()
                .is_some_and(|details| details.content == DetailContent::Loading);
        if describes_target && !awaiting_a_refused_result {
            return Vec::new();
        }

        provider.details = Some(ResourceDetailsState {
            panel_id: panel_id.clone(),
            resource_id: resource_id.clone(),
            resource_name,
            view_id: view.id.clone(),
            title: view.title,
            content: DetailContent::Loading,
            scroll: 0,
        });
        let request_id = ProviderRequestId(self.next_request_id);
        self.next_request_id += 1;
        self.pending_details = Some(PendingDetails {
            request_id,
            provider_id: provider_id.clone(),
            panel_id: panel_id.clone(),
            resource_id: resource_id.clone(),
            view_id: view.id.clone(),
        });
        vec![ProviderRequest::LoadResourceDetails {
            request_id,
            provider_id,
            panel_id,
            resource_id,
            view_id: view.id,
        }]
    }

    fn apply_details_completed(
        &mut self,
        request_id: ProviderRequestId,
        provider_id: ProviderId,
        panel_id: ResourcePanelId,
        resource_id: ResourceId,
        view_id: DetailViewId,
        result: Result<ResourceDetails, WorkspaceError>,
    ) {
        let expected = PendingDetails {
            request_id,
            provider_id: provider_id.clone(),
            panel_id,
            resource_id,
            view_id,
        };
        if self.pending_details.as_ref() != Some(&expected) {
            return;
        }
        self.pending_details = None;
        let Some(provider) = self
            .state
            .providers
            .iter_mut()
            .find(|provider| provider.id == provider_id)
        else {
            return;
        };
        let Some(details) = provider.details.as_mut() else {
            return;
        };
        details.content = match result {
            Ok(details) => DetailContent::Ready(details),
            Err(error) => DetailContent::Error(error),
        };
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
            self.state.command_error = Some(operation_failure(
                &provider_name,
                &command.to_string(),
                &resource_name,
                &resource_id,
                &error.message,
            ));
            return Vec::new();
        }
        self.state.command_error = None;
        if !self.is_active_provider(&provider_id) {
            return Vec::new();
        }
        self.refresh_active_provider()
    }

    /// Takes the Interactive Shell that is waiting for the terminal, if any.
    ///
    /// Taking it is what commits the application to the handover: a second call
    /// finds nothing, so one keypress can only ever hand the terminal over once.
    pub fn take_pending_shell(&mut self) -> Option<PendingShell> {
        self.state.pending_shell.take()
    }

    /// Brings the Active Workspace back into line with whatever the user did
    /// inside the shell.
    ///
    /// Only the Active Workspace is asked, and it is always the one the shell
    /// was opened in: the handover blocks the event loop from the moment the
    /// shell is taken until this runs, so nothing can have moved in between.
    fn apply_shell_closed(
        &mut self,
        shell: PendingShell,
        result: Result<(), ProcessError>,
    ) -> Vec<ProviderRequest> {
        // Only a shell that never started is reported. One that ran did what
        // was asked of it whatever status it left, and clearing here would
        // dismiss a failure the user has not read yet.
        if let Some(reason) = result.err().as_ref().and_then(ProcessError::start_failure) {
            self.state.command_error = Some(operation_failure(
                &shell.provider_name,
                "shell",
                &shell.resource_name,
                &shell.resource_id,
                &reason,
            ));
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
        let Some(target) = provider.selected_resource.clone() else {
            return Vec::new();
        };
        let WorkspaceState::Ready(snapshot) = &provider.workspace_state else {
            return Vec::new();
        };
        let Some(resource) = snapshot.resource(&target.panel_id, &target.resource_id) else {
            return Vec::new();
        };
        if !resource.available_commands.contains(&command) {
            return Vec::new();
        }
        let provider_id = provider.id.clone();
        let provider_name = provider.name.clone();
        let resource_name = resource.name.clone();
        let Some(state) = resource.state else {
            return Vec::new();
        };
        let panel_id = target.panel_id;
        let resource_id = target.resource_id;
        let target = ResourceCommandInvocation {
            provider_id,
            provider_name,
            panel_id,
            resource_id,
            resource_name,
            command,
            state,
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
            resource_id: target.resource_id.clone(),
            resource_name: target.resource_name.clone(),
            command: target.command,
        });

        vec![ProviderRequest::ExecuteResourceCommand {
            request_id,
            provider_id: target.provider_id,
            panel_id: target.panel_id,
            resource_id: target.resource_id,
            resource_name: target.resource_name,
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
        let Some(provider) = self
            .state
            .active_provider
            .and_then(|active| self.state.providers.get(active))
        else {
            return;
        };
        let provider_id = provider.id.clone();
        let provider_name = provider.name.clone();
        let Some(resource) = self.selected_resource() else {
            return;
        };
        let Some(process) = resource.shell.clone() else {
            return;
        };
        let resource_id = resource.id.clone();
        let resource_name = resource.name.clone();
        self.state.pending_shell = Some(PendingShell {
            provider_id,
            provider_name,
            resource_id,
            resource_name,
            process,
        });
    }

    fn toggle_help(&mut self) {
        if self.state.help_overlay.take().is_some() {
            return;
        }
        let scope = match &self.state.focused_pane {
            FocusedPane::FirstResourcePanel | FocusedPane::ResourcePanel(_) => {
                CommandScope::ResourceView
            }
            FocusedPane::Details => CommandScope::Details,
            FocusedPane::Providers => return,
        };
        let Some(resource) = self.selected_resource() else {
            return;
        };
        let target = resource.name.clone();
        let available_commands = resource.available_commands.clone();
        let offers_a_shell = resource.shell.is_some();
        let panel_count = self
            .state
            .active_provider
            .and_then(|active| self.state.providers.get(active))
            .and_then(|provider| match &provider.workspace_state {
                WorkspaceState::Ready(snapshot) => Some(snapshot.panels.len()),
                WorkspaceState::Loading | WorkspaceState::Error(_) => None,
            })
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

    fn selected_resource(&self) -> Option<&crate::provider::Resource> {
        let provider = self
            .state
            .active_provider
            .and_then(|active| self.state.providers.get(active))?;
        let selected = provider.selected_resource.as_ref()?;
        let WorkspaceState::Ready(snapshot) = &provider.workspace_state else {
            return None;
        };
        snapshot.resource(&selected.panel_id, &selected.resource_id)
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

    fn focus_resource_panel(&mut self, index: usize) {
        let Some(provider) = self
            .state
            .active_provider
            .and_then(|active| self.state.providers.get_mut(active))
        else {
            return;
        };
        let WorkspaceState::Ready(snapshot) = &provider.workspace_state else {
            return;
        };
        let Some(panel_id) = snapshot.panels.get(index).map(|panel| panel.id.clone()) else {
            return;
        };
        provider.focused_resource_panel = Some(panel_id.clone());
        provider.selected_resource = provider
            .panel_navigation
            .iter()
            .find(|navigation| navigation.panel_id == panel_id)
            .and_then(|navigation| {
                navigation
                    .selected_resource
                    .as_ref()
                    .map(|resource_id| ResourceTarget::new(panel_id.clone(), resource_id.clone()))
            });
        reconcile_detail_view(provider);
        self.state.focused_pane = FocusedPane::ResourcePanel(panel_id);
    }

    fn cycle_focus(&mut self, delta: isize) {
        let mut panes = vec![FocusedPane::Providers];
        if let Some(provider) = self
            .state
            .active_provider
            .and_then(|active| self.state.providers.get(active))
            && let WorkspaceState::Ready(snapshot) = &provider.workspace_state
        {
            panes.extend(
                snapshot
                    .panels
                    .iter()
                    .map(|panel| FocusedPane::ResourcePanel(panel.id.clone())),
            );
            panes.push(FocusedPane::Details);
        }
        let current = panes
            .iter()
            .position(|pane| pane == &self.state.focused_pane)
            .unwrap_or(0);
        let next = (current as isize + delta).rem_euclid(panes.len() as isize) as usize;
        match panes[next].clone() {
            FocusedPane::ResourcePanel(panel_id) => {
                if let Some(index) = self
                    .state
                    .active_provider
                    .and_then(|active| self.state.providers.get(active))
                    .and_then(|provider| match &provider.workspace_state {
                        WorkspaceState::Ready(snapshot) => snapshot
                            .panels
                            .iter()
                            .position(|panel| panel.id == panel_id),
                        WorkspaceState::Loading | WorkspaceState::Error(_) => None,
                    })
                {
                    self.focus_resource_panel(index);
                }
            }
            focused => self.state.focused_pane = focused,
        }
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
        let Some(panel_id) = provider.focused_resource_panel.clone() else {
            return;
        };
        let Some(panel) = snapshot.panel(&panel_id) else {
            return;
        };
        let Some(navigation) = provider
            .panel_navigation
            .iter_mut()
            .find(|navigation| navigation.panel_id == panel_id)
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
            provider.selected_resource = navigation
                .selected_resource
                .as_ref()
                .map(|resource_id| ResourceTarget::new(panel_id, resource_id.clone()));
            reconcile_detail_view(provider);
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
        provider.selected_resource = navigation
            .selected_resource
            .as_ref()
            .map(|resource_id| ResourceTarget::new(panel_id, resource_id.clone()));
        reconcile_detail_view(provider);
    }

    /// Moves the detail view's first visible line, keeping at least the last
    /// line on screen.
    ///
    /// Rendering owns the layout, so the application cannot know how tall the
    /// panel is; clamping to the last line is the strongest promise it can keep
    /// without one, and it is enough that scrolling never runs into blank space.
    fn scroll_details(&mut self, delta: isize) {
        let Some(details) = self
            .state
            .active_provider
            .and_then(|active| self.state.providers.get_mut(active))
            .and_then(|provider| provider.details.as_mut())
        else {
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

    /// Moves through the views the selected Resource's panel offers.
    ///
    /// The views are a ring: three tabs are few enough that walking off one end
    /// is a request for the other, not a mistake to clamp.
    fn move_detail_view(&mut self, delta: isize) {
        let Some(provider) = self
            .state
            .active_provider
            .and_then(|active| self.state.providers.get_mut(active))
        else {
            return;
        };
        let offered = provider
            .detail_views()
            .iter()
            .map(|view| view.id.clone())
            .collect::<Vec<_>>();
        if offered.is_empty() {
            return;
        }
        let current = provider
            .selected_detail_view
            .as_ref()
            .and_then(|selected| offered.iter().position(|view| view == selected))
            .unwrap_or(0);
        let next = (current as isize + delta).rem_euclid(offered.len() as isize) as usize;
        provider.selected_detail_view = offered.into_iter().nth(next);
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
        if matches!(
            &self.state.focused_pane,
            FocusedPane::FirstResourcePanel | FocusedPane::ResourcePanel(_)
        ) {
            self.state.focused_pane = self.state.providers[self.state.active_provider.unwrap()]
                .focused_resource_panel
                .clone()
                .map_or(FocusedPane::FirstResourcePanel, FocusedPane::ResourcePanel);
        }
        self.refresh_active_provider()
    }
}
