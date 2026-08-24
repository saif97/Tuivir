use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    time::Duration,
};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::Color;
use tokio::sync::{Barrier, Notify, mpsc};
use tuivir::{
    application::{
        App, AppEvent, AppState, Command, CommandRegistry, CommandScope, DetailView, FocusedPane,
        LifecycleCommandPolicy, PaneBoundary, ProviderRequest, Resource, ResourceCommand,
        ResourceDetails, ResourcePanel, ResourceShellEffect, ResourceShellPresentation,
        ResourceShellProcess, ResourceShellSessionId, ResourceShellSessionLifecycle,
        WorkspaceError, WorkspaceLoadState, WorkspaceSnapshot, lifecycle_commands,
    },
    domain::{
        DetailViewId, Provider, ProviderId, ResourceId, ResourcePanelId, ResourceState,
        ResourceTarget, TargetEnvironment,
    },
    infrastructure::process::{
        CliRunner, ProcessError, ProcessFailure, ProcessOutput, ProcessSpec,
    },
    infrastructure::provider::{DockerWorkspace, ProviderDiscovery, ProviderWorkspace},
    infrastructure::runtime::{ProviderRuntime, RefreshTimer},
    presentation::{
        ScreenLayout, key_from_event, render_background_colours, render_foreground_colours,
        render_to_text,
    },
};

mod common;
use common::{
    command_completed, command_request, detail_request, details_completed, first_provider_detail,
    ready_workspace, refresh_completed, refresh_request,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ShellControl {
    Continue,
    Quit,
}

/// Drives the application through the same logical-key seam as the host.
fn handle_key(app: &mut App, event: KeyEvent) -> (ShellControl, Vec<ProviderRequest>) {
    let Some(key) = key_from_event(event) else {
        return (ShellControl::Continue, Vec::new());
    };
    if app.reserved(key) == Some(Command::Quit) {
        let requests = app.invoke(Command::Quit);
        return (
            app.quit_is_ready()
                .then_some(ShellControl::Quit)
                .unwrap_or(ShellControl::Continue),
            requests,
        );
    }
    match app.resolve_command(key) {
        Some(Command::Quit) => {
            let requests = app.invoke(Command::Quit);
            (
                app.quit_is_ready()
                    .then_some(ShellControl::Quit)
                    .unwrap_or(ShellControl::Continue),
                requests,
            )
        }
        Some(command) => (ShellControl::Continue, app.invoke(command)),
        None => (ShellControl::Continue, Vec::new()),
    }
}

/// Reports the single foreground colour `text` is rendered in, panicking when
/// it is absent from the screen or split across colours.
fn foreground_of(state: &AppState, width: u16, height: u16, text: &str) -> Color {
    colour_of(state, width, height, text, render_foreground_colours)
}

fn background_of(state: &AppState, width: u16, height: u16, text: &str) -> Color {
    colour_of(state, width, height, text, render_background_colours)
}

/// Reports the single colour `text` is rendered in, panicking when it is
/// absent from the screen or split across colours.
fn colour_of(
    state: &AppState,
    width: u16,
    height: u16,
    text: &str,
    render_colours: fn(&AppState, u16, u16) -> Vec<Vec<Color>>,
) -> Color {
    let screen = render_to_text(state, width, height);
    let colours = render_colours(state, width, height);
    // Cell symbols such as a panel border are multi-byte, so a byte offset into
    // the rendered line is not a screen column until it is counted in chars.
    let (row, column) = screen
        .lines()
        .enumerate()
        .find_map(|(row, line)| {
            line.find(text)
                .map(|offset| (row, line[..offset].chars().count()))
        })
        .unwrap_or_else(|| panic!("{text:?} is on screen"));
    let cells = &colours[row][column..column + text.chars().count()];
    assert!(
        cells.iter().all(|colour| colour == &cells[0]),
        "{text:?} is rendered in one colour, not {cells:?}"
    );
    cells[0]
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

fn fixture_discovery() -> ProviderDiscovery {
    ProviderDiscovery::new(
        Provider::new(
            ProviderId::new("fixture"),
            "Fixture",
            Some(TargetEnvironment::new("local")),
            None,
        ),
        None,
    )
}

fn incus_discovery() -> ProviderDiscovery {
    ProviderDiscovery::new(
        Provider::new(
            ProviderId::new("incus"),
            "Incus",
            Some(TargetEnvironment::new("local / default")),
            None,
        ),
        None,
    )
}

#[test]
fn first_available_provider_becomes_the_active_workspace() {
    let mut app = App::new();
    assert_eq!(app.state().active_provider, None);

    app.update(docker_discovery().into_event());

    assert_eq!(app.state().active_provider, Some(0));
    assert_eq!(
        app.state()
            .active_workspace()
            .map(|workspace| workspace.id()),
        Some(&ProviderId::new("docker")),
    );
}

#[test]
fn lifecycle_commands_share_one_policy_with_only_real_provider_differences() {
    assert_eq!(
        lifecycle_commands(
            ResourceState::Running,
            LifecycleCommandPolicy::RestartAndResume,
        ),
        [
            ResourceCommand::Stop,
            ResourceCommand::Restart,
            ResourceCommand::Delete,
        ]
    );
    assert_eq!(
        lifecycle_commands(ResourceState::Running, LifecycleCommandPolicy::StartStop),
        [ResourceCommand::Stop, ResourceCommand::Delete]
    );
    assert_eq!(
        lifecycle_commands(ResourceState::Paused, LifecycleCommandPolicy::StartStop),
        [ResourceCommand::Delete],
        "a Provider without pause/resume support must not inherit Resume",
    );
    assert_eq!(
        lifecycle_commands(
            ResourceState::Unknown,
            LifecycleCommandPolicy::RestartAndResume,
        ),
        [ResourceCommand::Delete]
    );
}

struct DelayedCli {
    started: Arc<Notify>,
    release: Arc<Notify>,
}

struct MissingCli;

struct ExpectedCli {
    commands: Arc<Mutex<Vec<ProcessSpec>>>,
}

struct ConcurrentDiscoveryCli {
    first_probes: Arc<Barrier>,
}

impl CliRunner for ConcurrentDiscoveryCli {
    fn run<'a>(
        &'a self,
        command: ProcessSpec,
    ) -> Pin<Box<dyn Future<Output = Result<ProcessOutput, ProcessError>> + Send + 'a>> {
        Box::pin(async move {
            if command == ProcessSpec::new("docker", &["context", "show"])
                || command == ProcessSpec::new("incus", &["remote", "get-default"])
                || command == ProcessSpec::new("sbx", &["version"])
            {
                self.first_probes.wait().await;
            }
            let stdout = if command == ProcessSpec::new("docker", &["context", "show"]) {
                "desktop-linux\n"
            } else if command == ProcessSpec::new("incus", &["remote", "get-default"])
                || command == ProcessSpec::new("incus", &["project", "get-current"])
            {
                "local\n"
            } else if command == ProcessSpec::new("sbx", &["version"]) {
                "sbx version: v0.37.0 build\n"
            } else if command == ProcessSpec::new("sbx", &["ls", "--json"]) {
                "[]"
            } else {
                panic!("unexpected discovery command: {command:?}");
            };
            Ok(ProcessOutput {
                stdout: stdout.to_owned(),
                stderr: String::new(),
            })
        })
    }
}

impl CliRunner for ExpectedCli {
    fn run<'a>(
        &'a self,
        command: ProcessSpec,
    ) -> Pin<Box<dyn Future<Output = Result<ProcessOutput, ProcessError>> + Send + 'a>> {
        Box::pin(async move {
            self.commands
                .lock()
                .expect("recorded command lock")
                .push(command);
            Ok(ProcessOutput {
                stdout: String::new(),
                stderr: String::new(),
            })
        })
    }
}

struct RejectingCli {
    stderr: String,
}

impl CliRunner for RejectingCli {
    fn run<'a>(
        &'a self,
        _command: ProcessSpec,
    ) -> Pin<Box<dyn Future<Output = Result<ProcessOutput, ProcessError>> + Send + 'a>> {
        Box::pin(async move {
            Err(ProcessError::Exited(ProcessFailure {
                exit_code: Some(1),
                stdout: String::new(),
                stderr: self.stderr.clone(),
            }))
        })
    }
}

impl CliRunner for MissingCli {
    fn run<'a>(
        &'a self,
        _command: ProcessSpec,
    ) -> Pin<Box<dyn Future<Output = Result<ProcessOutput, ProcessError>> + Send + 'a>> {
        Box::pin(async { Err(ProcessError::ExecutableNotFound) })
    }
}

impl CliRunner for DelayedCli {
    fn run<'a>(
        &'a self,
        command: ProcessSpec,
    ) -> Pin<Box<dyn Future<Output = Result<ProcessOutput, ProcessError>> + Send + 'a>> {
        Box::pin(async move {
            if [
                ProcessSpec::new(
                    "docker",
                    &["image", "ls", "--no-trunc", "--format", "{{json .}}"],
                ),
                ProcessSpec::new("docker", &["volume", "ls", "--format", "{{json .}}"]),
            ]
            .contains(&command)
            {
                return Ok(ProcessOutput {
                    stdout: String::new(),
                    stderr: String::new(),
                });
            }
            assert_eq!(
                command,
                ProcessSpec::new(
                    "docker",
                    &[
                        "container",
                        "ls",
                        "--all",
                        "--no-trunc",
                        "--format",
                        "{{json .}}",
                    ],
                )
            );
            self.started.notify_one();
            self.release.notified().await;
            Ok(ProcessOutput {
                stdout: concat!(
                    "{\"ID\":\"container-a\",\"Image\":\"nginx:1.27\",\"Names\":\"api\",\"State\":\"running\",\"Status\":\"Up\"}\n",
                    "{\"ID\":\"container-b\",\"Image\":\"alpine:3.21\",\"Names\":\"worker\",\"State\":\"running\",\"Status\":\"Up\"}\n"
                )
                .to_owned(),
                stderr: String::new(),
            })
        })
    }
}

#[tokio::test]
async fn runtime_executes_resource_command_and_publishes_its_completion() {
    use std::time::Duration;

    let mut app = App::new();
    ready_workspace(
        &mut app,
        docker_discovery(),
        snapshot(&[("container-a", "api", "nginx:1.27")]),
    );
    let command = app.invoke(Command::Resource(ResourceCommand::Restart));
    let commands = Arc::new(Mutex::new(Vec::new()));
    let runtime = ProviderRuntime::new(
        vec![Arc::new(DockerWorkspace) as Arc<dyn ProviderWorkspace>],
        Arc::new(ExpectedCli {
            commands: Arc::clone(&commands),
        }),
    );
    let (events, mut completions) = mpsc::unbounded_channel();

    runtime.dispatch(command.into_iter().next().expect("restart request"), events);
    let completion = tokio::time::timeout(Duration::from_millis(100), completions.recv())
        .await
        .expect("Command completion should not time out")
        .expect("Command completion event");

    assert!(matches!(
        completion,
        AppEvent::ResourceCommandCompleted {
            provider_id,
            target,
            command: ResourceCommand::Restart,
            result: Ok(()),
            ..
        } if provider_id == ProviderId::new("docker")
            && target == ResourceTarget::new(
                ResourcePanelId::new("containers"),
                ResourceId::new("container-a"),
            )
    ));
    assert_eq!(
        *commands.lock().expect("recorded command lock"),
        [ProcessSpec::new(
            "docker",
            &["container", "restart", "container-a"]
        )]
    );
}

#[tokio::test]
async fn a_resource_command_that_exits_non_zero_reaches_the_screen_as_a_failure() {
    use std::time::Duration;

    let mut app = App::new();
    ready_workspace(
        &mut app,
        docker_discovery(),
        snapshot(&[("container-a", "api", "nginx:1.27")]),
    );
    let request = app
        .invoke(Command::Resource(ResourceCommand::Restart))
        .into_iter()
        .next()
        .expect("restart request");
    let runtime = ProviderRuntime::new(
        vec![Arc::new(DockerWorkspace) as Arc<dyn ProviderWorkspace>],
        Arc::new(RejectingCli {
            stderr: "no such container".to_owned(),
        }),
    );
    let (events, mut completions) = mpsc::unbounded_channel();

    runtime.dispatch(request, events);
    let completion = tokio::time::timeout(Duration::from_millis(100), completions.recv())
        .await
        .expect("Command completion should not time out")
        .expect("Command completion event");
    let follow_up = app.update(completion);

    assert!(follow_up.is_empty(), "failed Commands do not refresh");
    let screen = render_to_text(app.state(), 160, 24);
    assert!(
        screen.contains("Docker restart failed for api (container-a): no such container"),
        "rendered screen:\n{screen}"
    );
}

#[tokio::test(start_paused = true)]
async fn active_workspace_refresh_is_due_every_two_seconds() {
    use std::time::Duration;

    let mut app = App::new();
    ready_workspace(
        &mut app,
        docker_discovery(),
        snapshot(&[("container-a", "api", "nginx:1.27")]),
    );
    let mut timer = RefreshTimer::new();
    let tick = timer.tick();
    tokio::pin!(tick);

    tokio::time::advance(Duration::from_millis(1_999)).await;
    tokio::select! {
        biased;
        _ = &mut tick => panic!("refresh fired before two seconds"),
        _ = tokio::task::yield_now() => {}
    }
    tokio::time::advance(Duration::from_millis(1)).await;
    tick.await;

    assert_eq!(app.update(AppEvent::RefreshTimerElapsed).len(), 1);
}

#[tokio::test]
async fn slow_provider_refresh_does_not_block_navigation() {
    let mut app = App::new();
    ready_workspace(
        &mut app,
        docker_discovery(),
        snapshot(&[
            ("container-a", "api", "nginx:1.27"),
            ("container-b", "worker", "alpine:3.21"),
        ]),
    );
    let refresh = refresh_request(app.invoke(Command::Refresh));
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let runtime = ProviderRuntime::new(
        vec![Arc::new(DockerWorkspace) as Arc<dyn ProviderWorkspace>],
        Arc::new(DelayedCli {
            started: Arc::clone(&started),
            release: Arc::clone(&release),
        }),
    );
    let (events, mut completions) = mpsc::unbounded_channel();

    runtime.dispatch(refresh, events);
    started.notified().await;
    app.invoke(Command::SelectNext);
    let responsive_screen = render_to_text(app.state(), 100, 24);
    assert!(responsive_screen.contains("Image: alpine:3.21"));

    release.notify_one();
    let completion = completions.recv().await.expect("refresh completion event");
    app.update(completion);
    let refreshed_screen = render_to_text(app.state(), 100, 24);
    assert!(refreshed_screen.contains("Image: alpine:3.21"));
}

#[tokio::test]
async fn provider_is_omitted_when_docker_cli_is_absent() {
    let runtime = ProviderRuntime::new(
        vec![Arc::new(DockerWorkspace) as Arc<dyn ProviderWorkspace>],
        Arc::new(MissingCli),
    );
    let mut app = App::new();

    for discovered in runtime.discover().await {
        app.update(discovered.into_event());
    }

    let screen = render_to_text(app.state(), 100, 24);
    assert!(screen.contains("No providers discovered"));
    assert!(!screen.contains("Docker"));
}

#[tokio::test]
async fn provider_discoveries_run_together_and_keep_registration_order() {
    let first_probes = Arc::new(Barrier::new(4));
    let runtime = ProviderRuntime::with_builtin_providers(Arc::new(ConcurrentDiscoveryCli {
        first_probes: Arc::clone(&first_probes),
    }));

    let discovery = tokio::spawn(async move { runtime.discover().await });
    tokio::time::timeout(Duration::from_secs(1), first_probes.wait())
        .await
        .expect("all Provider discoveries start before any one finishes");
    let discovered = discovery.await.expect("discovery task");

    assert_eq!(
        discovered
            .iter()
            .map(|discovery| discovery.provider().id().0.as_str())
            .collect::<Vec<_>>(),
        ["docker", "incus", "docker-sandbox"],
    );
}

#[test]
fn keyboard_commands_drive_navigation_manual_refresh_and_quit() {
    let mut app = App::new();
    ready_workspace(
        &mut app,
        docker_discovery(),
        snapshot(&[
            ("container-a", "api", "nginx:1.27"),
            ("container-b", "worker", "alpine:3.21"),
        ]),
    );

    let (control, requests) = handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
    );
    assert_eq!(control, ShellControl::Continue);
    // The new Resource starts on its snapshot-backed Overview.
    assert!(requests.is_empty(), "unexpected requests: {requests:?}");
    assert!(render_to_text(app.state(), 100, 24).contains("Image: alpine:3.21"));

    let (_, requests) = handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL),
    );
    assert_eq!(requests.len(), 1);

    let (control, _) = handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
    );
    assert_eq!(control, ShellControl::Quit);
}

#[test]
fn restart_key_dispatches_the_selected_resource_command() {
    let mut app = App::new();
    ready_workspace(
        &mut app,
        docker_discovery(),
        snapshot(&[("container-a", "api", "nginx:1.27")]),
    );

    let (_, requests) = handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE),
    );

    assert!(matches!(
        requests.as_slice(),
        [ProviderRequest::ExecuteResourceCommand {
            provider_id,
            target,
            command: ResourceCommand::Restart,
            ..
        }] if provider_id == &ProviderId::new("docker")
            && target == &ResourceTarget::new(
                ResourcePanelId::new("containers"),
                ResourceId::new("container-a"),
            )
    ));
}

#[test]
fn start_key_dispatches_for_a_stopped_instance() {
    let mut app = App::new();
    ready_workspace(
        &mut app,
        incus_discovery(),
        incus_snapshot(&[("instance-a", "gateway", "Stopped")]),
    );

    let (_, requests) = handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('S'), KeyModifiers::SHIFT),
    );

    assert!(matches!(
        requests.as_slice(),
        [ProviderRequest::ExecuteResourceCommand {
            provider_id,
            target,
            command: ResourceCommand::Start,
            ..
        }] if provider_id == &ProviderId::new("incus")
            && target == &ResourceTarget::new(
                ResourcePanelId::new("instances"),
                ResourceId::new("instance-a"),
            )
    ));
}

#[test]
fn stop_key_dispatches_for_a_running_container() {
    let mut app = App::new();
    ready_workspace(
        &mut app,
        docker_discovery(),
        snapshot(&[("container-a", "api", "nginx:1.27")]),
    );
    let (_, requests) = handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE),
    );

    assert!(matches!(
        requests.as_slice(),
        [ProviderRequest::ExecuteResourceCommand {
            provider_id,
            target,
            command: ResourceCommand::Stop,
            ..
        }] if provider_id == &ProviderId::new("docker")
            && target == &ResourceTarget::new(
                ResourcePanelId::new("containers"),
                ResourceId::new("container-a"),
            )
    ));
}

#[test]
fn resume_key_dispatches_for_a_paused_container_and_carries_its_state() {
    let mut app = App::new();
    ready_workspace(
        &mut app,
        docker_discovery(),
        paused_snapshot(&[("container-a", "api", "nginx:1.27")]),
    );

    let (_, requests) = handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE),
    );

    assert!(matches!(
        requests.as_slice(),
        [ProviderRequest::ExecuteResourceCommand {
            provider_id,
            target,
            command: ResourceCommand::Resume,
            state: Some(ResourceState::Paused),
            ..
        }] if provider_id == &ProviderId::new("docker")
            && target == &ResourceTarget::new(
                ResourcePanelId::new("containers"),
                ResourceId::new("container-a"),
            )
    ));
}

/// Resuming anything that is not suspended would fail in the Provider CLI, so
/// the shell never dispatches it from another Resource State.
#[test]
fn resume_key_dispatches_nothing_for_a_resource_that_is_not_paused() {
    for snapshot in [
        snapshot(&[("container-a", "api", "nginx:1.27")]),
        stopped_snapshot(&[("container-a", "api", "nginx:1.27")]),
        container_snapshot(
            &[("container-a", "api", "nginx:1.27")],
            ResourceState::Broken,
        ),
    ] {
        let mut app = App::new();
        let initial = refresh_request(app.update(docker_discovery().into_event()));
        app.update(refresh_completed(initial, Ok(snapshot)));

        let (_, requests) = handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE),
        );

        assert!(requests.is_empty());
    }
}

#[test]
fn successful_resource_command_refreshes_the_active_workspace_and_preserves_selection() {
    let mut app = App::new();
    ready_workspace(
        &mut app,
        docker_discovery(),
        snapshot(&[
            ("container-a", "api", "nginx:1.27"),
            ("container-b", "worker", "alpine:3.21"),
        ]),
    );
    app.invoke(Command::SelectNext);
    let request = app.invoke(Command::Resource(ResourceCommand::Restart));
    let ProviderRequest::ExecuteResourceCommand {
        request_id,
        provider_id,
        target,
        command,
        ..
    } = request.into_iter().next().expect("restart request")
    else {
        panic!("expected Resource Command request");
    };

    let refresh = refresh_request(app.update(AppEvent::ResourceCommandCompleted {
        request_id,
        provider_id,
        target,
        command,
        result: Ok(()),
    }));
    app.update(refresh_completed(
        refresh,
        Ok(snapshot(&[
            ("container-c", "scheduler", "debian:bookworm"),
            ("container-b", "worker", "alpine:3.21"),
        ])),
    ));

    assert!(render_to_text(app.state(), 100, 24).contains("Image: alpine:3.21"));
}

/// A user who went into a shell to change something wants to see the change,
/// and wants to come back to the Resource they left. Only the Active Workspace
/// is asked, so a shell never wakes a Provider the user is not looking at.
#[test]
fn returning_from_a_shell_refreshes_the_active_workspace_and_preserves_selection() {
    let containers = [
        ("container-a", "api", "nginx:1.27"),
        ("container-b", "worker", "redis:7"),
    ];
    let mut app = App::new();
    let initial = refresh_request(app.update(docker_discovery().into_event()));
    app.update(refresh_completed(initial, Ok(snapshot(&containers))));
    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
    );
    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('E'), KeyModifiers::NONE),
    );
    let shell = app
        .state()
        .resource_shell_sessions
        .first()
        .expect("a Resource Shell Session to start")
        .clone();
    assert_eq!(
        shell.target,
        ResourceTarget::new(
            ResourcePanelId::new("containers"),
            ResourceId::new("container-b"),
        )
    );

    app.update(AppEvent::ResourceShellStarted {
        session_id: shell.id,
    });
    let requests = app.update(AppEvent::ResourceShellExited {
        session_id: shell.id,
    });

    let refresh = requests
        .iter()
        .filter(|request| matches!(request, ProviderRequest::RefreshWorkspace { .. }))
        .collect::<Vec<_>>();
    assert!(
        matches!(
            refresh.as_slice(),
            [ProviderRequest::RefreshWorkspace { provider_id, .. }]
                if provider_id == &ProviderId::new("docker")
        ),
        "exactly one refresh, for the Active Workspace, got {requests:?}"
    );
    app.update(refresh_completed(
        refresh_request(requests),
        Ok(snapshot(&containers)),
    ));

    assert_eq!(
        app.state().providers[0].selected_resource_target(),
        Some(ResourceTarget::new(
            ResourcePanelId::new("containers"),
            ResourceId::new("container-b")
        ))
    );
}

#[test]
fn failed_resource_command_identifies_provider_resource_and_attempted_command() {
    let mut app = App::new();
    ready_workspace(
        &mut app,
        docker_discovery(),
        snapshot(&[("container-a", "api", "nginx:1.27")]),
    );
    let request = app.invoke(Command::Resource(ResourceCommand::Restart));
    let ProviderRequest::ExecuteResourceCommand {
        request_id,
        provider_id,
        target,
        command,
        ..
    } = request.into_iter().next().expect("restart request")
    else {
        panic!("expected Resource Command request");
    };

    let follow_up = app.update(AppEvent::ResourceCommandCompleted {
        request_id,
        provider_id,
        target,
        command,
        result: Err(WorkspaceError::new("permission denied")),
    });

    assert!(follow_up.is_empty(), "failed Commands do not refresh");
    let screen = render_to_text(app.state(), 160, 24);
    assert!(
        screen.contains("Docker restart failed for api (container-a): permission denied"),
        "rendered screen:\n{screen}"
    );
}

#[test]
fn selecting_another_resource_keeps_an_in_flight_resource_command() {
    let mut app = App::new();
    ready_workspace(
        &mut app,
        docker_discovery(),
        snapshot(&[
            ("container-a", "api", "nginx:1.27"),
            ("container-b", "worker", "alpine:3.21"),
        ]),
    );
    let restart = command_request(app.invoke(Command::Resource(ResourceCommand::Restart)));

    app.invoke(Command::SelectNext);
    let follow_up = app.update(command_completed(restart, Ok(())));

    assert_eq!(
        follow_up.len(),
        1,
        "the completed Resource Command still refreshes its Provider Workspace"
    );
}

#[test]
fn a_resource_command_completion_for_another_target_stays_late() {
    let mut app = App::new();
    let initial = refresh_request(app.update(docker_discovery().into_event()));
    app.update(refresh_completed(
        initial,
        Ok(snapshot(&[("container-a", "api", "nginx:1.27")])),
    ));
    let restart = command_request(app.invoke(Command::Resource(ResourceCommand::Restart)));
    let ProviderRequest::ExecuteResourceCommand {
        request_id,
        provider_id,
        command,
        ..
    } = &restart
    else {
        panic!("expected Resource Command request");
    };

    let requests = app.update(AppEvent::ResourceCommandCompleted {
        request_id: *request_id,
        provider_id: provider_id.clone(),
        target: ResourceTarget::new(
            ResourcePanelId::new("images"),
            ResourceId::new("container-a"),
        ),
        command: *command,
        result: Ok(()),
    });

    assert!(requests.is_empty());
    assert_eq!(app.state().running_commands.len(), 1);
    assert_eq!(
        app.update(command_completed(restart, Ok(()))).len(),
        1,
        "the matching completion still refreshes its Provider Workspace"
    );
}

#[test]
fn a_resource_command_completion_for_another_command_stays_late() {
    let mut app = App::new();
    let initial = refresh_request(app.update(docker_discovery().into_event()));
    app.update(refresh_completed(
        initial,
        Ok(snapshot(&[("container-a", "api", "nginx:1.27")])),
    ));
    let restart = command_request(app.invoke(Command::Resource(ResourceCommand::Restart)));
    let ProviderRequest::ExecuteResourceCommand {
        request_id,
        provider_id,
        target,
        ..
    } = &restart
    else {
        panic!("expected Resource Command request");
    };

    let requests = app.update(AppEvent::ResourceCommandCompleted {
        request_id: *request_id,
        provider_id: provider_id.clone(),
        target: target.clone(),
        command: ResourceCommand::Stop,
        result: Ok(()),
    });

    assert!(requests.is_empty());
    assert_eq!(app.state().running_commands.len(), 1);
    assert_eq!(
        app.update(command_completed(restart, Ok(()))).len(),
        1,
        "the matching completion still refreshes its Provider Workspace"
    );
}

#[test]
fn switching_provider_workspaces_keeps_an_in_flight_resource_command() {
    let mut app = App::new();
    ready_workspace(
        &mut app,
        docker_discovery(),
        snapshot(&[("container-a", "api", "nginx:1.27")]),
    );
    let restart = command_request(app.invoke(Command::Resource(ResourceCommand::Restart)));
    app.update(incus_discovery().into_event());
    let incus_refresh = refresh_request(app.invoke(Command::NextWorkspace));
    app.update(refresh_completed(
        incus_refresh,
        Ok(incus_snapshot(&[("instance-a", "gateway", "Running")])),
    ));

    app.update(command_completed(
        restart,
        Err(WorkspaceError::new("permission denied")),
    ));

    let screen = render_to_text(app.state(), 160, 24);
    assert!(
        screen.contains("Docker restart failed for api (container-a): permission denied"),
        "rendered screen:\n{screen}"
    );
}

#[test]
fn a_running_resource_command_marks_its_resource_without_replacing_the_command_bar() {
    let mut app = App::new();
    ready_workspace(
        &mut app,
        docker_discovery(),
        snapshot(&[("container-a", "api", "nginx:1.27")]),
    );

    app.invoke(Command::Resource(ResourceCommand::Restart));

    let screen = render_to_text(app.state(), 160, 24);
    assert!(screen.contains("* api"), "rendered screen:\n{screen}");
    assert!(
        screen.contains("Running restart for api…"),
        "rendered screen:\n{screen}"
    );
    assert!(screen.contains("r  Restart"), "rendered screen:\n{screen}");
    assert!(
        screen.contains("?  all commands"),
        "rendered screen:\n{screen}"
    );
}

#[test]
fn concurrent_resource_commands_mark_every_affected_resource() {
    let mut app = App::new();
    ready_workspace(
        &mut app,
        docker_discovery(),
        snapshot(&[
            ("container-a", "api", "nginx:1.27"),
            ("container-b", "worker", "alpine:3.21"),
        ]),
    );

    app.invoke(Command::Resource(ResourceCommand::Restart));
    app.invoke(Command::SelectNext);
    app.invoke(Command::Resource(ResourceCommand::Restart));

    let screen = render_to_text(app.state(), 100, 24);
    assert!(screen.contains("* api"), "rendered screen:\n{screen}");
    assert!(screen.contains("* worker"), "rendered screen:\n{screen}");
}

#[test]
fn a_successful_resource_command_refreshes_only_its_own_provider_workspace() {
    let mut app = App::new();
    ready_workspace(
        &mut app,
        docker_discovery(),
        snapshot(&[("container-a", "api", "nginx:1.27")]),
    );
    let restart = command_request(app.invoke(Command::Resource(ResourceCommand::Restart)));
    app.update(incus_discovery().into_event());
    let incus_refresh = refresh_request(app.invoke(Command::NextWorkspace));
    app.update(refresh_completed(
        incus_refresh,
        Ok(incus_snapshot(&[("instance-a", "gateway", "Running")])),
    ));

    let follow_up = app.update(command_completed(restart, Ok(())));

    assert!(
        follow_up.is_empty(),
        "a Docker Resource Command does not refresh the active Incus workspace"
    );
}

#[test]
fn a_failed_resource_command_opens_an_error_popup_over_another_active_workspace() {
    let mut app = App::new();
    ready_workspace(
        &mut app,
        docker_discovery(),
        snapshot(&[("container-a", "api", "nginx:1.27")]),
    );
    let restart = command_request(app.invoke(Command::Resource(ResourceCommand::Restart)));
    app.update(incus_discovery().into_event());
    let incus_refresh = refresh_request(app.invoke(Command::NextWorkspace));
    app.update(refresh_completed(
        incus_refresh,
        Ok(incus_snapshot(&[("instance-a", "gateway", "Running")])),
    ));

    app.update(command_completed(
        restart,
        Err(WorkspaceError::new("permission denied")),
    ));

    let screen = render_to_text(app.state(), 160, 24);
    assert!(
        screen.contains("Command failed"),
        "rendered screen:\n{screen}"
    );
    assert!(
        screen.contains("Docker restart failed for api (container-a): permission denied"),
        "rendered screen:\n{screen}"
    );
}

#[test]
fn escape_dismisses_the_command_failure_popup_without_quitting() {
    let mut app = App::new();
    ready_workspace(
        &mut app,
        docker_discovery(),
        snapshot(&[("container-a", "api", "nginx:1.27")]),
    );
    let restart = command_request(app.invoke(Command::Resource(ResourceCommand::Restart)));
    app.update(command_completed(
        restart,
        Err(WorkspaceError::new("permission denied")),
    ));

    assert!(
        render_to_text(app.state(), 160, 24).contains("Press Esc to dismiss."),
        "the failure popup says how to dismiss it"
    );
    let (control, requests) = handle_key(&mut app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

    assert_eq!(control, ShellControl::Continue);
    assert!(requests.is_empty());
    let screen = render_to_text(app.state(), 160, 24);
    assert!(
        !screen.contains("Command failed"),
        "rendered screen:\n{screen}"
    );
}

#[test]
fn the_command_failure_popup_keeps_the_whole_message_on_a_narrow_terminal() {
    let mut app = App::new();
    ready_workspace(
        &mut app,
        docker_discovery(),
        snapshot(&[("container-a", "api", "nginx:1.27")]),
    );
    let restart = command_request(app.invoke(Command::Resource(ResourceCommand::Restart)));

    app.update(command_completed(
        restart,
        Err(WorkspaceError::new("permission denied")),
    ));

    let screen = render_to_text(app.state(), 60, 24);
    assert!(
        screen.contains("denied"),
        "the popup wraps instead of clipping its message:\n{screen}"
    );
}

#[test]
fn a_successful_resource_command_clears_its_status_without_opening_a_popup() {
    let mut app = App::new();
    ready_workspace(
        &mut app,
        docker_discovery(),
        snapshot(&[("container-a", "api", "nginx:1.27")]),
    );
    let restart = command_request(app.invoke(Command::Resource(ResourceCommand::Restart)));
    assert!(
        render_to_text(app.state(), 160, 24).contains("Running restart for api"),
        "the dispatched Resource Command is visible while it runs"
    );

    let refresh = refresh_request(app.update(command_completed(restart, Ok(()))));
    app.update(refresh_completed(
        refresh,
        Ok(snapshot(&[("container-a", "api", "nginx:1.28")])),
    ));

    let screen = render_to_text(app.state(), 160, 24);
    assert!(
        !screen.contains("Running restart for api"),
        "rendered screen:\n{screen}"
    );
    assert!(
        !screen.contains("Command failed"),
        "rendered screen:\n{screen}"
    );
    assert!(
        screen.contains("Image: nginx:1.28"),
        "the refreshed Provider Workspace is the success feedback:\n{screen}"
    );
}

#[test]
fn switching_providers_invalidates_a_refresh_but_not_a_running_resource_command() {
    let mut app = App::new();
    ready_workspace(
        &mut app,
        docker_discovery(),
        snapshot(&[("container-a", "api", "nginx:1.27")]),
    );
    let restart = command_request(app.invoke(Command::Resource(ResourceCommand::Restart)));
    let stale_docker = refresh_request(app.invoke(Command::Refresh));
    app.update(incus_discovery().into_event());
    let incus_refresh = refresh_request(app.invoke(Command::NextWorkspace));
    app.update(refresh_completed(
        incus_refresh,
        Ok(incus_snapshot(&[("instance-a", "gateway", "Running")])),
    ));
    let current_screen = render_to_text(app.state(), 160, 24);

    app.update(refresh_completed(
        stale_docker,
        Ok(snapshot(&[("container-z", "zombie", "nginx:1.27")])),
    ));

    assert_eq!(
        render_to_text(app.state(), 160, 24),
        current_screen,
        "a stale refresh snapshot does not disturb the Active Workspace"
    );
    assert!(current_screen.contains("?  all commands"));

    app.invoke(Command::PreviousWorkspace);
    let docker_screen = render_to_text(app.state(), 160, 24);
    assert!(
        !docker_screen.contains("zombie"),
        "a stale refresh snapshot cannot overwrite newer application state:\n{docker_screen}"
    );

    app.update(command_completed(
        restart,
        Err(WorkspaceError::new("permission denied")),
    ));

    let screen = render_to_text(app.state(), 160, 24);
    assert!(
        screen.contains("Docker restart failed for api (container-a): permission denied"),
        "rendered screen:\n{screen}"
    );
    assert!(
        !screen.contains("Running Docker restart for api (container-a)"),
        "rendered screen:\n{screen}"
    );
}

#[test]
fn question_mark_shows_registered_commands_for_the_focused_resource() {
    let mut app = App::new();
    ready_workspace(
        &mut app,
        docker_discovery(),
        snapshot(&[("container-a", "api", "nginx:1.27")]),
    );

    let (_, requests) = handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE),
    );

    assert!(requests.is_empty());
    let screen = render_to_text(app.state(), 100, 24);
    assert!(
        screen.contains("Commands for api"),
        "rendered screen:\n{screen}"
    );
    assert!(screen.contains("S  Start"), "rendered screen:\n{screen}");
    assert!(screen.contains("s  Stop"), "rendered screen:\n{screen}");
    assert!(screen.contains("r  Restart"), "rendered screen:\n{screen}");
    assert!(screen.contains("d  Delete"), "rendered screen:\n{screen}");
}

#[test]
fn help_offers_resume_only_while_the_resource_is_suspended() {
    let mut paused = App::new();
    let initial = refresh_request(paused.update(docker_discovery().into_event()));
    paused.update(refresh_completed(
        initial,
        Ok(paused_snapshot(&[("container-a", "api", "nginx:1.27")])),
    ));
    handle_key(
        &mut paused,
        KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE),
    );
    let screen = render_to_text(paused.state(), 100, 24);
    assert!(screen.contains("p  Resume"), "rendered screen:\n{screen}");
    assert!(
        !screen.contains("p  Resume (unavailable)"),
        "rendered screen:\n{screen}"
    );

    let mut running = App::new();
    let initial = refresh_request(running.update(docker_discovery().into_event()));
    running.update(refresh_completed(
        initial,
        Ok(snapshot(&[("container-a", "api", "nginx:1.27")])),
    ));
    handle_key(
        &mut running,
        KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE),
    );
    let screen = render_to_text(running.state(), 100, 24);
    assert!(
        screen.contains("p  Resume (unavailable)"),
        "rendered screen:\n{screen}"
    );
}

/// A Resource whose Provider offers no Resource Shell Session must not look as
/// though pressing the key would do something. Nothing is asked for, and help
/// says why the key is there but idle.
#[test]
fn help_offers_a_shell_only_while_the_resource_can_host_one() {
    let mut stopped = App::new();
    let initial = refresh_request(stopped.update(docker_discovery().into_event()));
    stopped.update(refresh_completed(
        initial,
        Ok(stopped_snapshot(&[("container-a", "api", "nginx:1.27")])),
    ));

    handle_key(
        &mut stopped,
        KeyEvent::new(KeyCode::Char('E'), KeyModifiers::NONE),
    );
    assert!(
        stopped.state().resource_shell_sessions.is_empty(),
        "a stopped container has no shell to open"
    );

    handle_key(
        &mut stopped,
        KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE),
    );
    let screen = render_to_text(stopped.state(), 100, 24);
    assert!(
        screen.contains("E  Shell (unavailable)"),
        "rendered screen:\n{screen}"
    );

    let mut running = App::new();
    let initial = refresh_request(running.update(docker_discovery().into_event()));
    running.update(refresh_completed(
        initial,
        Ok(snapshot(&[("container-a", "api", "nginx:1.27")])),
    ));
    handle_key(
        &mut running,
        KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE),
    );
    let screen = render_to_text(running.state(), 100, 24);
    assert!(screen.contains("E  Shell"), "rendered screen:\n{screen}");
    assert!(
        !screen.contains("E  Shell (unavailable)"),
        "rendered screen:\n{screen}"
    );
}

#[test]
fn question_mark_closes_the_help_overlay_when_it_is_already_open() {
    let mut app = App::new();
    ready_workspace(
        &mut app,
        docker_discovery(),
        snapshot(&[("container-a", "api", "nginx:1.27")]),
    );

    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE),
    );
    let (_, requests) = handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE),
    );

    assert!(requests.is_empty());
    let screen = render_to_text(app.state(), 100, 24);
    assert!(
        !screen.contains("Commands for api"),
        "rendered screen:\n{screen}"
    );
    assert!(screen.contains("Containers"), "rendered screen:\n{screen}");
    assert!(screen.contains("api"), "rendered screen:\n{screen}");
}

/// `E` is the direct enlarged Resource Shell Session path. It selects Shell,
/// emits one host start effect, and gives that same session the enlarged
/// presentation without taking the outer terminal away from Tuivir.
#[test]
fn the_shell_key_starts_and_enlarges_the_selected_containers_session() {
    let mut app = App::new();
    ready_workspace(
        &mut app,
        docker_discovery(),
        snapshot(&[("container-a", "api", "nginx:1.27")]),
    );

    let (_, requests) = handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('E'), KeyModifiers::NONE),
    );

    assert!(
        requests.is_empty(),
        "a Resource Shell Session is never provider background work"
    );
    let session = app
        .state()
        .resource_shell_sessions
        .first()
        .expect("a Resource Shell Session waiting for the host")
        .clone();
    assert_eq!(session.provider_id, ProviderId::new("docker"));
    assert_eq!(
        session.target,
        ResourceTarget::new(
            ResourcePanelId::new("containers"),
            ResourceId::new("container-a"),
        )
    );
    assert_eq!(
        app.state().enlarged_resource_shell_session(),
        Some(&session),
        "E presents the same starting session enlarged"
    );
    assert_eq!(
        app.take_resource_shell_effects(),
        vec![ResourceShellEffect::Start {
            session: session.clone(),
            process: ResourceShellProcess::new(
                "docker",
                &["exec", "-it", "container-a", "/bin/sh"],
            ),
        }]
    );

    app.update(AppEvent::ResourceShellStarted {
        session_id: session.id,
    });
    app.invoke(Command::ToggleResourceShellSize);

    assert!(
        app.state().enlarged_resource_shell_session().is_none(),
        "the size toggle restores Details without replacing the session"
    );
    assert!(
        app.take_resource_shell_effects().is_empty(),
        "moving presentation never starts another Resource Shell Session"
    );

    app.invoke(Command::ToggleResourceShellSize);
    assert_eq!(
        app.state().enlarged_resource_shell_session(),
        Some(&app.state().resource_shell_sessions[0]),
        "repeated toggles keep the same running Resource Shell Session"
    );
    assert!(app.take_resource_shell_effects().is_empty());
}

#[test]
fn an_enlarged_resource_shell_keeps_its_identity_and_restore_hint_above_the_terminal() {
    let mut app = App::new();
    ready_workspace(
        &mut app,
        docker_discovery(),
        snapshot(&[("container-a", "api", "nginx:1.27")]),
    );

    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('E'), KeyModifiers::NONE),
    );

    let screen = render_to_text(app.state(), 80, 24);
    assert!(
        screen.contains("Docker / api"),
        "rendered screen:\n{screen}"
    );
    assert!(
        screen.contains("Ctrl-B q restore"),
        "rendered screen:\n{screen}"
    );
    assert!(
        !screen.contains("Containers"),
        "the enlarged shell owns the former Tuivir layout:\n{screen}"
    );
}

#[test]
fn an_enlarged_resource_shell_uses_every_row_below_its_one_line_header() {
    let mut app = App::new();
    ready_workspace(
        &mut app,
        docker_discovery(),
        snapshot(&[("container-a", "api", "nginx:1.27")]),
    );
    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('E'), KeyModifiers::NONE),
    );

    let layout = ScreenLayout::measure(app.state(), ratatui::layout::Rect::new(0, 0, 80, 24));
    let shell = layout
        .resource_shell
        .expect("enlarged Resource Shell Session layout");
    assert_eq!(shell.header, Some(ratatui::layout::Rect::new(0, 0, 80, 1)));
    assert_eq!(shell.terminal, ratatui::layout::Rect::new(0, 1, 80, 23));
    assert!(
        layout.panes.is_none(),
        "Tuivir panes are restored only on exit"
    );
}

/// Shell availability is provider-declared, but the Shell Detail View Tab is
/// supplied by Tuivir. Selecting it is purely navigational: a persistent
/// Resource Shell Session starts only after an explicit Enter gesture.
#[test]
fn selecting_the_shell_detail_view_tab_is_inert() {
    let mut app = App::new();
    ready_workspace(
        &mut app,
        docker_discovery(),
        snapshot(&[("container-a", "api", "nginx:1.27")]),
    );

    let requests = app.invoke(Command::ActivateDetailView(4));

    assert!(
        requests.is_empty(),
        "selecting Shell must not load provider details"
    );
    assert!(
        app.state().resource_shell_sessions.is_empty(),
        "selecting Shell must not start a session"
    );
    let screen = render_to_text(app.state(), 100, 24);
    assert!(screen.contains("[ Shell ]"), "rendered screen:\n{screen}");
}

#[test]
fn enter_on_the_shell_tab_starts_one_session_with_the_provider_command() {
    let mut app = App::new();
    ready_workspace(
        &mut app,
        docker_discovery(),
        snapshot(&[("container-a", "api", "nginx:1.27")]),
    );
    app.invoke(Command::ActivateDetailView(4));

    let (_, requests) = handle_key(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

    assert!(requests.is_empty(), "a Resource Shell Session is host work");
    let session = app
        .state()
        .resource_shell_sessions
        .first()
        .expect("a starting Resource Shell Session")
        .clone();
    assert_eq!(session.lifecycle, ResourceShellSessionLifecycle::Starting);
    assert_eq!(
        app.take_resource_shell_effects(),
        vec![ResourceShellEffect::Start {
            session,
            process: ResourceShellProcess::new(
                "docker",
                &["exec", "-it", "container-a", "/bin/sh"],
            ),
        }]
    );
}

#[test]
fn resource_shell_runtime_events_update_only_the_matching_session_lifecycle() {
    let mut app = App::new();
    ready_workspace(
        &mut app,
        docker_discovery(),
        snapshot(&[("container-a", "api", "nginx:1.27")]),
    );
    app.invoke(Command::ActivateDetailView(4));
    app.invoke(Command::StartResourceShell);
    let session_id = app.state().resource_shell_sessions[0].id;

    app.update(AppEvent::ResourceShellStarted { session_id });
    assert_eq!(
        app.state().resource_shell_sessions[0].lifecycle,
        ResourceShellSessionLifecycle::Running
    );

    app.update(AppEvent::ResourceShellExited { session_id });
    assert_eq!(
        app.state().resource_shell_sessions[0].lifecycle,
        ResourceShellSessionLifecycle::Exited
    );
    assert!(render_to_text(app.state(), 100, 24).contains("Session exited"));
}

/// Quitting with no live Resource Shell Session leaves the host free to exit
/// immediately: no confirmation is needed when no local process can be lost.
#[test]
fn quit_without_live_resource_shell_sessions_is_ready_immediately() {
    let mut app = App::new();

    assert!(app.invoke(Command::Quit).is_empty());

    assert!(app.quit_is_ready());
    assert!(app.state().confirmation.is_none());
}

/// A Quit confirmation protects live Provider CLI processes without changing
/// the Resources that own them. Cancelling preserves the exact running session;
/// confirming asks the host to stop and reap it before Tuivir exits.
#[test]
fn quitting_live_resource_shell_sessions_requires_confirmation_before_cleanup() {
    let mut app = App::new();
    ready_workspace(
        &mut app,
        docker_discovery(),
        snapshot(&[("container-a", "api", "nginx:1.27")]),
    );
    app.invoke(Command::OpenShell);
    let session_id = app.state().resource_shell_sessions[0].id;
    app.update(AppEvent::ResourceShellStarted { session_id });
    let _ = app.take_resource_shell_effects();

    app.invoke(Command::Quit);

    assert!(!app.quit_is_ready());
    assert!(
        render_to_text(app.state(), 100, 24)
            .contains("Quit Tuivir and end 1 Resource Shell Session?")
    );

    app.invoke(Command::Cancel);
    assert_eq!(
        app.state().resource_shell_sessions[0].lifecycle,
        ResourceShellSessionLifecycle::Running
    );
    assert!(app.take_resource_shell_effects().is_empty());

    app.invoke(Command::Quit);
    app.invoke(Command::Confirm);

    assert!(app.quit_is_ready());
    assert_eq!(
        app.take_resource_shell_effects(),
        vec![ResourceShellEffect::Stop { session_id }]
    );
}

/// An exited Resource Shell Session keeps its final screen until the user
/// deliberately starts a replacement. The replacement must forget the old
/// runtime before starting with a distinct identity, so late events from the
/// old lifetime cannot change the fresh session.
#[test]
fn restarting_an_exited_resource_shell_replaces_its_runtime_and_refuses_stale_events() {
    let mut app = App::new();
    ready_workspace(
        &mut app,
        docker_discovery(),
        snapshot(&[("container-a", "api", "nginx:1.27")]),
    );
    app.invoke(Command::ActivateDetailView(4));
    app.invoke(Command::StartResourceShell);
    let first = app.state().resource_shell_sessions[0].clone();
    let _ = app.take_resource_shell_effects();
    app.update(AppEvent::ResourceShellStarted {
        session_id: first.id,
    });
    app.update(AppEvent::ResourceShellExited {
        session_id: first.id,
    });

    app.invoke(Command::StartResourceShell);

    let replacement = app.state().resource_shell_sessions[0].clone();
    assert_ne!(replacement.id, first.id);
    assert_eq!(
        replacement.lifecycle,
        ResourceShellSessionLifecycle::Starting
    );
    assert_eq!(
        app.take_resource_shell_effects(),
        vec![
            ResourceShellEffect::Stop {
                session_id: first.id,
            },
            ResourceShellEffect::Start {
                session: replacement.clone(),
                process: ResourceShellProcess::new(
                    "docker",
                    &["exec", "-it", "container-a", "/bin/sh"],
                ),
            },
        ]
    );

    app.update(AppEvent::ResourceShellExited {
        session_id: first.id,
    });
    assert_eq!(
        app.state().resource_shell_sessions,
        vec![replacement],
        "a late exit belongs to the replaced session, not its successor"
    );
}

fn accept_resource_shell_removal(app: &mut App) -> ResourceShellSessionId {
    app.invoke(Command::OpenShell);
    let session_id = app.state().resource_shell_sessions[0].id;
    let _ = app.take_resource_shell_effects();

    let requests = app.update(AppEvent::RefreshTimerElapsed);
    let ProviderRequest::RefreshWorkspace {
        request_id,
        provider_id,
    } = requests.into_iter().next().expect("a refresh request")
    else {
        panic!("refresh timer requests the active workspace");
    };
    app.update(AppEvent::RefreshCompleted {
        request_id,
        provider_id,
        result: Ok(snapshot(&[])),
    });
    session_id
}

/// A removed Resource owns no lingering Session: accepting the new snapshot
/// asks the host to stop and reap its private PTY before state forgets it.
#[test]
fn an_accepted_refresh_stops_the_shell_for_a_removed_resource() {
    let mut app = App::new();
    ready_workspace(
        &mut app,
        docker_discovery(),
        snapshot(&[("container-a", "api", "nginx:1.27")]),
    );
    let session_id = accept_resource_shell_removal(&mut app);

    assert!(app.state().resource_shell_sessions.is_empty());
    assert_eq!(
        app.take_resource_shell_effects(),
        vec![ResourceShellEffect::Stop { session_id }]
    );
}

/// Deletion removes every application-owned trace of a Resource Shell Session,
/// including an enlarged presentation that would otherwise point at an absent
/// session. A failed refresh remains deliberately outside this reconciliation.
#[test]
fn accepted_resource_removal_restores_details_after_forgetting_its_shell() {
    let mut app = App::new();
    ready_workspace(
        &mut app,
        docker_discovery(),
        snapshot(&[("container-a", "api", "nginx:1.27")]),
    );
    let session_id = accept_resource_shell_removal(&mut app);

    assert!(app.state().resource_shell_sessions.is_empty());
    assert_eq!(
        app.state().resource_shell_presentation,
        ResourceShellPresentation::Details
    );
    assert_eq!(
        app.take_resource_shell_effects(),
        vec![ResourceShellEffect::Stop { session_id }]
    );
}

#[test]
fn unavailable_resource_command_is_disabled_in_help_and_does_not_dispatch() {
    let mut app = App::new();
    ready_workspace(
        &mut app,
        docker_discovery(),
        snapshot(&[("container-a", "api", "nginx:1.27")]),
    );

    let (_, requests) = handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('S'), KeyModifiers::NONE),
    );
    assert!(requests.is_empty(), "a running container cannot be started");

    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE),
    );
    let screen = render_to_text(app.state(), 100, 24);
    assert!(
        screen.contains("S  Start (unavailable)"),
        "rendered screen:\n{screen}"
    );
}

#[test]
fn delete_requires_target_identifying_confirmation_before_dispatch() {
    let mut app = App::new();
    ready_workspace(
        &mut app,
        docker_discovery(),
        snapshot(&[("container-a", "api", "nginx:1.27")]),
    );

    let (_, delete_requests) = handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE),
    );
    assert!(
        delete_requests.is_empty(),
        "no provider process runs before confirmation"
    );
    let confirmation = render_to_text(app.state(), 100, 24);
    assert!(
        confirmation.contains("Delete Docker resource api (container-a)?"),
        "rendered screen:\n{confirmation}"
    );

    let (_, confirmed_requests) = handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE),
    );
    assert!(matches!(
        confirmed_requests.as_slice(),
        [ProviderRequest::ExecuteResourceCommand {
            provider_id,
            target,
            command: ResourceCommand::Delete,
            ..
        }] if provider_id == &ProviderId::new("docker")
            && target == &ResourceTarget::new(
                ResourcePanelId::new("containers"),
                ResourceId::new("container-a"),
            )
    ));
}

#[test]
fn deleting_a_stateless_resource_confirms_permanent_removal_before_dispatch() {
    let mut app = App::new();
    ready_workspace(&mut app, docker_discovery(), stateless_snapshot());

    let workspace = render_to_text(app.state(), 100, 24);
    assert!(
        workspace.contains("cache · local"),
        "rendered screen:\n{workspace}"
    );

    let (_, delete_requests) = handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE),
    );
    assert!(
        delete_requests.is_empty(),
        "no provider process runs before confirmation"
    );

    let confirmation = render_to_text(app.state(), 100, 24);
    assert!(
        confirmation.contains("Delete Docker resource cache (cache-volume)?"),
        "rendered screen:\n{confirmation}"
    );
    assert!(
        confirmation.contains("It will be permanently removed."),
        "rendered screen:\n{confirmation}"
    );
    assert!(
        !confirmation.contains("It will be stopped and removed."),
        "rendered screen:\n{confirmation}"
    );

    let (_, confirmed_requests) = handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE),
    );
    assert!(matches!(
        confirmed_requests.as_slice(),
        [ProviderRequest::ExecuteResourceCommand {
            provider_id,
            target,
            command: ResourceCommand::Delete,
            state: None,
            ..
        }] if provider_id == &ProviderId::new("docker")
            && target == &ResourceTarget::new(
                ResourcePanelId::new("volumes"),
                ResourceId::new("cache-volume"),
            )
    ));
}

#[test]
fn cancelling_a_stateless_resource_deletion_dispatches_nothing() {
    let mut app = App::new();
    ready_workspace(&mut app, docker_discovery(), stateless_snapshot());

    assert!(
        app.invoke(Command::Resource(ResourceCommand::Delete))
            .is_empty()
    );
    assert!(app.invoke(Command::Cancel).is_empty());
    assert!(app.state().confirmation.is_none());
}

#[test]
fn a_delete_confirmation_is_a_raised_warning_surface() {
    let mut app = App::new();
    ready_workspace(
        &mut app,
        docker_discovery(),
        snapshot(&[("container-a", "api", "nginx:1.27")]),
    );

    app.invoke(Command::Resource(ResourceCommand::Delete));

    assert_eq!(
        foreground_of(app.state(), 100, 24, "Confirm deletion"),
        Color::Yellow
    );
    assert_eq!(
        background_of(
            app.state(),
            100,
            24,
            "Press y/Enter to confirm or n/Esc to cancel."
        ),
        Color::Black
    );
}

#[test]
fn help_and_failures_are_raised_semantic_surfaces() {
    let mut help = App::new();
    ready_workspace(
        &mut help,
        docker_discovery(),
        snapshot(&[("container-a", "api", "nginx:1.27")]),
    );
    handle_key(
        &mut help,
        KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE),
    );
    assert_eq!(
        foreground_of(help.state(), 100, 24, "Commands for api"),
        Color::Blue
    );
    assert_eq!(
        background_of(help.state(), 100, 24, "d  Delete"),
        Color::Black
    );

    let mut failed = App::new();
    ready_workspace(
        &mut failed,
        docker_discovery(),
        snapshot(&[("container-a", "api", "nginx:1.27")]),
    );
    let restart = command_request(failed.invoke(Command::Resource(ResourceCommand::Restart)));
    failed.update(command_completed(
        restart,
        Err(WorkspaceError::new("permission denied")),
    ));
    assert_eq!(
        foreground_of(failed.state(), 100, 24, "Command failed"),
        Color::Red
    );
    assert_eq!(
        background_of(failed.state(), 100, 24, "Press Esc to dismiss."),
        Color::Black
    );
}

#[test]
fn confirming_a_running_resource_warns_it_is_stopped_and_dispatches_its_state() {
    let mut app = App::new();
    ready_workspace(
        &mut app,
        docker_discovery(),
        snapshot(&[("container-a", "api", "nginx:1.27")]),
    );

    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE),
    );
    let confirmation = render_to_text(app.state(), 100, 24);
    assert!(
        confirmation.contains("It will be stopped and removed."),
        "rendered screen:\n{confirmation}"
    );

    let (_, confirmed_requests) = handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE),
    );
    assert!(matches!(
        confirmed_requests.as_slice(),
        [ProviderRequest::ExecuteResourceCommand {
            command: ResourceCommand::Delete,
            state: Some(ResourceState::Running),
            ..
        }]
    ));
}

/// A paused Resource is not running, so the prompt must not claim it is — but
/// removing it still stops it, and the deletion still has to force.
#[test]
fn confirming_a_paused_resource_warns_it_is_stopped_and_dispatches_its_state() {
    let mut app = App::new();
    ready_workspace(
        &mut app,
        docker_discovery(),
        paused_snapshot(&[("container-a", "api", "nginx:1.27")]),
    );

    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE),
    );
    let confirmation = render_to_text(app.state(), 100, 24);
    assert!(
        confirmation.contains("It will be stopped and removed."),
        "rendered screen:\n{confirmation}"
    );
    assert!(
        !confirmation.contains("is running"),
        "a paused Resource is never called running:\n{confirmation}"
    );

    let (_, confirmed_requests) = handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE),
    );
    assert!(matches!(
        confirmed_requests.as_slice(),
        [ProviderRequest::ExecuteResourceCommand {
            command: ResourceCommand::Delete,
            state: Some(ResourceState::Paused),
            ..
        }]
    ));
}

#[test]
fn confirming_a_stopped_resource_promises_no_stop_and_dispatches_its_state() {
    let mut app = App::new();
    ready_workspace(
        &mut app,
        docker_discovery(),
        stopped_snapshot(&[("container-a", "api", "nginx:1.27")]),
    );

    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE),
    );
    let confirmation = render_to_text(app.state(), 100, 24);
    assert!(
        confirmation.contains("Delete Docker resource api (container-a)?"),
        "rendered screen:\n{confirmation}"
    );
    assert!(
        !confirmation.contains("will be stopped"),
        "a stopped Resource is never promised a stop:\n{confirmation}"
    );

    let (_, confirmed_requests) = handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE),
    );
    assert!(matches!(
        confirmed_requests.as_slice(),
        [ProviderRequest::ExecuteResourceCommand {
            command: ResourceCommand::Delete,
            state: Some(ResourceState::Stopped),
            ..
        }]
    ));
}

#[test]
fn n_cancels_delete_confirmation_without_dispatching() {
    let mut app = App::new();
    ready_workspace(
        &mut app,
        docker_discovery(),
        snapshot(&[("container-a", "api", "nginx:1.27")]),
    );

    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE),
    );
    let (_, requests) = handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE),
    );

    assert!(requests.is_empty());
    let screen = render_to_text(app.state(), 100, 24);
    assert!(
        !screen.contains("Delete Docker resource api (container-a)?"),
        "rendered screen:\n{screen}"
    );
}

#[test]
fn escape_cancels_delete_confirmation_without_dispatching() {
    let mut app = App::new();
    ready_workspace(
        &mut app,
        docker_discovery(),
        snapshot(&[("container-a", "api", "nginx:1.27")]),
    );

    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE),
    );
    let (_, requests) = handle_key(&mut app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

    assert!(requests.is_empty());
    let screen = render_to_text(app.state(), 100, 24);
    assert!(
        !screen.contains("Delete Docker resource api (container-a)?"),
        "rendered screen:\n{screen}"
    );
}

#[test]
fn provider_bar_precedes_the_workspace_panes() {
    let mut app = App::new();
    ready_workspace(
        &mut app,
        docker_discovery(),
        snapshot(&[("container-a", "api", "nginx:1.27")]),
    );

    let screen = render_to_text(app.state(), 100, 24);
    let mut lines = screen.lines();
    assert!(
        lines
            .next()
            .expect("provider row")
            .starts_with("[1] Docker")
    );
    assert!(
        lines
            .next()
            .expect("workspace row")
            .starts_with(" ▶ [2] Containers")
    );
}

#[test]
fn numbered_panels_render_their_navigation_shortcuts() {
    let mut app = App::new();
    ready_workspace(
        &mut app,
        docker_discovery(),
        snapshot(&[("container-a", "api", "nginx:1.27")]),
    );

    let screen = render_to_text(app.state(), 100, 24);
    assert!(screen.starts_with("[1] Docker"));
    assert!(screen.contains("[2] Containers"));
}

#[test]
fn resource_panel_keeps_its_navigation_shortcut_while_loading_or_unavailable() {
    let mut loading_app = App::new();
    loading_app.update(docker_discovery().into_event());
    assert!(render_to_text(loading_app.state(), 100, 24).contains("[2] Resources"));

    let unavailable = ProviderDiscovery::new(
        docker_discovery().provider().clone(),
        Some(WorkspaceError::new("Docker is unavailable")),
    );
    let mut error_app = App::new();
    error_app.update(unavailable.into_event());
    assert!(render_to_text(error_app.state(), 100, 24).contains("[2] Error"));
}

#[test]
fn each_resource_state_has_a_coloured_symbol_without_repeating_its_status_text() {
    for (state, status, symbol, colour) in [
        (ResourceState::Running, "running", "●", Color::Green),
        (ResourceState::Stopped, "exited", "○", Color::DarkGray),
        (ResourceState::Paused, "paused", "‖", Color::Yellow),
        (ResourceState::Transitioning, "restarting", "↻", Color::Blue),
        (ResourceState::Broken, "dead", "✕", Color::Red),
        // An unrecognised Provider status stays neutral rather than borrowing
        // the colour of a state Tuivir understands.
        (ResourceState::Unknown, "teleporting", "?", Color::Reset),
    ] {
        let mut app = App::new();
        let initial = refresh_request(app.update(docker_discovery().into_event()));
        app.update(refresh_completed(
            initial,
            Ok(container_snapshot(
                &[("container-a", "api", "nginx:1.27")],
                state,
            )),
        ));

        let screen = render_to_text(app.state(), 100, 24);
        assert!(screen.contains(symbol), "rendered:\n{screen}");
        assert!(!screen.contains(status), "rendered:\n{screen}");
        assert_eq!(
            foreground_of(app.state(), 100, 24, symbol),
            colour,
            "{state:?} symbol should be rendered in {colour:?}"
        );
    }
}

#[test]
fn a_resource_name_is_left_uncoloured_by_its_resource_state() {
    let mut app = App::new();
    ready_workspace(
        &mut app,
        docker_discovery(),
        snapshot(&[("container-a", "api", "nginx:1.27")]),
    );

    assert_eq!(foreground_of(app.state(), 100, 24, "api"), Color::Reset);
}

#[test]
fn an_empty_resource_panel_is_compact_muted_and_uses_the_terminal_background() {
    let mut app = App::new();
    ready_workspace(&mut app, docker_discovery(), snapshot(&[]));

    let screen = render_to_text(app.state(), 100, 24);
    assert!(screen.contains("No resources"), "rendered:\n{screen}");
    assert!(
        !screen.contains("No Docker Containers found"),
        "the Panel title already identifies the Resources:\n{screen}"
    );
    assert_eq!(
        foreground_of(app.state(), 100, 24, "No resources"),
        Color::DarkGray
    );
    assert_eq!(
        background_of(app.state(), 100, 24, "No resources"),
        Color::Reset
    );
}

#[test]
fn bracket_keys_switch_the_active_workspace() {
    let mut app = App::new();
    ready_workspace(
        &mut app,
        docker_discovery(),
        snapshot(&[("container-a", "api", "nginx:1.27")]),
    );
    assert!(
        app.update(fixture_discovery().into_event()).is_empty(),
        "inactive workspaces remain idle"
    );

    let (_, requests) = handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char(']'), KeyModifiers::NONE),
    );
    assert_eq!(requests.len(), 1, "new Active Workspace is refreshed");
    assert!(render_to_text(app.state(), 100, 24).starts_with("[1] Docker   [1] Fixture"));

    let (_, requests) = handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('['), KeyModifiers::NONE),
    );
    // Returning refreshes Docker and asks again for the detail view whose load
    // was abandoned on the way out.
    assert_eq!(requests.len(), 1, "unexpected requests: {requests:?}");
    assert!(render_to_text(app.state(), 100, 24).starts_with("[1] Docker   [1] Fixture"));
}

#[test]
fn numbered_provider_panel_activates_incus_and_requests_its_refresh() {
    let mut app = App::new();
    ready_workspace(
        &mut app,
        docker_discovery(),
        snapshot(&[("container-a", "api", "nginx:1.27")]),
    );
    assert!(
        app.update(incus_discovery().into_event()).is_empty(),
        "inactive workspaces remain idle"
    );

    let (_, focus_requests) = handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE),
    );
    assert!(focus_requests.is_empty());
    let (_, activation_requests) = handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
    );

    let request = activation_requests
        .into_iter()
        .next()
        .expect("activating Incus requests an immediate refresh");
    assert!(matches!(
        request,
        ProviderRequest::RefreshWorkspace {
            provider_id,
            ..
        } if provider_id == ProviderId::new("incus")
    ));
    assert!(render_to_text(app.state(), 100, 24).starts_with("▶ [1] Docker   [1] Incus"));
}

#[test]
fn late_docker_result_cannot_replace_the_active_incus_workspace() {
    let mut app = App::new();
    let stale_docker = refresh_request(app.update(docker_discovery().into_event()));
    app.update(incus_discovery().into_event());
    app.invoke(Command::FocusProviders);
    let incus_request = refresh_request(app.invoke(Command::NextWorkspace));
    app.update(refresh_completed(
        incus_request,
        Ok(incus_snapshot(&[("instance-a", "gateway", "Running")])),
    ));
    let current_screen = render_to_text(app.state(), 100, 24);

    app.update(refresh_completed(
        stale_docker,
        Ok(snapshot(&[("container-a", "api", "nginx:1.27")])),
    ));

    assert_eq!(render_to_text(app.state(), 100, 24), current_screen);
    assert!(current_screen.contains("Target: local / default"));
    assert!(current_screen.contains("gateway"));
    assert!(!current_screen.contains("nginx:1.27"));
}

/// A resource list long enough that a five-item jump lands inside it and a
/// second one runs off the end.
fn seven_resources() -> WorkspaceSnapshot {
    snapshot(&[
        ("c0", "r0", "i0"),
        ("c1", "r1", "i1"),
        ("c2", "r2", "i2"),
        ("c3", "r3", "i3"),
        ("c4", "r4", "i4"),
        ("c5", "r5", "i5"),
        ("c6", "r6", "i6"),
    ])
}

fn snapshot(containers: &[(&str, &str, &str)]) -> WorkspaceSnapshot {
    container_snapshot(containers, ResourceState::Running)
}

fn stateless_snapshot() -> WorkspaceSnapshot {
    WorkspaceSnapshot {
        panels: vec![ResourcePanel {
            id: ResourcePanelId::new("volumes"),
            title: "Volumes".to_owned(),
            detail_views: Vec::new(),
            resources: vec![Resource {
                id: ResourceId::new("cache-volume"),
                name: "cache".to_owned(),
                secondary_text: Some("local".to_owned()),
                status: None,
                state: None,
                fields: Vec::new(),
                snapshot_details: Vec::new(),
                available_commands: &[ResourceCommand::Delete],
                shell: None,
            }],
        }],
    }
}

fn stopped_snapshot(containers: &[(&str, &str, &str)]) -> WorkspaceSnapshot {
    container_snapshot(containers, ResourceState::Stopped)
}

fn paused_snapshot(containers: &[(&str, &str, &str)]) -> WorkspaceSnapshot {
    container_snapshot(containers, ResourceState::Paused)
}

fn container_snapshot(
    containers: &[(&str, &str, &str)],
    state: ResourceState,
) -> WorkspaceSnapshot {
    let status = match state {
        ResourceState::Running => "running",
        ResourceState::Stopped => "exited",
        ResourceState::Paused => "paused",
        ResourceState::Transitioning => "restarting",
        ResourceState::Broken => "dead",
        ResourceState::Unknown => "teleporting",
    };
    let available_commands = lifecycle_commands(state, LifecycleCommandPolicy::RestartAndResume);
    WorkspaceSnapshot {
        panels: vec![ResourcePanel {
            id: ResourcePanelId::new("containers"),
            title: "Containers".to_owned(),
            detail_views: vec![
                DetailView::new("logs", "Logs"),
                DetailView::new("stats", "Stats"),
                DetailView::new("inspect", "Inspect"),
            ],
            resources: containers
                .iter()
                .map(|(id, name, image)| Resource {
                    id: ResourceId((*id).to_owned()),
                    name: (*name).to_owned(),
                    secondary_text: None,
                    status: Some(status.to_owned()),
                    state: Some(state),
                    fields: vec![("Image", (*image).to_owned())],
                    snapshot_details: Vec::new(),
                    available_commands,
                    shell: (state == ResourceState::Running).then(|| {
                        ResourceShellProcess::new("docker", &["exec", "-it", *id, "/bin/sh"])
                    }),
                })
                .collect(),
        }],
    }
}

fn incus_snapshot(instances: &[(&str, &str, &str)]) -> WorkspaceSnapshot {
    WorkspaceSnapshot {
        panels: vec![ResourcePanel {
            id: ResourcePanelId::new("instances"),
            title: "Instances".to_owned(),
            detail_views: vec![
                DetailView::new("info", "Info"),
                DetailView::new("config", "Config"),
                DetailView::new("console-log", "Console Log"),
            ],
            resources: instances
                .iter()
                .map(|(id, name, status)| {
                    let running = status.eq_ignore_ascii_case("running");
                    Resource {
                        id: ResourceId((*id).to_owned()),
                        name: (*name).to_owned(),
                        secondary_text: None,
                        status: Some((*status).to_owned()),
                        state: Some(if running {
                            ResourceState::Running
                        } else {
                            ResourceState::Stopped
                        }),
                        fields: vec![("Type", "container".to_owned())],
                        snapshot_details: Vec::new(),
                        available_commands: lifecycle_commands(
                            if running {
                                ResourceState::Running
                            } else {
                                ResourceState::Stopped
                            },
                            LifecycleCommandPolicy::RestartAndResume,
                        ),
                        shell: running.then(|| {
                            ResourceShellProcess::new("incus", &["exec", *name, "--", "su", "-l"])
                        }),
                    }
                })
                .collect(),
        }],
    }
}

fn docker_multi_panel_snapshot() -> WorkspaceSnapshot {
    WorkspaceSnapshot {
        panels: vec![
            ResourcePanel {
                id: ResourcePanelId::new("containers"),
                title: "Containers".to_owned(),
                detail_views: vec![
                    DetailView::new("logs", "Logs"),
                    DetailView::new("stats", "Stats"),
                    DetailView::new("inspect", "Inspect"),
                ],
                resources: vec![Resource {
                    id: ResourceId::new("shared-id"),
                    name: "api".to_owned(),
                    secondary_text: None,
                    status: Some("running".to_owned()),
                    state: Some(ResourceState::Running),
                    fields: vec![
                        ("Image", "nginx:1.27".to_owned()),
                        ("Status", "Up 3 hours".to_owned()),
                    ],
                    snapshot_details: Vec::new(),
                    available_commands: &[
                        ResourceCommand::Stop,
                        ResourceCommand::Restart,
                        ResourceCommand::Delete,
                    ],
                    shell: Some(ResourceShellProcess::new(
                        "docker",
                        &["exec", "-it", "shared-id", "/bin/sh"],
                    )),
                }],
            },
            ResourcePanel {
                id: ResourcePanelId::new("images"),
                title: "Images".to_owned(),
                detail_views: vec![DetailView::new("inspect", "Inspect")],
                resources: vec![Resource {
                    id: ResourceId::new("shared-id"),
                    name: "nginx:1.27".to_owned(),
                    secondary_text: None,
                    status: None,
                    state: None,
                    fields: vec![
                        ("Repository", "nginx".to_owned()),
                        ("Tag", "1.27".to_owned()),
                        ("Identity", "sha256:shared-id".to_owned()),
                        ("Size", "192MB".to_owned()),
                    ],
                    snapshot_details: Vec::new(),
                    available_commands: &[],
                    shell: None,
                }],
            },
        ],
    }
}

/// Every selected Resource starts with its snapshot-backed Overview before the
/// Provider's own Detail View Tabs.
#[test]
fn a_selected_container_starts_on_snapshot_backed_overview() {
    let mut app = App::new();
    let initial = refresh_request(app.update(docker_discovery().into_event()));

    app.update(refresh_completed(
        initial,
        Ok(snapshot(&[("container-a", "api", "nginx:1.27")])),
    ));

    let screen = render_to_text(app.state(), 100, 24);
    assert!(
        screen.contains("[ Overview ]  Logs  Stats  Inspect"),
        "rendered:\n{screen}"
    );
    assert!(
        screen.find("[ Overview ]") < screen.find("Image: nginx:1.27"),
        "the Overview tab precedes its snapshot-backed content:\n{screen}"
    );
}

/// The visible Overview comes entirely from the Workspace Snapshot, so
/// selecting a Resource does not ask its Provider to load details yet.
#[test]
fn settling_on_a_resource_does_not_load_snapshot_backed_overview() {
    let mut app = App::new();
    let initial = refresh_request(app.update(docker_discovery().into_event()));

    let requests = app.update(refresh_completed(
        initial,
        Ok(snapshot(&[("container-a", "api", "nginx:1.27")])),
    ));

    assert!(requests.is_empty(), "unexpected requests: {requests:?}");
}

#[test]
fn a_detail_request_is_current_only_while_its_full_identity_remains_visible() {
    let mut app = App::new();
    ready_workspace(
        &mut app,
        docker_discovery(),
        snapshot(&[("container-a", "api", "nginx:1.27")]),
    );
    let request = first_provider_detail(&mut app);

    assert!(app.detail_request_is_current(&request));

    assert!(app.update(fixture_discovery().into_event()).is_empty());
    app.invoke(Command::NextWorkspace);

    assert!(!app.detail_request_is_current(&request));
}

/// Rendered at a width a terminal actually has, so a row that only fits on a
/// very wide screen fails here rather than in front of a user.
#[test]
fn docker_renders_every_provider_defined_resource_panel() {
    let mut app = App::new();
    let initial = refresh_request(app.update(docker_discovery().into_event()));

    app.update(refresh_completed(
        initial,
        Ok(docker_multi_panel_snapshot()),
    ));

    let screen = render_to_text(app.state(), 80, 30);
    assert!(screen.contains("Containers"), "rendered:\n{screen}");
    assert!(screen.contains("Containers (1)"), "rendered:\n{screen}");
    assert!(screen.contains("Images"), "rendered:\n{screen}");
    assert!(screen.contains("Images (1)"), "rendered:\n{screen}");
    assert!(screen.contains("api"), "rendered:\n{screen}");
    assert!(screen.contains("nginx:1.27"), "rendered:\n{screen}");
}

#[test]
fn every_resource_panel_advertises_its_effective_focus_key() {
    let registry =
        CommandRegistry::effective(&[("focus_resources_2".to_owned(), vec!["f7".to_owned()])])
            .expect("a valid override");
    let mut app = App::with_registry(registry);
    ready_workspace(&mut app, docker_discovery(), docker_multi_panel_snapshot());

    let screen = render_to_text(app.state(), 80, 30);

    assert!(screen.contains("[2] Containers"), "rendered:\n{screen}");
    assert!(screen.contains("[f7] Images"), "rendered:\n{screen}");
}

#[test]
fn focus_accents_a_pane_title_and_edge_without_a_thick_border() {
    let mut app = App::new();
    ready_workspace(&mut app, docker_discovery(), docker_multi_panel_snapshot());

    let screen = render_to_text(app.state(), 80, 30);
    assert!(screen.contains("▶ [2] Containers"), "rendered:\n{screen}");
    assert_eq!(
        foreground_of(app.state(), 80, 30, "▶ [2] Containers"),
        Color::Blue,
        "the focused Pane title uses the primary accent"
    );

    app.invoke(Command::FocusResourcePanel(1));
    let screen = render_to_text(app.state(), 80, 30);
    assert!(screen.contains("▶ [3] Images"), "rendered:\n{screen}");
    assert!(!screen.contains("▶ [2] Containers"));

    app.invoke(Command::FocusDetails);
    let screen = render_to_text(app.state(), 80, 30);
    assert!(
        screen.contains("┌ ▶ [enter] Details"),
        "rendered:\n{screen}"
    );

    app.invoke(Command::FocusProviders);
    let screen = render_to_text(app.state(), 80, 30);
    assert!(screen.starts_with("▶ [1] Docker"), "rendered:\n{screen}");
}

#[test]
fn selected_resource_uses_a_full_row_background_that_dims_without_focus() {
    let mut app = App::new();
    ready_workspace(&mut app, docker_discovery(), docker_multi_panel_snapshot());

    assert_eq!(background_of(app.state(), 80, 30, "api"), Color::Blue);

    app.invoke(Command::FocusDetails);

    assert_eq!(background_of(app.state(), 80, 30, "api"), Color::DarkGray);
}

#[test]
fn resource_panel_shows_a_visual_scrollbar_only_when_rows_overflow() {
    let mut app = App::new();
    ready_workspace(&mut app, docker_discovery(), seven_resources());

    assert!(render_to_text(app.state(), 100, 8).contains("█"));
    assert!(!render_to_text(app.state(), 100, 24).contains("█"));
}

#[test]
fn a_workspace_over_the_numbered_panel_capacity_is_refused() {
    let mut app = App::new();
    let initial = refresh_request(app.update(docker_discovery().into_event()));
    let template = docker_multi_panel_snapshot().panels.remove(0);
    let panels = (0..10)
        .map(|index| {
            let mut panel = template.clone();
            panel.id = ResourcePanelId::new(format!("panel-{index}"));
            panel.title = format!("Panel {index}");
            panel
        })
        .collect();

    app.update(refresh_completed(initial, Ok(WorkspaceSnapshot { panels })));

    assert!(matches!(
        app.state().providers[0].load_state(),
        WorkspaceLoadState::Error(error)
            if error.message.contains("supports at most 9 Resource Panels")
    ));
}

#[test]
fn provider_workspaces_render_as_navigation_segments_with_the_active_target_on_the_right() {
    let mut app = App::new();
    ready_workspace(&mut app, docker_discovery(), docker_multi_panel_snapshot());
    app.update(fixture_discovery().into_event());

    let screen = render_to_text(app.state(), 80, 30);
    let provider_bar = screen.lines().next().expect("a Provider bar");
    assert!(
        provider_bar.starts_with("[1] Docker   [1] Fixture"),
        "rendered:\n{screen}"
    );
    assert!(
        provider_bar.ends_with("Target: desktop-linux"),
        "rendered:\n{screen}"
    );
    assert!(!provider_bar.contains("Docker · desktop-linux"));
}

#[test]
fn the_default_target_environment_stays_visible_beside_the_provider_navigation() {
    let default_docker = ProviderDiscovery::new(
        Provider::new(
            ProviderId::new("docker"),
            "Docker",
            Some(TargetEnvironment::new("default")),
            None,
        ),
        None,
    );
    let mut app = App::new();
    ready_workspace(&mut app, default_docker, docker_multi_panel_snapshot());

    assert!(
        render_to_text(app.state(), 80, 30)
            .lines()
            .next()
            .expect("a Provider bar")
            .ends_with("Target: default")
    );
}

#[test]
fn the_active_workspace_stays_filled_when_the_providers_pane_has_focus() {
    let mut app = App::new();
    ready_workspace(&mut app, docker_discovery(), docker_multi_panel_snapshot());
    app.update(fixture_discovery().into_event());

    assert_eq!(
        background_of(app.state(), 80, 30, "Docker"),
        Color::Blue,
        "the Active Workspace has a filled segment"
    );
    assert_eq!(
        foreground_of(app.state(), 80, 30, "Fixture"),
        Color::DarkGray,
        "inactive Provider Workspaces are subdued"
    );

    app.invoke(Command::FocusProviders);
    assert!(
        render_to_text(app.state(), 80, 30).starts_with("▶ [1] Docker   [1] Fixture"),
        "the Providers Pane carries its own focus accent"
    );
    assert_eq!(
        background_of(app.state(), 80, 30, "Docker"),
        Color::Blue,
        "Pane focus does not remove the Active Workspace selection"
    );
}

#[test]
fn resource_panel_scroll_is_restored_when_focus_returns() {
    let mut app = App::new();
    let initial = refresh_request(app.update(docker_discovery().into_event()));
    let mut workspace = docker_multi_panel_snapshot();
    for index in 1..12 {
        let mut resource = workspace.panels[0].resources[0].clone();
        resource.id = ResourceId::new(format!("container-{index}"));
        resource.name = format!("worker-{index}");
        workspace.panels[0].resources.push(resource);
    }
    app.update(refresh_completed(initial, Ok(workspace)));

    for _ in 0..8 {
        app.invoke(Command::SelectNext);
    }
    app.invoke(Command::FocusResourcePanel(1));
    app.invoke(Command::FocusResourcePanel(0));

    let screen = render_to_text(app.state(), 80, 12);
    assert!(screen.contains("● worker-8"), "rendered:\n{screen}");
    assert!(!screen.contains("● api"), "rendered:\n{screen}");
}

#[test]
fn direct_focus_commands_target_each_resource_panel_and_details() {
    let mut app = App::new();
    ready_workspace(&mut app, docker_discovery(), docker_multi_panel_snapshot());

    app.invoke(Command::FocusResourcePanel(1));
    assert_eq!(app.state().focused_pane, FocusedPane::Resources);
    assert_eq!(app.active_scope(), CommandScope::ResourcePanel(1));

    app.invoke(Command::FocusDetails);
    assert_eq!(app.state().focused_pane, FocusedPane::Details);
    assert_eq!(app.active_scope(), CommandScope::Details);
}

#[test]
fn removing_the_focused_resource_panel_reconciles_its_command_scope() {
    let mut app = App::new();
    ready_workspace(&mut app, docker_discovery(), docker_multi_panel_snapshot());
    app.invoke(Command::FocusResourcePanel(1));
    assert_eq!(app.active_scope(), CommandScope::ResourcePanel(1));
    let refresh = refresh_request(app.invoke(Command::Refresh));

    app.update(refresh_completed(
        refresh,
        Ok(snapshot(&[("container-a", "api", "nginx:1.27")])),
    ));

    assert_eq!(app.active_scope(), CommandScope::ResourcePanel(0));
}

#[test]
fn focus_cycles_through_provider_order_and_wraps() {
    let mut app = App::new();
    ready_workspace(&mut app, docker_discovery(), docker_multi_panel_snapshot());

    let expected = [
        (FocusedPane::Resources, Some("images")),
        (FocusedPane::Details, None),
        (FocusedPane::Providers, None),
        (FocusedPane::Resources, Some("containers")),
    ];
    for (focused, panel_id) in expected {
        app.invoke(Command::FocusNextPane);
        assert_eq!(app.state().focused_pane, focused);
        if let Some(panel_id) = panel_id {
            assert_eq!(
                app.state().providers[0].focused_resource_panel(),
                Some(&ResourcePanelId::new(panel_id))
            );
        }
    }

    app.invoke(Command::FocusPreviousPane);
    assert_eq!(app.state().focused_pane, FocusedPane::Providers);
}

#[test]
fn every_workspace_and_resource_panel_restores_its_navigation_state() {
    let mut app = App::new();
    let docker_refresh = refresh_request(app.update(docker_discovery().into_event()));
    let mut docker_snapshot = docker_multi_panel_snapshot();
    for (panel_index, suffix) in [(0, "worker"), (1, "alpine:3.20")] {
        let mut second = docker_snapshot.panels[panel_index].resources[0].clone();
        second.id = ResourceId::new(format!("second-{panel_index}"));
        second.name = suffix.to_owned();
        docker_snapshot.panels[panel_index].resources.push(second);
    }
    app.update(refresh_completed(docker_refresh, Ok(docker_snapshot)));

    app.invoke(Command::SelectNext);
    app.invoke(Command::FocusResourcePanel(1));
    app.invoke(Command::SelectNext);
    app.update(incus_discovery().into_event());
    let incus_refresh = refresh_request(app.invoke(Command::NextWorkspace));
    app.update(refresh_completed(
        incus_refresh,
        Ok(incus_snapshot(&[("instance-a", "gateway", "Running")])),
    ));

    app.invoke(Command::PreviousWorkspace);

    let docker = &app.state().providers[0];
    let WorkspaceLoadState::Ready(snapshot) = docker.load_state() else {
        panic!("Docker is ready");
    };
    let view = docker.view(snapshot);
    assert_eq!(
        view.focused_resource_panel,
        Some(&ResourcePanelId::new("images"))
    );
    assert_eq!(app.state().focused_pane, FocusedPane::Resources);
    assert_eq!(
        view.panels()
            .map(|panel| {
                (
                    &panel.panel.id,
                    panel.selected_resource,
                    panel.selected_index,
                )
            })
            .collect::<Vec<_>>(),
        vec![
            (
                &ResourcePanelId::new("containers"),
                Some(&ResourceId::new("second-0")),
                1,
            ),
            (
                &ResourcePanelId::new("images"),
                Some(&ResourceId::new("second-1")),
                1,
            ),
        ]
    );
}

#[test]
fn selecting_an_image_routes_details_by_panel_and_resource() {
    let mut app = App::new();
    ready_workspace(&mut app, docker_discovery(), docker_multi_panel_snapshot());

    let requests = app.invoke(Command::FocusResourcePanel(1));
    assert!(requests.is_empty(), "unexpected requests: {requests:?}");
    let screen = render_to_text(app.state(), 160, 30);
    assert!(screen.contains("Repository: nginx"), "rendered:\n{screen}");
    assert!(screen.contains("Identity: sha256:shared-id"));
    assert!(screen.contains("[ Overview ]  Inspect"));
}

#[test]
fn stale_container_details_cannot_replace_selected_image_details() {
    let mut app = App::new();
    let initial = refresh_request(app.update(docker_discovery().into_event()));
    app.update(refresh_completed(
        initial,
        Ok(docker_multi_panel_snapshot()),
    ));
    let stale_container = first_provider_detail(&mut app);
    assert!(app.invoke(Command::FocusResourcePanel(1)).is_empty());
    let image_request = first_provider_detail(&mut app);
    app.update(details_completed(
        image_request,
        Ok(ResourceDetails::from_lines(["selected image details"])),
    ));

    app.update(details_completed(
        stale_container,
        Ok(ResourceDetails::from_lines(["stale container details"])),
    ));

    let screen = render_to_text(app.state(), 160, 30);
    assert!(screen.contains("selected image details"));
    assert!(!screen.contains("stale container details"));
}

/// The panel says it is working before the Provider answers, then shows what
/// the Provider returned.
#[test]
fn the_detail_panel_reports_loading_and_then_the_providers_own_output() {
    let mut app = App::new();
    let initial = refresh_request(app.update(docker_discovery().into_event()));
    app.update(refresh_completed(
        initial,
        Ok(snapshot(&[("container-a", "api", "nginx:1.27")])),
    ));
    let request = first_provider_detail(&mut app);

    let loading = render_to_text(app.state(), 100, 24);
    assert!(loading.contains("Loading Logs…"), "rendered:\n{loading}");

    app.update(details_completed(
        request,
        Ok(ResourceDetails::from_lines(["listening on port 80"])),
    ));

    let loaded = render_to_text(app.state(), 100, 24);
    assert!(
        loaded.contains("listening on port 80"),
        "rendered:\n{loaded}"
    );
    assert!(!loaded.contains("Loading Logs…"));
}

/// Moving between views loads the one that became visible, and only that one.
#[test]
fn moving_through_the_detail_views_loads_only_the_newly_visible_one() {
    let mut app = App::new();
    ready_workspace(
        &mut app,
        docker_discovery(),
        snapshot(&[("container-a", "api", "nginx:1.27")]),
    );
    app.invoke(Command::FocusDetails);

    let (_, requests) = handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE),
    );

    assert!(
        matches!(
            requests.as_slice(),
            [ProviderRequest::LoadResourceDetails { view_id, .. }]
                if view_id == &DetailViewId::new("logs")
        ),
        "unexpected requests: {requests:?}"
    );
    let screen = render_to_text(app.state(), 100, 24);
    assert!(
        screen.contains("Overview  [ Logs ]  Stats  Inspect"),
        "rendered:\n{screen}"
    );

    let (_, requests) = handle_key(&mut app, KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));

    assert!(requests.is_empty(), "unexpected requests: {requests:?}");
    assert!(render_to_text(app.state(), 100, 24).contains("[ Overview ]  Logs  Stats  Inspect"));
}

/// The views are a ring, so moving past either end lands on the other rather
/// than sticking.
#[test]
fn detail_views_wrap_around_at_both_ends() {
    let mut app = App::new();
    ready_workspace(
        &mut app,
        docker_discovery(),
        snapshot(&[("container-a", "api", "nginx:1.27")]),
    );

    app.invoke(Command::PreviousDetailView);
    assert!(render_to_text(app.state(), 100, 24).contains("Logs  Stats  Inspect  [ Shell ]"));

    app.invoke(Command::NextDetailView);
    assert!(
        render_to_text(app.state(), 100, 24).contains("[ Overview ]  Logs  Stats  Inspect  Shell")
    );
}

/// The view survives moving between Resources, so reading one kind of detail
/// down a list does not keep resetting to the first view.
#[test]
fn the_chosen_detail_view_survives_moving_to_another_resource() {
    let mut app = App::new();
    ready_workspace(
        &mut app,
        docker_discovery(),
        snapshot(&[
            ("container-a", "api", "nginx:1.27"),
            ("container-b", "worker", "alpine:3.21"),
        ]),
    );
    app.invoke(Command::NextDetailView);
    app.invoke(Command::NextDetailView);

    let requests = app.invoke(Command::SelectNext);

    assert!(
        matches!(
            requests.as_slice(),
            [ProviderRequest::LoadResourceDetails { target, view_id, .. }]
                if target == &ResourceTarget::new(
                    ResourcePanelId::new("containers"),
                    ResourceId::new("container-b"),
                )
                    && view_id == &DetailViewId::new("stats")
        ),
        "unexpected requests: {requests:?}"
    );
    assert!(render_to_text(app.state(), 100, 24).contains("Overview  Logs  [ Stats ]  Inspect"));
}

/// A user who moves off a Resource before its details arrive must not have the
/// panel filled in behind them.
#[test]
fn a_late_result_for_the_previous_resource_cannot_replace_current_details() {
    let mut app = App::new();
    let initial = refresh_request(app.update(docker_discovery().into_event()));
    app.update(refresh_completed(
        initial,
        Ok(snapshot(&[
            ("container-a", "api", "nginx:1.27"),
            ("container-b", "worker", "alpine:3.21"),
        ])),
    ));
    let stale = first_provider_detail(&mut app);
    let current = detail_request(app.invoke(Command::SelectNext));
    app.update(details_completed(
        current,
        Ok(ResourceDetails::from_lines(["worker is up"])),
    ));

    app.update(details_completed(
        stale,
        Ok(ResourceDetails::from_lines(["api is up"])),
    ));

    let screen = render_to_text(app.state(), 100, 24);
    assert!(screen.contains("worker is up"), "rendered:\n{screen}");
    assert!(!screen.contains("api is up"));
}

/// The same holds for the view: Logs arriving late must not overwrite the Stats
/// the user switched to.
#[test]
fn a_late_result_for_the_previous_detail_view_cannot_replace_current_details() {
    let mut app = App::new();
    let initial = refresh_request(app.update(docker_discovery().into_event()));
    app.update(refresh_completed(
        initial,
        Ok(snapshot(&[("container-a", "api", "nginx:1.27")])),
    ));
    let stale = first_provider_detail(&mut app);
    let current = detail_request(app.invoke(Command::NextDetailView));
    app.update(details_completed(
        current,
        Ok(ResourceDetails::from_lines(["CPU 2.40%"])),
    ));

    app.update(details_completed(
        stale,
        Ok(ResourceDetails::from_lines(["listening on port 80"])),
    ));

    let screen = render_to_text(app.state(), 100, 24);
    assert!(screen.contains("CPU 2.40%"), "rendered:\n{screen}");
    assert!(!screen.contains("listening on port 80"));
}

#[test]
fn a_late_docker_detail_result_cannot_reach_the_active_incus_workspace() {
    let mut app = App::new();
    let docker_refresh = refresh_request(app.update(docker_discovery().into_event()));
    app.update(refresh_completed(
        docker_refresh,
        Ok(snapshot(&[("container-a", "api", "nginx:1.27")])),
    ));
    let stale = first_provider_detail(&mut app);
    app.update(incus_discovery().into_event());
    app.invoke(Command::FocusProviders);
    let incus_refresh = refresh_request(app.invoke(Command::NextWorkspace));
    app.update(refresh_completed(
        incus_refresh,
        Ok(incus_snapshot(&[("instance-a", "gateway", "Running")])),
    ));
    let incus_details = first_provider_detail(&mut app);
    app.update(details_completed(
        incus_details,
        Ok(ResourceDetails::from_lines(["gateway is up"])),
    ));
    let current_screen = render_to_text(app.state(), 100, 24);

    app.update(details_completed(
        stale,
        Ok(ResourceDetails::from_lines(["api is up"])),
    ));

    assert_eq!(render_to_text(app.state(), 100, 24), current_screen);
    assert!(current_screen.contains("gateway is up"));
}

/// Invalidating a pending load leaves the workspace it belonged to without one,
/// so coming back has to ask again rather than wait forever for a result that
/// will now be refused.
#[test]
fn returning_to_a_workspace_whose_detail_load_was_invalidated_asks_for_it_again() {
    let mut app = App::new();
    let docker_refresh = refresh_request(app.update(docker_discovery().into_event()));
    app.update(refresh_completed(
        docker_refresh,
        Ok(snapshot(&[("container-a", "api", "nginx:1.27")])),
    ));
    let abandoned = first_provider_detail(&mut app);
    app.update(incus_discovery().into_event());
    app.invoke(Command::FocusProviders);
    app.invoke(Command::NextWorkspace);

    let requests = app.invoke(Command::PreviousWorkspace);

    let reloaded = detail_request(requests);
    assert!(
        matches!(
            &reloaded,
            ProviderRequest::LoadResourceDetails { view_id, .. }
                if view_id == &DetailViewId::new("logs")
        ),
        "unexpected request: {reloaded:?}"
    );
    app.update(details_completed(
        abandoned,
        Ok(ResourceDetails::from_lines(["abandoned output"])),
    ));
    app.update(details_completed(
        reloaded,
        Ok(ResourceDetails::from_lines(["listening on port 80"])),
    ));

    let screen = render_to_text(app.state(), 100, 24);
    assert!(
        screen.contains("listening on port 80"),
        "rendered:\n{screen}"
    );
    assert!(!screen.contains("abandoned output"));
}

/// Details are lazy, so the two-second clock must not keep re-running provider
/// work for a view that is already on screen.
#[test]
fn an_ordinary_refresh_neither_reloads_nor_discards_the_loaded_details() {
    let mut app = App::new();
    let initial = refresh_request(app.update(docker_discovery().into_event()));
    app.update(refresh_completed(
        initial,
        Ok(snapshot(&[
            ("container-a", "api", "nginx:1.27"),
            ("container-b", "worker", "alpine:3.21"),
        ])),
    ));
    let details = first_provider_detail(&mut app);
    app.update(details_completed(
        details,
        Ok(ResourceDetails::from_lines(["listening on port 80"])),
    ));

    let refresh = refresh_request(app.update(AppEvent::RefreshTimerElapsed));
    let requests = app.update(refresh_completed(
        refresh,
        Ok(snapshot(&[
            ("container-b", "worker", "alpine:3.21"),
            ("container-a", "api", "nginx:1.27"),
        ])),
    ));

    assert!(
        requests.is_empty(),
        "a refresh that keeps the selection asks for nothing: {requests:?}"
    );
    let screen = render_to_text(app.state(), 100, 24);
    assert!(
        screen.contains("listening on port 80"),
        "rendered:\n{screen}"
    );
}

/// When the selected Resource is gone, the selection moves and the details have
/// to follow it.
#[test]
fn a_refresh_that_removes_the_selected_resource_loads_the_new_selections_details() {
    let mut app = App::new();
    let initial = refresh_request(app.update(docker_discovery().into_event()));
    app.update(refresh_completed(
        initial,
        Ok(snapshot(&[("container-a", "api", "nginx:1.27")])),
    ));
    let details = first_provider_detail(&mut app);
    app.update(details_completed(
        details,
        Ok(ResourceDetails::from_lines(["listening on port 80"])),
    ));

    let refresh = refresh_request(app.update(AppEvent::RefreshTimerElapsed));
    let requests = app.update(refresh_completed(
        refresh,
        Ok(snapshot(&[("container-b", "worker", "alpine:3.21")])),
    ));

    let reloaded = detail_request(requests);
    assert!(
        matches!(
            &reloaded,
            ProviderRequest::LoadResourceDetails { target, .. }
                if target == &ResourceTarget::new(
                    ResourcePanelId::new("containers"),
                    ResourceId::new("container-b"),
                )
        ),
        "unexpected request: {reloaded:?}"
    );
    let screen = render_to_text(app.state(), 100, 24);
    assert!(screen.contains("Loading Logs…"), "rendered:\n{screen}");
    assert!(!screen.contains("listening on port 80"));
}

/// A container that has logged nothing is not a broken one, so the panel says
/// which view came back empty rather than leaving a blank area.
#[test]
fn a_detail_view_the_provider_answered_with_nothing_gets_its_own_empty_state() {
    let mut app = App::new();
    let initial = refresh_request(app.update(docker_discovery().into_event()));
    app.update(refresh_completed(
        initial,
        Ok(snapshot(&[("container-a", "api", "nginx:1.27")])),
    ));
    let details = first_provider_detail(&mut app);

    app.update(details_completed(details, Ok(ResourceDetails::default())));

    let screen = render_to_text(app.state(), 100, 24);
    assert!(
        screen.contains("Docker returned no Logs for api"),
        "rendered:\n{screen}"
    );
}

#[test]
fn a_failed_detail_view_names_the_provider_resource_and_view() {
    let mut app = App::new();
    let initial = refresh_request(app.update(docker_discovery().into_event()));
    app.update(refresh_completed(
        initial,
        Ok(snapshot(&[("container-a", "api", "nginx:1.27")])),
    ));
    let details = first_provider_detail(&mut app);

    app.update(details_completed(
        details,
        Err(WorkspaceError::new("Error: No such container")),
    ));

    let screen = render_to_text(app.state(), 100, 24);
    assert!(
        screen.contains("Docker Logs failed for api"),
        "rendered:\n{screen}"
    );
    assert!(screen.contains("Error: No such container"));
}

/// A failed view is the Provider's own failure, so it stays inside the detail
/// panel instead of taking over the screen the way a failed Command does.
#[test]
fn a_failed_detail_view_leaves_the_resource_list_and_its_commands_alone() {
    let mut app = App::new();
    let initial = refresh_request(app.update(docker_discovery().into_event()));
    app.update(refresh_completed(
        initial,
        Ok(snapshot(&[("container-a", "api", "nginx:1.27")])),
    ));
    let details = first_provider_detail(&mut app);

    app.update(details_completed(
        details,
        Err(WorkspaceError::new("Error: No such container")),
    ));

    let screen = render_to_text(app.state(), 100, 24);
    assert!(screen.contains("api"), "rendered:\n{screen}");
    assert!(!screen.contains("Press Esc to dismiss."));
    let (_, requests) = handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE),
    );
    assert!(
        requests.iter().any(|request| matches!(
            request,
            ProviderRequest::ExecuteResourceCommand {
                command: ResourceCommand::Restart,
                ..
            }
        )),
        "workspace Commands still resolve: {requests:?}"
    );
}

/// More output than fits has to be reachable, and neither end may be
/// overshot.
#[test]
fn scrolling_moves_through_a_long_detail_view_and_clamps_at_both_ends() {
    let mut app = App::new();
    let initial = refresh_request(app.update(docker_discovery().into_event()));
    app.update(refresh_completed(
        initial,
        Ok(snapshot(&[("container-a", "api", "nginx:1.27")])),
    ));
    let details = first_provider_detail(&mut app);
    app.update(details_completed(
        details,
        Ok(ResourceDetails::from_lines(
            (0..30).map(|line| format!("line-{line}")),
        )),
    ));
    assert!(render_to_text(app.state(), 100, 24).contains("line-0"));
    app.invoke(Command::FocusDetails);

    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE),
    );

    let screen = render_to_text(app.state(), 100, 24);
    assert!(!screen.contains("line-0"), "rendered:\n{screen}");
    assert!(screen.contains("line-10"));

    // Far past the end lands on the last line rather than scrolling into blank
    // space below it.
    for _ in 0..10 {
        app.invoke(Command::ScrollDetailsDown);
    }
    let screen = render_to_text(app.state(), 100, 24);
    assert!(screen.contains("line-29"), "rendered:\n{screen}");

    for _ in 0..10 {
        app.invoke(Command::ScrollDetailsUp);
    }
    let screen = render_to_text(app.state(), 100, 24);
    assert!(screen.contains("line-0"), "rendered:\n{screen}");
}

#[test]
fn detail_source_lines_clip_at_the_panel_edge_instead_of_wrapping() {
    let mut app = App::new();
    let initial = refresh_request(app.update(docker_discovery().into_event()));
    app.update(refresh_completed(
        initial,
        Ok(snapshot(&[("container-a", "api", "nginx:1.27")])),
    ));
    let details = first_provider_detail(&mut app);
    app.update(details_completed(
        details,
        Ok(ResourceDetails::from_lines([
            "visible-prefix--------------------------------SHOULD-BE-CLIPPED",
            "second-source-line",
        ])),
    ));

    let screen = render_to_text(app.state(), 60, 20);
    assert!(screen.contains("visible-prefix"), "rendered:\n{screen}");
    assert!(screen.contains("second-source-line"), "rendered:\n{screen}");
    assert!(!screen.contains("SHOULD"), "rendered:\n{screen}");
}

#[test]
fn copying_a_details_selection_returns_exact_source_text() {
    let mut app = App::new();
    let initial = refresh_request(app.update(docker_discovery().into_event()));
    app.update(refresh_completed(
        initial,
        Ok(snapshot(&[("container-a", "api", "nginx:1.27")])),
    ));
    let details = first_provider_detail(&mut app);
    app.update(details_completed(
        details,
        Ok(ResourceDetails::from_lines(["hello", "world"])),
    ));

    app.invoke(Command::BeginDetailsSelection { line: 0, column: 1 });
    app.invoke(Command::ExtendDetailsSelection { line: 1, column: 3 });
    app.invoke(Command::CopyDetails);

    assert_eq!(
        app.take_pending_details_copy(),
        Some("ello\nwor".to_owned())
    );
}

#[test]
fn selected_details_text_is_visibly_highlighted() {
    let mut app = App::new();
    let initial = refresh_request(app.update(docker_discovery().into_event()));
    app.update(refresh_completed(
        initial,
        Ok(snapshot(&[("container-a", "api", "nginx:1.27")])),
    ));
    let details = first_provider_detail(&mut app);
    app.update(details_completed(
        details,
        Ok(ResourceDetails::from_lines(["hello"])),
    ));
    app.invoke(Command::BeginDetailsSelection { line: 0, column: 1 });
    app.invoke(Command::ExtendDetailsSelection { line: 0, column: 4 });

    let screen = render_to_text(app.state(), 100, 24);
    let backgrounds = render_background_colours(app.state(), 100, 24);
    let (row, column) = screen
        .lines()
        .enumerate()
        .find_map(|(row, line)| {
            line.find("hello")
                .map(|offset| (row, line[..offset].chars().count()))
        })
        .expect("detail text is rendered");

    assert_eq!(
        &backgrounds[row][column + 1..column + 4],
        &[Color::Blue, Color::Blue, Color::Blue],
        "screen:\n{screen}",
    );
}

#[test]
fn changing_the_selected_resource_clears_details_selection() {
    let mut app = App::new();
    let initial = refresh_request(app.update(docker_discovery().into_event()));
    app.update(refresh_completed(
        initial,
        Ok(snapshot(&[
            ("container-a", "api", "nginx:1.27"),
            ("container-b", "worker", "alpine:3.21"),
        ])),
    ));
    let details = first_provider_detail(&mut app);
    app.update(details_completed(
        details,
        Ok(ResourceDetails::from_lines(["hello"])),
    ));
    app.invoke(Command::BeginDetailsSelection { line: 0, column: 0 });
    app.invoke(Command::ExtendDetailsSelection { line: 0, column: 5 });
    app.invoke(Command::SelectNext);
    app.invoke(Command::CopyDetails);

    assert_eq!(app.take_pending_details_copy(), None);
}

#[test]
fn dragging_below_details_autoscrolls_and_extends_the_selection() {
    let mut app = App::new();
    let initial = refresh_request(app.update(docker_discovery().into_event()));
    app.update(refresh_completed(
        initial,
        Ok(snapshot(&[("container-a", "api", "nginx:1.27")])),
    ));
    let details = first_provider_detail(&mut app);
    app.update(details_completed(
        details,
        Ok(ResourceDetails::from_lines(
            (0..20).map(|line| format!("line-{line}")),
        )),
    ));
    app.invoke(Command::BeginDetailsSelection { line: 0, column: 0 });
    for _ in 0..3 {
        app.invoke(Command::ExtendDetailsSelectionAtEdge {
            above: false,
            column: 6,
            visible_rows: 1,
        });
    }
    app.invoke(Command::CopyDetails);

    assert_eq!(
        app.take_pending_details_copy(),
        Some("line-0\nline-1\nline-2\nline-3".to_owned())
    );
}

/// Every view starts at its own top: a scrolled position belongs to the output
/// it was scrolled through, not to the panel.
#[test]
fn moving_to_another_resource_starts_its_detail_view_at_the_top() {
    let mut app = App::new();
    let initial = refresh_request(app.update(docker_discovery().into_event()));
    app.update(refresh_completed(
        initial,
        Ok(snapshot(&[
            ("container-a", "api", "nginx:1.27"),
            ("container-b", "worker", "alpine:3.21"),
        ])),
    ));
    let details = first_provider_detail(&mut app);
    app.update(details_completed(
        details,
        Ok(ResourceDetails::from_lines(
            (0..30).map(|line| format!("api-{line}")),
        )),
    ));
    app.invoke(Command::ScrollDetailsDown);

    let worker = detail_request(app.invoke(Command::SelectNext));
    app.update(details_completed(
        worker,
        Ok(ResourceDetails::from_lines(
            (0..30).map(|line| format!("worker-{line}")),
        )),
    ));

    let screen = render_to_text(app.state(), 100, 24);
    assert!(screen.contains("worker-0"), "rendered:\n{screen}");
}

/// Detail navigation is registered like every other Command, so one override
/// moves dispatch and the help it is advertised in together.
#[test]
fn configured_detail_view_keys_change_dispatch_and_help_together() {
    let registry =
        CommandRegistry::effective(&[("detail_view_next".to_owned(), vec!["f12".to_owned()])])
            .expect("a valid override");
    let mut app = App::with_registry(registry);
    ready_workspace(
        &mut app,
        docker_discovery(),
        snapshot(&[("container-a", "api", "nginx:1.27")]),
    );

    app.invoke(Command::FocusDetails);
    handle_key(&mut app, KeyEvent::new(KeyCode::F(12), KeyModifiers::NONE));
    assert!(render_to_text(app.state(), 100, 24).contains("Overview  [ Logs ]  Stats  Inspect"));

    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE),
    );
    assert!(
        render_to_text(app.state(), 100, 24).contains("Overview  [ Logs ]  Stats  Inspect"),
        "the replaced default no longer moves the view"
    );

    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE),
    );
    let help = render_to_text(app.state(), 100, 30);
    assert!(
        help.contains("f12  Next detail view"),
        "help follows the override:\n{help}"
    );
    assert!(
        help.contains("h  Previous detail view"),
        "rendered:\n{help}"
    );
    assert!(help.contains("Scroll details down"), "rendered:\n{help}");
}

/// Incus details are Incus's own, so the shell must not dress them up as the
/// Docker views it happens to render the same way.
#[test]
fn a_selected_instance_offers_incus_views_rather_than_docker_equivalents() {
    let mut app = App::new();
    let initial = refresh_request(app.update(incus_discovery().into_event()));

    app.update(refresh_completed(
        initial,
        Ok(incus_snapshot(&[("instance-a", "gateway", "Running")])),
    ));

    let screen = render_to_text(app.state(), 100, 24);
    assert!(
        screen.contains("[ Overview ]  Info  Config  Console Log"),
        "rendered:\n{screen}"
    );
    for docker_view in ["Logs", "Stats", "Inspect"] {
        assert!(
            !screen.contains(docker_view),
            "Incus does not borrow Docker's {docker_view} view:\n{screen}"
        );
    }
}

#[test]
fn refresh_preserves_container_selection_by_stable_identity() {
    let mut app = App::new();
    ready_workspace(
        &mut app,
        docker_discovery(),
        snapshot(&[
            ("container-a", "api", "nginx:1.27"),
            ("container-b", "worker", "alpine:3.21"),
        ]),
    );
    app.invoke(Command::SelectNext);
    let refresh = refresh_request(app.invoke(Command::Refresh));

    app.update(refresh_completed(
        refresh,
        Ok(snapshot(&[
            ("container-b", "worker", "alpine:3.21"),
            ("container-c", "scheduler", "debian:bookworm"),
        ])),
    ));

    let screen = render_to_text(app.state(), 100, 24);
    assert!(
        screen.contains("Image: alpine:3.21"),
        "rendered screen:\n{screen}"
    );
    assert!(!screen.contains("Image: nginx:1.27"));
}

#[test]
fn resource_navigation_changes_the_selected_details() {
    let mut app = App::new();
    ready_workspace(
        &mut app,
        docker_discovery(),
        snapshot(&[
            ("container-a", "api", "nginx:1.27"),
            ("container-b", "worker", "alpine:3.21"),
        ]),
    );

    app.invoke(Command::SelectNext);
    let worker = render_to_text(app.state(), 100, 24);
    assert!(worker.contains("Image: alpine:3.21"));

    app.invoke(Command::SelectPrevious);
    let api = render_to_text(app.state(), 100, 24);
    assert!(api.contains("Image: nginx:1.27"));
}

#[test]
fn resource_navigation_keeps_earlier_rows_visible_until_the_selection_leaves_the_viewport() {
    let mut app = App::new();
    ready_workspace(
        &mut app,
        docker_discovery(),
        snapshot(&[
            ("container-a", "api", "nginx:1.27"),
            ("container-b", "worker", "alpine:3.21"),
        ]),
    );

    app.invoke(Command::SelectNext);

    let screen = render_to_text(app.state(), 100, 24);
    assert!(screen.contains("● api"), "rendered screen:\n{screen}");
    assert!(screen.contains("● worker"), "rendered screen:\n{screen}");
}

#[test]
fn automatic_and_manual_refreshes_do_not_overlap() {
    let mut app = App::new();
    let initial = refresh_request(app.update(docker_discovery().into_event()));

    assert!(app.update(AppEvent::RefreshTimerElapsed).is_empty());
    assert!(app.invoke(Command::Refresh).is_empty());

    app.update(refresh_completed(
        initial,
        Ok(snapshot(&[("container-a", "api", "nginx:1.27")])),
    ));
    let automatic = refresh_request(app.update(AppEvent::RefreshTimerElapsed));
    assert!(app.update(AppEvent::RefreshTimerElapsed).is_empty());
    assert!(app.invoke(Command::Refresh).is_empty());

    app.update(refresh_completed(
        automatic,
        Ok(snapshot(&[("container-a", "api", "nginx:1.27")])),
    ));
    assert_eq!(app.invoke(Command::Refresh).len(), 1);
}

#[test]
fn capital_j_moves_the_resource_selection_five_items_and_clamps_at_the_end() {
    let mut app = App::new();
    let initial = refresh_request(app.update(docker_discovery().into_event()));
    app.update(refresh_completed(initial, Ok(seven_resources())));

    // Five ahead from the first lands on the sixth resource.
    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('J'), KeyModifiers::SHIFT),
    );
    assert!(render_to_text(app.state(), 100, 24).contains("Image: i5"));

    // One more jumps past the end and clamps onto the last resource.
    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('J'), KeyModifiers::SHIFT),
    );
    assert!(render_to_text(app.state(), 100, 24).contains("Image: i6"));
}

#[test]
fn capital_k_moves_the_resource_selection_five_items_back_and_clamps_at_the_start() {
    let mut app = App::new();
    let initial = refresh_request(app.update(docker_discovery().into_event()));
    app.update(refresh_completed(initial, Ok(seven_resources())));
    // Park the selection on the last resource.
    for _ in 0..6 {
        app.invoke(Command::SelectNext);
    }

    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('K'), KeyModifiers::SHIFT),
    );
    assert!(render_to_text(app.state(), 100, 24).contains("Image: i1"));

    // One more would undershoot the start and clamps onto the first resource.
    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('K'), KeyModifiers::SHIFT),
    );
    assert!(render_to_text(app.state(), 100, 24).contains("Image: i0"));
}

/// Fast navigation is registered for resource lists only, so focusing the
/// Provider selector leaves `J` bound to nothing rather than moving the
/// resource selection out from under the user.
#[test]
fn fast_navigation_does_nothing_while_the_provider_selector_has_focus() {
    let mut app = App::new();
    let initial = refresh_request(app.update(docker_discovery().into_event()));
    app.update(refresh_completed(initial, Ok(seven_resources())));
    app.update(fixture_discovery().into_event());
    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE),
    );

    for pressed in ['J', 'K'] {
        let (control, requests) = handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char(pressed), KeyModifiers::SHIFT),
        );

        assert_eq!(control, ShellControl::Continue);
        assert!(requests.is_empty(), "{pressed} asked for no provider work");
        assert_eq!(
            app.state().active_provider,
            Some(0),
            "{pressed} left the Active Workspace alone"
        );
        assert!(
            render_to_text(app.state(), 100, 24).contains("Image: i0"),
            "{pressed} left the resource selection on the first resource"
        );
    }
}

/// Help is generated from the effective registry, so a resource list advertises
/// its five-item jumps under whichever keys are actually bound.
#[test]
fn help_lists_fast_navigation_under_its_effective_bindings() {
    let mut default = App::new();
    let initial = refresh_request(default.update(docker_discovery().into_event()));
    default.update(refresh_completed(initial, Ok(seven_resources())));

    handle_key(
        &mut default,
        KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE),
    );
    let help = render_to_text(default.state(), 100, 24);
    assert!(help.contains("J  Select five ahead"), "rendered:\n{help}");
    assert!(help.contains("K  Select five back"), "rendered:\n{help}");

    let registry =
        CommandRegistry::effective(&[("selection_next_fast".to_owned(), vec!["f5".to_owned()])])
            .expect("a valid override");
    let mut configured = App::with_registry(registry);
    let initial = refresh_request(configured.update(docker_discovery().into_event()));
    configured.update(refresh_completed(initial, Ok(seven_resources())));

    handle_key(
        &mut configured,
        KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE),
    );
    let help = render_to_text(configured.state(), 100, 24);
    assert!(
        help.contains("f5  Select five ahead"),
        "help follows the override:\n{help}"
    );
    assert!(
        !help.contains("J  Select five ahead"),
        "the replaced default is gone from help:\n{help}"
    );
}

/// Fast navigation is an ordinary configurable Command: rebinding it moves the
/// jump onto the configured keys and takes the defaults away with it.
#[test]
fn configured_fast_navigation_keys_replace_the_capital_defaults() {
    let registry = CommandRegistry::effective(&[
        ("selection_next_fast".to_owned(), vec!["f5".to_owned()]),
        ("selection_previous_fast".to_owned(), vec!["f6".to_owned()]),
    ])
    .expect("a valid override set");
    let mut app = App::with_registry(registry);
    let initial = refresh_request(app.update(docker_discovery().into_event()));
    app.update(refresh_completed(initial, Ok(seven_resources())));

    handle_key(&mut app, KeyEvent::new(KeyCode::F(5), KeyModifiers::NONE));
    assert!(
        render_to_text(app.state(), 100, 24).contains("Image: i5"),
        "the configured key jumps five ahead"
    );

    handle_key(&mut app, KeyEvent::new(KeyCode::F(6), KeyModifiers::NONE));
    assert!(
        render_to_text(app.state(), 100, 24).contains("Image: i0"),
        "the configured key jumps five back"
    );

    for replaced in ['J', 'K'] {
        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char(replaced), KeyModifiers::SHIFT),
        );
        assert!(
            render_to_text(app.state(), 100, 24).contains("Image: i0"),
            "the replaced default {replaced} no longer moves the selection"
        );
    }
}

/// An empty key list is how a user turns a Command off, so the jump stops
/// happening and stops being advertised.
#[test]
fn unbound_fast_navigation_neither_moves_the_selection_nor_appears_in_help() {
    let registry = CommandRegistry::effective(&[
        ("selection_next_fast".to_owned(), vec![]),
        ("selection_previous_fast".to_owned(), vec![]),
    ])
    .expect("unbinding is valid");
    let mut app = App::with_registry(registry);
    let initial = refresh_request(app.update(docker_discovery().into_event()));
    app.update(refresh_completed(initial, Ok(seven_resources())));

    for unbound in ['J', 'K'] {
        let (control, requests) = handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char(unbound), KeyModifiers::SHIFT),
        );

        assert_eq!(control, ShellControl::Continue);
        assert!(requests.is_empty());
        assert!(
            render_to_text(app.state(), 100, 24).contains("Image: i0"),
            "an unbound {unbound} leaves the selection where it was"
        );
    }

    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE),
    );
    let help = render_to_text(app.state(), 100, 24);
    assert!(
        !help.contains("Select five ahead"),
        "an unbound Command is not a control the user has:\n{help}"
    );
    assert!(
        !help.contains("Select five back"),
        "an unbound Command is not a control the user has:\n{help}"
    );
}

/// `ctrl+c` is reserved by the registry, so it quits even from inside a modal
/// that swallows every other key.
#[test]
fn ctrl_c_quits_even_while_a_confirmation_modal_is_open() {
    let mut app = App::new();
    ready_workspace(
        &mut app,
        docker_discovery(),
        snapshot(&[("container-a", "api", "nginx:1.27")]),
    );
    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE),
    );
    assert!(app.state().confirmation.is_some());

    let (control, requests) = handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
    );
    assert_eq!(control, ShellControl::Quit);
    assert!(requests.is_empty());
}

/// A workspace Command cannot fire while a modal is open: isolation comes from
/// scoped resolution, not the order of hard-coded branches.
#[test]
fn a_resource_command_key_does_not_dispatch_while_a_confirmation_modal_is_open() {
    let mut app = App::new();
    ready_workspace(
        &mut app,
        docker_discovery(),
        snapshot(&[("container-a", "api", "nginx:1.27")]),
    );
    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE),
    );
    assert!(app.state().confirmation.is_some());

    let (_, requests) = handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE),
    );
    assert!(
        requests.is_empty(),
        "Stop cannot fire while the delete confirmation is open"
    );
    assert!(
        app.state().confirmation.is_some(),
        "the confirmation modal is undisturbed"
    );
}

/// `Esc` backs out of modals but is no longer a global Quit: with nothing open
/// it does nothing rather than exit.
#[test]
fn escape_does_not_quit_when_no_modal_is_open() {
    let mut app = App::new();
    ready_workspace(
        &mut app,
        docker_discovery(),
        snapshot(&[("container-a", "api", "nginx:1.27")]),
    );

    let (control, requests) = handle_key(&mut app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

    assert_eq!(control, ShellControl::Continue);
    assert!(requests.is_empty());
}

#[test]
fn an_overridden_focus_key_renders_its_effective_hint() {
    let registry =
        CommandRegistry::effective(&[("focus_providers".to_owned(), vec!["f10".to_owned()])])
            .expect("a valid override");
    let mut app = App::with_registry(registry);
    ready_workspace(
        &mut app,
        docker_discovery(),
        snapshot(&[("container-a", "api", "nginx:1.27")]),
    );

    let screen = render_to_text(app.state(), 100, 24);
    assert!(
        screen.starts_with("[f10] Docker"),
        "the panel hint follows the effective binding:\n{screen}"
    );
    assert!(
        !screen.contains("[1] Docker"),
        "the replaced default hint is gone:\n{screen}"
    );
}

#[test]
fn an_unbound_focus_command_omits_its_inline_hint() {
    let registry =
        CommandRegistry::effective(&[("focus_resources".to_owned(), vec![])]).expect("unbinding");
    let mut app = App::with_registry(registry);
    ready_workspace(
        &mut app,
        docker_discovery(),
        snapshot(&[("container-a", "api", "nginx:1.27")]),
    );

    let screen = render_to_text(app.state(), 100, 24);
    assert!(
        !screen.contains("[2]"),
        "an unbound focus command shows no inline hint:\n{screen}"
    );
    assert!(screen.contains("Containers"));
}

#[test]
fn one_override_changes_dispatch_help_and_the_inline_hint_together() {
    let registry = CommandRegistry::effective(&[
        ("resource_restart".to_owned(), vec!["x".to_owned()]),
        ("focus_providers".to_owned(), vec!["f10".to_owned()]),
    ])
    .expect("a valid override set");
    let mut app = App::with_registry(registry);
    ready_workspace(
        &mut app,
        docker_discovery(),
        snapshot(&[("container-a", "api", "nginx:1.27")]),
    );

    // The inline hint follows the override.
    let screen = render_to_text(app.state(), 100, 24);
    assert!(screen.starts_with("[f10] Docker"), "rendered:\n{screen}");

    // Contextual help follows the override.
    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE),
    );
    let help = render_to_text(app.state(), 100, 24);
    assert!(help.contains("x  Restart"), "rendered:\n{help}");
    assert!(!help.contains("r  Restart"), "rendered:\n{help}");

    // Close help so the workspace scope is active, then dispatch the new key.
    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE),
    );
    let (_, requests) = handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE),
    );
    assert!(matches!(
        requests.as_slice(),
        [ProviderRequest::ExecuteResourceCommand {
            command: ResourceCommand::Restart,
            ..
        }]
    ));

    // The replaced default key no longer dispatches Restart.
    let (_, stale) = handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE),
    );
    assert!(stale.is_empty(), "the old Restart key is unbound");
}

/// The user moves the Pane Boundary in steps they can predict, so five presses
/// one way and five back leave the Panes where they started.
#[test]
fn moving_the_pane_boundary_steps_it_by_five_points_each_way() {
    let mut app = App::new();
    let start = app.state().pane_boundary.resources_percent();

    app.invoke(Command::MovePaneBoundaryRight);

    assert_eq!(
        app.state().pane_boundary.resources_percent(),
        start + 5,
        "moving right gives the Resource Panels more of the width"
    );

    app.invoke(Command::MovePaneBoundaryLeft);
    app.invoke(Command::MovePaneBoundaryLeft);

    assert_eq!(
        app.state().pane_boundary.resources_percent(),
        start - 5,
        "moving left gives the Details Pane more of the width"
    );
}

#[test]
fn a_restored_pane_boundary_is_the_starting_dimension_for_every_workspace() {
    let mut app =
        App::with_registry_and_pane_boundary(CommandRegistry::builtin(), PaneBoundary::new(60));
    app.update(docker_discovery().into_event());
    app.update(incus_discovery().into_event());

    assert_eq!(app.state().pane_boundary.resources_percent(), 60);
    app.invoke(Command::NextWorkspace);
    assert_eq!(app.state().pane_boundary.resources_percent(), 60);
}

/// Neither Pane may be squeezed out of usefulness, however long the user holds
/// the key down.
#[test]
fn the_pane_boundary_stops_at_the_edges_of_its_range() {
    let mut app = App::new();

    for _ in 0..20 {
        app.invoke(Command::MovePaneBoundaryRight);
    }

    assert_eq!(
        app.state().pane_boundary.resources_percent(),
        75,
        "the Details Pane keeps a quarter of the width"
    );

    for _ in 0..20 {
        app.invoke(Command::MovePaneBoundaryLeft);
    }

    assert_eq!(
        app.state().pane_boundary.resources_percent(),
        25,
        "the Resource Panels keep a quarter of the width"
    );
}

/// A drag is only the Pane Boundary's while the pointer holds it. Every other
/// drag on the screen reaches the application the same way and must not move
/// the Panes.
#[test]
fn the_pane_boundary_moves_only_while_the_pointer_holds_it() {
    let mut app = App::new();
    let start = app.state().pane_boundary.resources_percent();

    app.invoke(Command::SetPaneBoundary(60));

    assert_eq!(
        app.state().pane_boundary.resources_percent(),
        start,
        "a boundary nobody grabbed ignores the pointer"
    );

    app.invoke(Command::GrabPaneBoundary(0));
    app.invoke(Command::SetPaneBoundary(60));

    assert_eq!(app.state().pane_boundary.resources_percent(), 60);

    app.invoke(Command::ReleasePaneBoundary);
    app.invoke(Command::SetPaneBoundary(30));

    assert_eq!(
        app.state().pane_boundary.resources_percent(),
        60,
        "letting go stops the boundary following the pointer"
    );
}

/// A resize is only a resize. Which Pane has focus, which Resource is selected,
/// which Detail View Tab is on screen, and how far through it the user had read are
/// all still there afterwards.
#[test]
fn resizing_the_panes_disturbs_nothing_inside_them() {
    let mut app = App::new();
    let initial = refresh_request(app.update(docker_discovery().into_event()));
    app.update(refresh_completed(
        initial,
        Ok(snapshot(&[
            ("container-a", "api", "nginx:1.27"),
            ("container-b", "worker", "alpine:3.21"),
        ])),
    ));
    let details = first_provider_detail(&mut app);
    app.update(details_completed(
        details,
        Ok(ResourceDetails::from_lines(
            (0..30).map(|line| format!("line-{line}")),
        )),
    ));
    app.invoke(Command::FocusDetails);
    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE),
    );
    let selected_before = app.state().providers[0].selected_resource_target();
    let scrolled = render_to_text(app.state(), 100, 24);
    assert!(scrolled.contains("line-10"), "rendered:\n{scrolled}");
    assert!(
        !scrolled.contains("line-0 "),
        "the first line has been scrolled past, rendered:\n{scrolled}"
    );

    for _ in 0..4 {
        app.invoke(Command::MovePaneBoundaryRight);
    }

    assert_ne!(
        app.state().pane_boundary.resources_percent(),
        48,
        "the Panes really did change size"
    );
    assert_eq!(
        app.state().focused_pane,
        FocusedPane::Details,
        "the Pane the keyboard drives is still the one the user left it on"
    );
    assert_eq!(
        app.state().providers[0].selected_resource_target(),
        selected_before,
        "the selected Resource survives the resize"
    );
    let resized = render_to_text(app.state(), 100, 24);
    assert!(
        resized.contains("line-10") && !resized.contains("line-0 "),
        "the Detail View Tab is still where it was read to, rendered:\n{resized}"
    );
}

/// The Pane Boundary belongs to the run, not to one Provider Workspace. A user
/// who sized the Panes to read Docker logs finds them that size in Incus too.
#[test]
fn the_pane_boundary_survives_provider_workspace_navigation() {
    let mut app = App::new();
    app.update(docker_discovery().into_event());
    app.update(incus_discovery().into_event());
    for _ in 0..3 {
        app.invoke(Command::MovePaneBoundaryLeft);
    }
    let chosen = app.state().pane_boundary.resources_percent();
    assert_eq!(chosen, 33, "three steps left from the starting share");

    app.invoke(Command::NextWorkspace);

    assert_eq!(app.state().active_provider, Some(1), "Incus is active");
    assert_eq!(
        app.state().pane_boundary.resources_percent(),
        chosen,
        "the Panes keep the size the user gave them"
    );

    app.invoke(Command::PreviousWorkspace);

    assert_eq!(app.state().pane_boundary.resources_percent(), chosen);
}
