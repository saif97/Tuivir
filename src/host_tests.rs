//! The terminal handover, at the seam that can show its ordering without a real
//! terminal, container, or instance.
//!
//! Suspending a screen and running a process that owns the user's streams are
//! both things only the host can do, so both are traits here. A fake of each,
//! writing to one shared log, is what makes "suspend, then run, then resume"
//! an assertion rather than a hope.

use std::{
    io,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use super::{DetailDispatchQueue, ShellTerminal, handle_key, handle_mouse, open_pending_shell};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use virtui::{
    application::Command,
    application::{
        App, AppEvent, DetailView, InteractiveShellProcess, ProviderRequest, Resource,
        ResourceCommand, ResourcePanel, WorkspaceSnapshot,
    },
    domain::{Provider, ProviderId, ResourceId, ResourcePanelId, ResourceState, TargetEnvironment},
    infrastructure::process::{InteractiveRunner, ProcessError, ProcessFailure, ProcessSpec},
    infrastructure::provider::ProviderDiscovery,
    presentation::{ScreenLayout, WorkspacePanes, render_to_text, resolve_mouse},
};

/// Everything the host was asked to do, in the order it was asked.
#[derive(Clone, Default)]
struct Handover(Arc<Mutex<Vec<String>>>);

impl Handover {
    fn record(&self, step: impl Into<String>) {
        self.0.lock().expect("handover log lock").push(step.into());
    }

    fn steps(&self) -> Vec<String> {
        self.0.lock().expect("handover log lock").clone()
    }
}

struct FakeTerminal {
    handover: Handover,
    /// Whether the screen can be taken back, so a host that comes back can be
    /// told apart from one whose terminal is beyond saving.
    screen_comes_back: bool,
}

impl FakeTerminal {
    fn new(handover: Handover) -> Self {
        Self {
            handover,
            screen_comes_back: true,
        }
    }

    fn whose_screen_never_comes_back(handover: Handover) -> Self {
        Self {
            handover,
            screen_comes_back: false,
        }
    }
}

impl ShellTerminal for FakeTerminal {
    fn suspend(&mut self) -> io::Result<()> {
        self.handover.record("suspend");
        Ok(())
    }

    fn resume(&mut self) -> io::Result<()> {
        self.handover.record("resume");
        if self.screen_comes_back {
            Ok(())
        } else {
            Err(io::Error::other("the screen is beyond saving"))
        }
    }

    fn discard_keys(&mut self) {
        self.handover.record("discard keys");
    }

    fn resume_reading(&mut self) {
        self.handover.record("resume reading");
    }
}

struct FakeShell {
    handover: Handover,
    outcome: Result<(), ProcessError>,
}

impl InteractiveRunner for FakeShell {
    fn run_interactive(&self, process: &ProcessSpec) -> Result<(), ProcessError> {
        self.handover.record(format!(
            "run {} {}",
            process.program,
            process.args.join(" ")
        ));
        self.outcome.clone()
    }
}

fn docker_discovery() -> ProviderDiscovery {
    ProviderDiscovery::new(
        Provider::new(
            ProviderId::new("docker"),
            "Docker",
            Some(TargetEnvironment::new("desktop-linux")),
            None,
        ),
        None,
    )
}

fn detail_request(resource_id: &str) -> ProviderRequest {
    ProviderRequest::LoadResourceDetails {
        request_id: virtui::application::ProviderRequestId::new(1),
        provider_id: ProviderId::new("docker"),
        target: virtui::domain::ResourceTarget::new(
            ResourcePanelId::new("containers"),
            ResourceId::new(resource_id),
        ),
        view_id: virtui::domain::DetailViewId::new("logs"),
    }
}

#[test]
fn a_navigation_burst_dispatches_only_the_detail_view_where_selection_settles() {
    let quiet_period = Duration::from_millis(75);
    let started = Instant::now();
    let mut dispatch = DetailDispatchQueue::new(quiet_period);
    let refresh = ProviderRequest::RefreshWorkspace {
        request_id: virtui::application::ProviderRequestId::new(2),
        provider_id: ProviderId::new("docker"),
    };

    assert_eq!(
        dispatch.accept(started, detail_request("container-a")),
        None
    );
    assert_eq!(
        dispatch.accept(started, refresh.clone()),
        Some(refresh),
        "refresh work remains immediate"
    );
    assert!(
        dispatch
            .accept(
                started + Duration::from_millis(20),
                detail_request("container-b"),
            )
            .is_none()
    );
    assert!(
        dispatch
            .take_ready(started + Duration::from_millis(94))
            .is_none()
    );
    assert_eq!(
        dispatch.take_ready(started + Duration::from_millis(95)),
        Some(detail_request("container-b")),
    );
}

/// One running container, carrying the Interactive Shell Docker offers inside
/// it.
fn running_container() -> WorkspaceSnapshot {
    WorkspaceSnapshot {
        panels: vec![ResourcePanel {
            id: ResourcePanelId::new("containers"),
            title: "Containers".to_owned(),
            detail_views: vec![DetailView::new("logs", "Logs")],
            resources: vec![Resource {
                id: ResourceId::new("container-a"),
                name: "api".to_owned(),
                status: Some("running".to_owned()),
                state: Some(ResourceState::Running),
                fields: vec![("Image", "nginx:1.27".to_owned())],
                snapshot_details: Vec::new(),
                available_commands: &[ResourceCommand::Stop],
                shell: Some(InteractiveShellProcess::new(
                    "docker",
                    &["exec", "-it", "container-a", "/bin/sh"],
                )),
            }],
        }],
    }
}

/// An application sitting on a running container, with `E` already pressed.
fn app_awaiting_the_terminal() -> App {
    let mut app = App::new();
    let requests = app.update(docker_discovery().into_event());
    let ProviderRequest::RefreshWorkspace {
        request_id,
        provider_id,
    } = requests.into_iter().next().expect("an initial refresh")
    else {
        panic!("discovery refreshes the Active Workspace");
    };
    app.update(AppEvent::RefreshCompleted {
        request_id,
        provider_id,
        result: Ok(running_container()),
    });
    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('E'), KeyModifiers::NONE),
    );
    assert!(
        app.state().pending_shell.is_some(),
        "a running container asks for the terminal"
    );
    app
}

/// The same Workspace with a second container, so a click can land on a
/// Resource that is not already selected.
fn two_containers() -> WorkspaceSnapshot {
    let mut snapshot = running_container();
    let first = snapshot.panels[0].resources[0].clone();
    snapshot.panels[0].resources.push(Resource {
        id: ResourceId::new("container-b"),
        name: "worker".to_owned(),
        ..first
    });
    snapshot
}

/// An application sitting on a loaded Docker Workspace, with nothing pending.
fn app_on_a_loaded_workspace() -> App {
    app_on_workspace(running_container())
}

fn app_on_workspace(snapshot: WorkspaceSnapshot) -> App {
    let mut app = App::new();
    let requests = app.update(docker_discovery().into_event());
    let ProviderRequest::RefreshWorkspace {
        request_id,
        provider_id,
    } = requests.into_iter().next().expect("an initial refresh")
    else {
        panic!("discovery refreshes the Active Workspace");
    };
    app.update(AppEvent::RefreshCompleted {
        request_id,
        provider_id,
        result: Ok(snapshot),
    });
    app
}

fn click(app: &mut App, layout: &ScreenLayout, column: u16, row: u16) -> Vec<ProviderRequest> {
    handle_mouse(
        app,
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: KeyModifiers::NONE,
        },
        Some(layout),
    )
}

/// A clicked Resource must load like a keyboard-selected one. Selecting without
/// loading is the failure this guards: the Detail View would look activated and
/// stay empty until an unrelated refresh happened along.
#[test]
fn clicking_a_resource_row_selects_it_and_asks_for_its_details() {
    let mut app = app_on_workspace(two_containers());
    let layout = ScreenLayout::measure(app.state(), Rect::new(0, 0, 80, 24));
    let panes = layout.panes.as_ref().expect("a Workspace is active");
    let (_, row) = panes.resource_rows[0][1];

    let requests = click(&mut app, &layout, row.x, row.y);

    assert_eq!(
        app.state().focused_pane,
        virtui::application::FocusedPane::Resources,
        "clicking a Resource focuses the Panel holding it"
    );
    assert!(
        requests
            .iter()
            .any(|request| matches!(request, ProviderRequest::LoadResourceDetails { .. })),
        "the click asks for the Detail View, got {requests:?}"
    );
}

#[test]
fn clicking_a_provider_workspace_makes_it_active() {
    let mut app = app_on_a_loaded_workspace();
    let layout = ScreenLayout::measure(app.state(), Rect::new(0, 0, 80, 24));
    let workspace = layout.provider_workspaces[0];

    click(&mut app, &layout, workspace.x, workspace.y);

    assert_eq!(app.state().active_provider, Some(0));
    assert_eq!(
        app.state().focused_pane,
        virtui::application::FocusedPane::Providers
    );
}

/// Pointing is not focusing: the wheel moves what is under the pointer and
/// leaves the keyboard exactly where the user left it.
#[test]
fn the_wheel_scrolls_without_moving_keyboard_focus() {
    let mut app = app_on_a_loaded_workspace();
    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE),
    );
    let focus_before = app.state().focused_pane;
    let layout = ScreenLayout::measure(app.state(), Rect::new(0, 0, 80, 24));
    let panel = layout
        .panes
        .as_ref()
        .expect("a Workspace is active")
        .resource_panels[0];

    handle_mouse(
        &mut app,
        MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: panel.x + 1,
            row: panel.y + 1,
            modifiers: KeyModifiers::NONE,
        },
        Some(&layout),
    );

    assert_eq!(
        app.state().focused_pane,
        focus_before,
        "the wheel never changes which Pane the keyboard drives"
    );
}

/// A modal owns the screen, so a click meant for it must not reach the
/// Provider Workspaces drawn underneath.
#[test]
fn a_click_on_an_open_overlay_changes_nothing_beneath_it() {
    let mut app = app_on_a_loaded_workspace();
    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE),
    );
    let layout = ScreenLayout::measure(app.state(), Rect::new(0, 0, 80, 24));
    assert!(layout.overlay.is_some(), "the help overlay is open");
    let focus_before = app.state().focused_pane;
    let workspace = layout.provider_workspaces[0];

    click(&mut app, &layout, workspace.x, workspace.y);

    assert_eq!(app.state().focused_pane, focus_before);
}

/// The whole point of the handover: Virtui is off the screen before the
/// Provider CLI touches it, and back on afterwards. The refresh can only be
/// returned once all three have happened.
#[test]
fn the_terminal_is_suspended_for_the_provider_cli_and_taken_back_after() {
    let mut app = app_awaiting_the_terminal();
    let handover = Handover::default();
    let mut terminal = FakeTerminal::new(handover.clone());
    let shell = FakeShell {
        handover: handover.clone(),
        outcome: Ok(()),
    };

    let requests =
        open_pending_shell(&mut app, &mut terminal, &shell).expect("the terminal to come back");

    assert_eq!(
        handover.steps(),
        [
            "suspend",
            "run docker exec -it container-a /bin/sh",
            "resume",
            "discard keys",
            "resume reading",
        ]
    );
    assert!(
        matches!(
            requests.as_slice(),
            [ProviderRequest::RefreshWorkspace { provider_id, .. }]
                if provider_id == &ProviderId::new("docker")
        ),
        "returning from a shell refreshes only the Active Workspace, got {requests:?}"
    );
    assert!(app.state().pending_shell.is_none());
    assert!(app.state().command_error.is_none());
}

/// A Provider CLI that was there at discovery and is gone by the time the user
/// asks for a shell must not take the terminal with it. The complaint waits
/// until Virtui is back on screen, which is the only place the user can read
/// it.
#[test]
fn a_shell_that_never_starts_still_gives_the_terminal_back_and_names_what_failed() {
    let mut app = app_awaiting_the_terminal();
    let handover = Handover::default();
    let mut terminal = FakeTerminal::new(handover.clone());
    let shell = FakeShell {
        handover: handover.clone(),
        outcome: Err(ProcessError::ExecutableNotFound),
    };

    open_pending_shell(&mut app, &mut terminal, &shell).expect("the terminal to come back");

    assert_eq!(
        handover.steps().last().map(String::as_str),
        Some("resume reading"),
        "the handover finishes even when the shell never started"
    );
    assert_eq!(
        app.state().command_error.as_deref(),
        Some("Docker shell failed for api (container-a): the CLI is no longer available")
    );
    let screen = render_to_text(app.state(), 100, 24);
    assert!(
        screen.contains("Docker shell failed for api"),
        "rendered screen:\n{screen}"
    );
}

/// A shell exits with the status of the last command typed into it, so a
/// non-zero status is the user's own — a `grep` that matched nothing, then
/// Ctrl-D — and not Virtui failing to give them a shell. Reporting it would put
/// a modal in front of a user who did nothing wrong, every time they left a
/// shell on a failed command.
///
/// Virtui gave them the shell they asked for. What they did inside it is theirs.
#[test]
fn a_shell_that_ran_is_never_a_failure_whatever_status_it_left() {
    let mut app = app_awaiting_the_terminal();
    let handover = Handover::default();
    let mut terminal = FakeTerminal::new(handover.clone());
    let shell = FakeShell {
        handover: handover.clone(),
        outcome: Err(ProcessError::Exited(ProcessFailure {
            exit_code: Some(1),
            stdout: String::new(),
            stderr: String::new(),
        })),
    };

    let requests =
        open_pending_shell(&mut app, &mut terminal, &shell).expect("the terminal to come back");

    assert_eq!(
        app.state().command_error,
        None,
        "a shell that ran leaves nothing to report"
    );
    assert!(
        matches!(
            requests.as_slice(),
            [ProviderRequest::RefreshWorkspace { .. }]
        ),
        "a shell that ran still leaves the workspace worth refreshing, got {requests:?}"
    );
}

/// The other half of the same rule: a shell Virtui could not start is a promise
/// it failed to keep, and says so. The CLI's own words are kept, because the
/// user needs the part Virtui cannot supply.
#[test]
fn a_shell_that_could_not_be_started_names_what_the_cli_said() {
    let mut app = app_awaiting_the_terminal();
    let handover = Handover::default();
    let mut terminal = FakeTerminal::new(handover.clone());
    let shell = FakeShell {
        handover: handover.clone(),
        outcome: Err(ProcessError::SpawnFailed("permission denied".to_owned())),
    };

    open_pending_shell(&mut app, &mut terminal, &shell).expect("the terminal to come back");

    assert_eq!(
        app.state().command_error.as_deref(),
        Some(
            "Docker shell failed for api (container-a): the CLI could not be started: permission denied"
        )
    );
}

/// Keys typed while the shell held the terminal were typed at the shell, and
/// are gone before Virtui reads anything.
///
/// The order is what makes that true rather than merely likely: a reader
/// started first is already pulling those keys out of the queue, so a discard
/// that follows it empties a queue the reader has partly drained and the
/// remainder lands on Virtui as commands the user never aimed at it.
#[test]
fn keys_typed_at_the_shell_are_discarded_before_virtui_reads_again() {
    let mut app = app_awaiting_the_terminal();
    let handover = Handover::default();
    let mut terminal = FakeTerminal::new(handover.clone());
    let shell = FakeShell {
        handover: handover.clone(),
        outcome: Ok(()),
    };

    open_pending_shell(&mut app, &mut terminal, &shell).expect("the terminal to come back");

    let steps = handover.steps();
    let discarded = steps
        .iter()
        .position(|step| step == "discard keys")
        .expect("the keys typed at the shell to be discarded");
    let reading = steps
        .iter()
        .position(|step| step == "resume reading")
        .expect("Virtui to read keys again");
    assert!(
        discarded < reading,
        "discarding must precede reading, got {steps:?}"
    );
}

/// A screen that will not come back is the end of the session, and the shell's
/// outcome is the one fact that would otherwise leave with it.
///
/// So the application is told how the shell ended before the screen's failure
/// is passed on, rather than the two racing to be the news: the host still
/// learns its terminal is beyond saving, and the state it is carrying out is
/// no longer missing what happened inside the shell.
#[test]
fn a_screen_that_never_comes_back_still_carries_the_shells_outcome_out_with_it() {
    let mut app = app_awaiting_the_terminal();
    let handover = Handover::default();
    let mut terminal = FakeTerminal::whose_screen_never_comes_back(handover.clone());
    let shell = FakeShell {
        handover: handover.clone(),
        outcome: Err(ProcessError::ExecutableNotFound),
    };

    let error = open_pending_shell(&mut app, &mut terminal, &shell)
        .expect_err("a screen that will not come back is still reported");

    assert!(
        error.to_string().contains("the screen is beyond saving"),
        "the host learns why its terminal is gone, got {error}"
    );
    assert_eq!(
        app.state().command_error.as_deref(),
        Some("Docker shell failed for api (container-a): the CLI is no longer available"),
        "what the shell did survives the screen that could not report it"
    );
}

/// Nothing to hand over means nothing is disturbed: the host calls this every
/// time round the loop, so an untouched terminal must stay untouched.
#[test]
fn a_loop_with_no_shell_waiting_leaves_the_terminal_alone() {
    let mut app = App::new();
    let handover = Handover::default();
    let mut terminal = FakeTerminal::new(handover.clone());
    let shell = FakeShell {
        handover: handover.clone(),
        outcome: Ok(()),
    };

    let requests = open_pending_shell(&mut app, &mut terminal, &shell).expect("nothing to do");

    assert!(handover.steps().is_empty());
    assert!(requests.is_empty());
}

/// Every region resolves to the Command it means, with no terminal involved.
#[test]
fn mouse_routing_resolves_each_region_without_a_terminal() {
    let layout = ScreenLayout {
        provider_bar: Rect::new(0, 0, 80, 1),
        workspace: Rect::new(0, 1, 80, 22),
        status: Rect::new(0, 23, 80, 0),
        provider_selector: Rect::new(0, 0, 2, 1),
        provider_workspaces: vec![Rect::new(10, 0, 6, 1), Rect::new(2, 0, 8, 1)],
        panes: Some(WorkspacePanes {
            resources: Rect::new(0, 1, 10, 5),
            resource_panels: vec![Rect::new(0, 1, 10, 5)],
            resource_rows: vec![vec![(3, Rect::new(1, 2, 8, 1))]],
            details: Rect::new(10, 1, 20, 5),
            detail_views: vec![Rect::new(12, 2, 8, 1)],
            pane_boundary: Rect::new(9, 1, 2, 5),
        }),
        overlay: None,
    };

    assert_eq!(
        resolve_mouse(&layout, press(3, 0), None),
        Some(Command::ActivateProviderWorkspace(1))
    );
    assert_eq!(
        resolve_mouse(&layout, press(2, 2), None),
        Some(Command::SelectResource {
            panel: 0,
            resource: 3
        })
    );
    assert_eq!(
        resolve_mouse(&layout, press(13, 2), None),
        Some(Command::ActivateDetailView(0))
    );
    assert_eq!(
        resolve_mouse(&layout, press(29, 5), None),
        Some(Command::FocusDetails)
    );
    assert_eq!(resolve_mouse(&layout, press(79, 23), None), None);
}

fn press(column: u16, row: u16) -> virtui::presentation::MouseInput {
    virtui::presentation::MouseInput {
        action: virtui::presentation::MouseAction::Press,
        column,
        row,
    }
}

#[test]
fn mouse_detail_click_focuses_details_without_live_terminal() {
    let mut app = App::new();
    let layout = ScreenLayout {
        provider_bar: Rect::new(0, 0, 80, 0),
        workspace: Rect::new(0, 1, 80, 22),
        status: Rect::new(0, 23, 80, 0),
        provider_selector: Rect::new(0, 0, 0, 0),
        provider_workspaces: Vec::new(),
        panes: Some(WorkspacePanes {
            resources: Rect::new(0, 1, 0, 0),
            resource_panels: Vec::new(),
            resource_rows: Vec::new(),
            details: Rect::new(0, 1, 0, 0),
            detail_views: vec![Rect::new(0, 0, 8, 1)],
            pane_boundary: Rect::new(0, 1, 0, 0),
        }),
        overlay: None,
    };
    handle_mouse(
        &mut app,
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 1,
            row: 0,
            modifiers: KeyModifiers::NONE,
        },
        Some(&layout),
    );
    assert_eq!(
        app.state().focused_pane,
        virtui::application::FocusedPane::Details
    );
}
