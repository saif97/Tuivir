//! Host-adapter tests for terminal I/O, event dispatch, and durable UI state.

use std::{
    io,
    path::Path,
    sync::Mutex,
    thread,
    time::{Duration, Instant},
};

use super::{
    Clipboard, DetailDispatchQueue, Osc52Clipboard, ResourceShellRuntime,
    ResourceShellRuntimeEvent, ShellInputRouter, ShellKeyRoute, ShellPointerRoute, handle_key,
    handle_mouse, persist_pane_boundary, release_resource_shell,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use ratatui::style::Color;
use tuivir::{
    application::Command,
    application::{
        App, AppEvent, DetailView, ProviderRequest, Resource, ResourceCommand, ResourcePanel,
        ResourceShellProcess, WorkspaceSnapshot,
    },
    domain::{Provider, ProviderId, ResourceId, ResourcePanelId, ResourceState, TargetEnvironment},
    infrastructure::provider::ProviderDiscovery,
    infrastructure::{
        config::ReadFile,
        pane_boundary_state::{Env as StateEnv, StateStorage},
    },
    presentation::{ScreenLayout, WorkspacePanes, render_to_text, resolve_mouse},
};

#[derive(Default)]
struct RecordingState(Mutex<Vec<String>>);

impl ReadFile for RecordingState {
    fn read(&self, _: &Path) -> io::Result<String> {
        Err(io::Error::new(io::ErrorKind::NotFound, "no saved state"))
    }
}

impl StateStorage for RecordingState {
    fn write_atomically(&self, _: &Path, contents: &str) -> io::Result<()> {
        self.0
            .lock()
            .expect("state writes")
            .push(contents.to_owned());
        Ok(())
    }
}

struct FailingState;

impl ReadFile for FailingState {
    fn read(&self, _: &Path) -> io::Result<String> {
        Err(io::Error::new(io::ErrorKind::NotFound, "no saved state"))
    }
}

impl StateStorage for FailingState {
    fn write_atomically(&self, _: &Path, _: &str) -> io::Result<()> {
        Err(io::Error::other("disk full"))
    }
}

#[test]
fn releasing_a_resized_pane_boundary_persists_one_preference() {
    let mut app = App::new();
    let state = RecordingState::default();
    let env = StateEnv {
        home: Some("/home/me".into()),
        ..StateEnv::default()
    };

    for command in [
        Command::GrabPaneBoundary(0),
        Command::SetPaneBoundary(60),
        Command::ReleasePaneBoundary,
    ] {
        app.invoke(command);
        persist_pane_boundary(&mut app, &env, &state);
    }

    assert_eq!(
        *state.0.lock().expect("state writes"),
        vec![r#"{"resources_percent":60}"#],
        "drag updates reach durable state only once the user lets go"
    );
}

#[test]
fn a_pane_boundary_write_failure_uses_the_normal_in_app_error() {
    let mut app = App::new();
    let env = StateEnv {
        home: Some("/home/me".into()),
        ..StateEnv::default()
    };

    app.invoke(Command::MovePaneBoundaryRight);
    persist_pane_boundary(&mut app, &env, &FailingState);

    assert_eq!(
        app.state().command_error.as_deref(),
        Some("saving Pane Boundary failed: disk full")
    );
}

#[test]
fn a_real_pty_shell_wakes_the_host_and_keeps_its_rendered_output() {
    let (events, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    let mut runtime = ResourceShellRuntime::default();
    let session = tuivir::application::ResourceShellSessionId::new(1);
    runtime
        .start(
            session,
            &ResourceShellProcess::new("/bin/sh", &["-c", "printf 'hello from pty\\n'"]),
            80,
            24,
            events,
        )
        .expect("local shell starts in a PTY");

    assert!((0..3).any(|_| matches!(
        receiver.blocking_recv().expect("PTY output wakes the host"),
        ResourceShellRuntimeEvent::OutputReady { session_id } if session_id == session
    )));
    assert!(
        runtime
            .screen(session)
            .expect("live session screen")
            .lines
            .into_iter()
            .flatten()
            .any(|cell| cell.text == "h")
    );
}

#[test]
fn a_real_pty_shell_keeps_colour_unicode_and_cursor_in_its_screen() {
    let (events, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    let mut runtime = ResourceShellRuntime::default();
    let session = tuivir::application::ResourceShellSessionId::new(8);
    runtime
        .start(
            session,
            &ResourceShellProcess::new("/bin/sh", &["-c", "printf '\\033[31mred\\033[0m 鮫'"]),
            80,
            24,
            events,
        )
        .expect("local shell starts in a PTY");
    let _ = receiver.blocking_recv().expect("PTY output wakes the host");

    let screen = runtime.screen(session).expect("live session screen");
    let cells = screen.lines.into_iter().flatten().collect::<Vec<_>>();
    assert!(
        cells
            .iter()
            .any(|cell| cell.text == "r" && cell.foreground == Some(Color::Red))
    );
    assert!(cells.iter().any(|cell| cell.text == "鮫"));
    assert!(cells.iter().any(|cell| cell.cursor));
}

#[test]
fn a_real_pty_shell_receives_the_exact_enlarged_viewport_size() {
    let (events, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    let mut runtime = ResourceShellRuntime::default();
    let session = tuivir::application::ResourceShellSessionId::new(9);
    runtime
        .start(
            session,
            &ResourceShellProcess::new(
                "/bin/sh",
                &[
                    "-c",
                    "trap 'stty size' WINCH; printf ready; while :; do :; done",
                ],
            ),
            80,
            24,
            events,
        )
        .expect("local shell starts in a PTY");
    let _ = receiver.blocking_recv().expect("shell announces readiness");

    runtime
        .resize(session, 78, 23)
        .expect("the visible enlarged terminal resizes its PTY");
    let deadline = Instant::now() + Duration::from_secs(1);
    while Instant::now() < deadline {
        if matches!(
            receiver.try_recv(),
            Ok(ResourceShellRuntimeEvent::OutputReady { session_id }) if session_id == session
        ) {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    let screen = runtime
        .screen(session)
        .expect("live Resource Shell Session screen")
        .lines
        .into_iter()
        .flatten()
        .map(|cell| cell.text)
        .collect::<String>();
    assert!(screen.contains("23 78"), "terminal screen: {screen:?}");
    runtime.stop(session);
}

#[test]
fn stopping_a_live_session_forgets_and_reaps_its_pty() {
    let (events, _receiver) = tokio::sync::mpsc::unbounded_channel();
    let mut runtime = ResourceShellRuntime::default();
    let session = tuivir::application::ResourceShellSessionId::new(2);
    runtime
        .start(
            session,
            &ResourceShellProcess::new("/bin/sh", &["-c", "sleep 30"]),
            80,
            24,
            events,
        )
        .expect("local shell starts in a PTY");

    runtime.stop(session);
    assert!(runtime.screen(session).is_none());
}

#[test]
fn ctrl_b_q_releases_a_resource_shell_sessions_keyboard_focus() {
    let session = tuivir::application::ResourceShellSessionId::new(7);
    let mut router = ShellInputRouter::default();

    assert!(matches!(
        router.route(session, KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE)),
        ShellKeyRoute::ToPty(bytes) if bytes == b"l"
    ));
    assert!(matches!(
        router.route(
            session,
            KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL)
        ),
        ShellKeyRoute::ToTuivir
    ));
    assert!(matches!(
        router.route(
            session,
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)
        ),
        ShellKeyRoute::Released
    ));
    assert!(matches!(
        router.route(
            session,
            KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE)
        ),
        ShellKeyRoute::ToTuivir
    ));
}

#[test]
fn ctrl_b_z_toggles_the_resource_shell_sessions_size_without_reaching_its_pty() {
    let session = tuivir::application::ResourceShellSessionId::new(7);
    let mut router = ShellInputRouter::default();

    assert!(matches!(
        router.route(
            session,
            KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL)
        ),
        ShellKeyRoute::ToTuivir
    ));
    assert!(matches!(
        router.route(session, KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE)),
        ShellKeyRoute::ToggleSize
    ));
}

#[test]
fn releasing_an_enlarged_resource_shell_restores_the_details_presentation() {
    let mut app = app_on_a_loaded_workspace();
    app.invoke(Command::OpenShell);
    let session = app.state().resource_shell_sessions[0].id;

    assert!(app.state().enlarged_resource_shell_session().is_some());
    assert!(release_resource_shell(&mut app).is_empty());
    assert!(app.state().enlarged_resource_shell_session().is_none());
    assert_eq!(app.state().resource_shell_sessions[0].id, session);
}

#[test]
fn ctrl_b_ctrl_b_sends_one_literal_prefix_to_the_resource_shell_session() {
    let session = tuivir::application::ResourceShellSessionId::new(7);
    let mut router = ShellInputRouter::default();

    assert!(matches!(
        router.route(
            session,
            KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL)
        ),
        ShellKeyRoute::ToTuivir
    ));
    assert!(matches!(
        router.route(
            session,
            KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL)
        ),
        ShellKeyRoute::ToPty(bytes) if bytes == [0x02]
    ));
}

#[test]
fn a_prefix_followed_by_an_unrecognised_key_reaches_the_resource_shell_session() {
    let session = tuivir::application::ResourceShellSessionId::new(7);
    let mut router = ShellInputRouter::default();

    assert!(matches!(
        router.route(
            session,
            KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL)
        ),
        ShellKeyRoute::ToTuivir
    ));
    assert!(matches!(
        router.route(session, KeyEvent::new(KeyCode::F(13), KeyModifiers::NONE)),
        ShellKeyRoute::ToPty(bytes) if bytes == b"\x02\x1b[25~"
    ));
}

#[test]
fn focused_resource_shell_session_receives_escape_ctrl_c_and_function_keys() {
    let session = tuivir::application::ResourceShellSessionId::new(7);
    let mut router = ShellInputRouter::default();

    assert!(matches!(
        router.route(session, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        ShellKeyRoute::ToPty(bytes) if bytes == [0x1b]
    ));
    assert!(matches!(
        router.route(
            session,
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)
        ),
        ShellKeyRoute::ToPty(bytes) if bytes == [0x03]
    ));
    assert!(matches!(
        router.route(session, KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE)),
        ShellKeyRoute::ToPty(bytes) if bytes == b"\x1bOP"
    ));
}

#[test]
fn multiline_unicode_paste_uses_the_resource_shell_sessions_bracketed_paste_mode() {
    let session = tuivir::application::ResourceShellSessionId::new(7);
    let mut router = ShellInputRouter::default();
    let _ = router.route(
        session,
        KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE),
    );

    assert!(matches!(
        router.route_paste(session, "first\n鮫", true),
        ShellKeyRoute::ToPty(bytes) if bytes == b"\x1b[200~first\n\xe9\xae\xab\x1b[201~"
    ));
    assert!(matches!(
        router.route_paste(session, "first\n鮫", false),
        ShellKeyRoute::ToPty(bytes) if bytes == "first\n鮫".as_bytes()
    ));
}

#[test]
fn a_focused_resource_shell_session_receives_mouse_coordinates_relative_to_its_viewport() {
    let session = tuivir::application::ResourceShellSessionId::new(7);
    let mut router = ShellInputRouter::default();
    let _ = router.route(
        session,
        KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE),
    );

    assert!(matches!(
        router.route_mouse(
            session,
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: 12,
                row: 23,
                modifiers: KeyModifiers::NONE,
            },
            Rect::new(10, 20, 40, 10),
            true,
            true,
        ),
        ShellPointerRoute::ToPty(bytes) if bytes == b"\x1b[<0;3;4M"
    ));
}

#[test]
fn dragging_or_wheeling_without_mouse_reporting_selects_or_scrolls_without_shell_input() {
    let session = tuivir::application::ResourceShellSessionId::new(7);
    let mut router = ShellInputRouter::default();
    let area = Rect::new(10, 20, 40, 10);

    assert!(matches!(
        router.route_mouse(
            session,
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: 12,
                row: 23,
                modifiers: KeyModifiers::NONE,
            },
            area,
            false,
            false,
        ),
        ShellPointerRoute::None
    ));
    assert!(matches!(
        router.route_mouse(
            session,
            MouseEvent {
                kind: MouseEventKind::Drag(MouseButton::Left),
                column: 15,
                row: 24,
                modifiers: KeyModifiers::NONE,
            },
            area,
            false,
            false,
        ),
        ShellPointerRoute::Select {
            start: (2, 3),
            end: (5, 4)
        }
    ));
    assert!(matches!(
        router.route_mouse(
            session,
            MouseEvent {
                kind: MouseEventKind::ScrollUp,
                column: 12,
                row: 23,
                modifiers: KeyModifiers::NONE,
            },
            area,
            false,
            false,
        ),
        ShellPointerRoute::Scroll { lines: 3 }
    ));
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
        request_id: tuivir::application::ProviderRequestId::new(1),
        provider_id: ProviderId::new("docker"),
        target: tuivir::domain::ResourceTarget::new(
            ResourcePanelId::new("containers"),
            ResourceId::new(resource_id),
        ),
        view_id: tuivir::domain::DetailViewId::new("logs"),
    }
}

#[test]
fn clipboard_adapter_emits_osc_52_for_exact_text() {
    let mut clipboard = Osc52Clipboard(Vec::new());
    clipboard.copy("a\nb").expect("clipboard write");
    assert_eq!(clipboard.0, b"\x1b]52;c;YQpi\x07");
}

#[test]
fn a_navigation_burst_dispatches_only_the_detail_view_where_selection_settles() {
    let quiet_period = Duration::from_millis(75);
    let started = Instant::now();
    let mut dispatch = DetailDispatchQueue::new(quiet_period);
    let refresh = ProviderRequest::RefreshWorkspace {
        request_id: tuivir::application::ProviderRequestId::new(2),
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

/// One running container, carrying the Resource Shell Session Docker offers inside
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
                secondary_text: None,
                status: Some("running".to_owned()),
                state: Some(ResourceState::Running),
                fields: vec![("Image", "nginx:1.27".to_owned())],
                snapshot_details: Vec::new(),
                available_commands: &[ResourceCommand::Stop],
                shell: Some(ResourceShellProcess::new(
                    "docker",
                    &["exec", "-it", "container-a", "/bin/sh"],
                )),
            }],
        }],
    }
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
    pointer(
        app,
        layout,
        MouseEventKind::Down(MouseButton::Left),
        column,
        row,
    )
}

fn pointer(
    app: &mut App,
    layout: &ScreenLayout,
    kind: MouseEventKind,
    column: u16,
    row: u16,
) -> Vec<ProviderRequest> {
    handle_mouse(
        app,
        MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        },
        Some(layout),
    )
}

/// One whole gesture through the terminal's own event type: take hold of the
/// Pane Boundary, carry it, and let go.
///
/// Grabbing must not focus the Resource Panel the boundary's left column sits
/// in, and a drag after the release must find nothing to move.
#[test]
fn a_whole_drag_gesture_resizes_the_panes_and_then_lets_go() {
    let mut app = app_on_workspace(two_containers());
    let layout = ScreenLayout::measure(app.state(), Rect::new(0, 0, 80, 24));
    let boundary = layout
        .panes
        .as_ref()
        .expect("a Provider Workspace is active")
        .pane_boundary;
    let row = boundary.y + 2;
    let focus_before = app.state().focused_pane;

    pointer(
        &mut app,
        &layout,
        MouseEventKind::Down(MouseButton::Left),
        boundary.x,
        row,
    );

    assert_eq!(
        app.state().focused_pane,
        focus_before,
        "taking hold of the boundary is not focusing the Pane behind it"
    );

    pointer(
        &mut app,
        &layout,
        MouseEventKind::Drag(MouseButton::Left),
        51,
        row,
    );

    assert_eq!(
        app.state().pane_boundary.resources_percent(),
        65,
        "column 51 ends a Resources column 52 of the 80 the Workspace has"
    );

    pointer(
        &mut app,
        &layout,
        MouseEventKind::Up(MouseButton::Left),
        51,
        row,
    );
    pointer(
        &mut app,
        &layout,
        MouseEventKind::Drag(MouseButton::Left),
        30,
        row,
    );

    assert_eq!(
        app.state().pane_boundary.resources_percent(),
        65,
        "a drag after the release carries nothing"
    );
}

/// A clicked Resource starts on its snapshot-backed Overview, so selecting it
/// never asks the Provider to load a hidden Detail View Tab.
#[test]
fn clicking_a_resource_row_selects_it_without_loading_hidden_details() {
    let mut app = app_on_workspace(two_containers());
    let layout = ScreenLayout::measure(app.state(), Rect::new(0, 0, 80, 24));
    let panes = layout.panes.as_ref().expect("a Workspace is active");
    let (_, row) = panes.resource_rows[0][1];

    let requests = click(&mut app, &layout, row.x, row.y);

    assert_eq!(
        app.state().focused_pane,
        tuivir::application::FocusedPane::Resources,
        "clicking a Resource focuses the Panel holding it"
    );
    assert!(requests.is_empty(), "unexpected requests: {requests:?}");
    assert!(render_to_text(app.state(), 80, 24).contains("[ Overview ]"));
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
        tuivir::application::FocusedPane::Providers
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

/// Every region resolves to the Command it means, with no terminal involved.
#[test]
fn mouse_routing_resolves_each_region_without_a_terminal() {
    let layout = ScreenLayout {
        provider_bar: Rect::new(0, 0, 80, 1),
        workspace: Rect::new(0, 1, 80, 22),
        status: Rect::new(0, 23, 80, 0),
        provider_selector: Rect::new(0, 0, 2, 1),
        active_target: None,
        provider_workspaces: vec![Rect::new(10, 0, 6, 1), Rect::new(2, 0, 8, 1)],
        panes: Some(WorkspacePanes {
            resources: Rect::new(0, 1, 10, 5),
            resource_panels: vec![Rect::new(0, 1, 10, 5)],
            resource_rows: vec![vec![(3, Rect::new(1, 2, 8, 1))]],
            details: Rect::new(10, 1, 20, 5),
            detail_content: Rect::new(11, 3, 18, 2),
            detail_views: vec![Rect::new(12, 2, 8, 1)],
            pane_boundary: Rect::new(9, 1, 2, 5),
        }),
        overlay: None,
        resource_shell: None,
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
        resolve_mouse(&layout, press(14, 3), None),
        Some(Command::BeginDetailsSelection { line: 0, column: 3 })
    );
    assert_eq!(
        resolve_mouse(
            &layout,
            tuivir::presentation::MouseInput {
                action: tuivir::presentation::MouseAction::Drag,
                column: 16,
                row: 4,
            },
            None,
        ),
        Some(Command::ExtendDetailsSelection { line: 1, column: 5 })
    );
    assert_eq!(
        resolve_mouse(&layout, press(29, 5), None),
        Some(Command::FocusDetails)
    );
    assert_eq!(resolve_mouse(&layout, press(79, 23), None), None);
}

fn press(column: u16, row: u16) -> tuivir::presentation::MouseInput {
    tuivir::presentation::MouseInput {
        action: tuivir::presentation::MouseAction::Press,
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
        active_target: None,
        provider_workspaces: Vec::new(),
        panes: Some(WorkspacePanes {
            resources: Rect::new(0, 1, 0, 0),
            resource_panels: Vec::new(),
            resource_rows: Vec::new(),
            details: Rect::new(0, 1, 0, 0),
            detail_content: Rect::default(),
            detail_views: vec![Rect::new(0, 0, 8, 1)],
            pane_boundary: Rect::new(0, 1, 0, 0),
        }),
        overlay: None,
        resource_shell: None,
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
        tuivir::application::FocusedPane::Details
    );
}
