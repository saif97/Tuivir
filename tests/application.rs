use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::Color;
use tokio::sync::{Notify, mpsc};
use virtui::{
    app::{App, AppEvent, AppState},
    cli::{CliRunner, ProcessError, ProcessFailure, ProcessOutput, ProcessSpec},
    command::{Command, CommandRegistry},
    docker::DockerWorkspace,
    provider::{
        ProviderDiscovery, ProviderId, ProviderRequest, Resource, ResourceCommand, ResourceId,
        ResourcePanel, ResourceState, WorkspaceError, WorkspaceSnapshot,
    },
    runtime::{ProviderRuntime, RefreshTimer, ShellControl, handle_key},
    ui::{render_foreground_colours, render_to_text},
};

/// Reports the single foreground colour `text` is rendered in, panicking when
/// it is absent from the screen or split across colours.
fn foreground_of(state: &AppState, width: u16, height: u16, text: &str) -> Color {
    let screen = render_to_text(state, width, height);
    let colours = render_foreground_colours(state, width, height);
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
    ProviderDiscovery {
        id: ProviderId::new("docker"),
        name: "Docker".to_owned(),
        target_environment: "desktop-linux".to_owned(),
        error: None,
    }
}

fn fixture_discovery() -> ProviderDiscovery {
    ProviderDiscovery {
        id: ProviderId::new("fixture"),
        name: "Fixture".to_owned(),
        target_environment: "local".to_owned(),
        error: None,
    }
}

fn incus_discovery() -> ProviderDiscovery {
    ProviderDiscovery {
        id: ProviderId::new("incus"),
        name: "Incus".to_owned(),
        target_environment: "local / default".to_owned(),
        error: None,
    }
}

#[test]
fn first_available_provider_becomes_the_active_workspace() {
    let mut app = App::new();
    assert_eq!(app.state().active_provider, None);

    app.update(AppEvent::ProviderDiscovered(docker_discovery()));

    assert_eq!(app.state().active_provider, Some(0));
}

struct DelayedCli {
    started: Arc<Notify>,
    release: Arc<Notify>,
}

struct MissingCli;

struct ExpectedCli {
    commands: Arc<Mutex<Vec<ProcessSpec>>>,
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
    let initial = refresh_request(app.update(AppEvent::ProviderDiscovered(docker_discovery())));
    app.update(refresh_completed(
        initial,
        Ok(snapshot(&[("container-a", "api", "nginx:1.27")])),
    ));
    let command = app.invoke(Command::Resource(ResourceCommand::Restart));
    let commands = Arc::new(Mutex::new(Vec::new()));
    let runtime = ProviderRuntime::new(
        vec![Arc::new(DockerWorkspace) as Arc<dyn virtui::provider::ProviderWorkspace>],
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
            resource_id,
            command: ResourceCommand::Restart,
            result: Ok(()),
            ..
        } if provider_id == ProviderId::new("docker")
            && resource_id == ResourceId::new("container-a")
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
    let initial = refresh_request(app.update(AppEvent::ProviderDiscovered(docker_discovery())));
    app.update(refresh_completed(
        initial,
        Ok(snapshot(&[("container-a", "api", "nginx:1.27")])),
    ));
    let request = app
        .invoke(Command::Resource(ResourceCommand::Restart))
        .into_iter()
        .next()
        .expect("restart request");
    let runtime = ProviderRuntime::new(
        vec![Arc::new(DockerWorkspace) as Arc<dyn virtui::provider::ProviderWorkspace>],
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
    let initial = refresh_request(app.update(AppEvent::ProviderDiscovered(docker_discovery())));
    app.update(refresh_completed(
        initial,
        Ok(snapshot(&[("container-a", "api", "nginx:1.27")])),
    ));
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
    let initial = refresh_request(app.update(AppEvent::ProviderDiscovered(docker_discovery())));
    app.update(refresh_completed(
        initial,
        Ok(snapshot(&[
            ("container-a", "api", "nginx:1.27"),
            ("container-b", "worker", "alpine:3.21"),
        ])),
    ));
    let refresh = refresh_request(app.invoke(Command::Refresh));
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let runtime = ProviderRuntime::new(
        vec![Arc::new(DockerWorkspace) as Arc<dyn virtui::provider::ProviderWorkspace>],
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
        vec![Arc::new(DockerWorkspace) as Arc<dyn virtui::provider::ProviderWorkspace>],
        Arc::new(MissingCli),
    );
    let mut app = App::new();

    for discovered in runtime.discover().await {
        app.update(AppEvent::ProviderDiscovered(discovered));
    }

    let screen = render_to_text(app.state(), 100, 24);
    assert!(screen.contains("No providers discovered"));
    assert!(!screen.contains("Docker"));
}

#[test]
fn keyboard_commands_drive_navigation_manual_refresh_and_quit() {
    let mut app = App::new();
    let initial = refresh_request(app.update(AppEvent::ProviderDiscovered(docker_discovery())));
    app.update(refresh_completed(
        initial,
        Ok(snapshot(&[
            ("container-a", "api", "nginx:1.27"),
            ("container-b", "worker", "alpine:3.21"),
        ])),
    ));

    let (control, requests) = handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
    );
    assert_eq!(control, ShellControl::Continue);
    assert!(requests.is_empty());
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
    let initial = refresh_request(app.update(AppEvent::ProviderDiscovered(docker_discovery())));
    app.update(refresh_completed(
        initial,
        Ok(snapshot(&[("container-a", "api", "nginx:1.27")])),
    ));

    let (_, requests) = handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE),
    );

    assert!(matches!(
        requests.as_slice(),
        [ProviderRequest::ExecuteResourceCommand {
            provider_id,
            resource_id,
            resource_name,
            command: ResourceCommand::Restart,
            ..
        }] if provider_id == &ProviderId::new("docker")
            && resource_id == &ResourceId::new("container-a")
            && resource_name == "api"
    ));
}

#[test]
fn start_key_dispatches_for_a_stopped_instance() {
    let mut app = App::new();
    let initial = refresh_request(app.update(AppEvent::ProviderDiscovered(incus_discovery())));
    app.update(refresh_completed(
        initial,
        Ok(incus_snapshot(&[("instance-a", "gateway", "Stopped")])),
    ));

    let (_, requests) = handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('S'), KeyModifiers::SHIFT),
    );

    assert!(matches!(
        requests.as_slice(),
        [ProviderRequest::ExecuteResourceCommand {
            provider_id,
            resource_id,
            command: ResourceCommand::Start,
            ..
        }] if provider_id == &ProviderId::new("incus")
            && resource_id == &ResourceId::new("instance-a")
    ));
}

#[test]
fn stop_key_dispatches_for_a_running_container() {
    let mut app = App::new();
    let initial = refresh_request(app.update(AppEvent::ProviderDiscovered(docker_discovery())));
    app.update(refresh_completed(
        initial,
        Ok(snapshot(&[("container-a", "api", "nginx:1.27")])),
    ));

    let (_, requests) = handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE),
    );

    assert!(matches!(
        requests.as_slice(),
        [ProviderRequest::ExecuteResourceCommand {
            provider_id,
            resource_id,
            command: ResourceCommand::Stop,
            ..
        }] if provider_id == &ProviderId::new("docker")
            && resource_id == &ResourceId::new("container-a")
    ));
}

#[test]
fn resume_key_dispatches_for_a_paused_container_and_carries_its_state() {
    let mut app = App::new();
    let initial = refresh_request(app.update(AppEvent::ProviderDiscovered(docker_discovery())));
    app.update(refresh_completed(
        initial,
        Ok(paused_snapshot(&[("container-a", "api", "nginx:1.27")])),
    ));

    let (_, requests) = handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE),
    );

    assert!(matches!(
        requests.as_slice(),
        [ProviderRequest::ExecuteResourceCommand {
            provider_id,
            resource_id,
            resource_name,
            command: ResourceCommand::Resume,
            state: ResourceState::Paused,
            ..
        }] if provider_id == &ProviderId::new("docker")
            && resource_id == &ResourceId::new("container-a")
            && resource_name == "api"
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
        let initial = refresh_request(app.update(AppEvent::ProviderDiscovered(docker_discovery())));
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
    let initial = refresh_request(app.update(AppEvent::ProviderDiscovered(docker_discovery())));
    app.update(refresh_completed(
        initial,
        Ok(snapshot(&[
            ("container-a", "api", "nginx:1.27"),
            ("container-b", "worker", "alpine:3.21"),
        ])),
    ));
    app.invoke(Command::SelectNext);
    let request = app.invoke(Command::Resource(ResourceCommand::Restart));
    let ProviderRequest::ExecuteResourceCommand {
        request_id,
        provider_id,
        resource_id,
        resource_name,
        command,
        ..
    } = request.into_iter().next().expect("restart request")
    else {
        panic!("expected Resource Command request");
    };

    let refresh = refresh_request(app.update(AppEvent::ResourceCommandCompleted {
        request_id,
        provider_id,
        resource_id,
        resource_name,
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

#[test]
fn failed_resource_command_identifies_provider_resource_and_attempted_command() {
    let mut app = App::new();
    let initial = refresh_request(app.update(AppEvent::ProviderDiscovered(docker_discovery())));
    app.update(refresh_completed(
        initial,
        Ok(snapshot(&[("container-a", "api", "nginx:1.27")])),
    ));
    let request = app.invoke(Command::Resource(ResourceCommand::Restart));
    let ProviderRequest::ExecuteResourceCommand {
        request_id,
        provider_id,
        resource_id,
        resource_name,
        command,
        ..
    } = request.into_iter().next().expect("restart request")
    else {
        panic!("expected Resource Command request");
    };

    let follow_up = app.update(AppEvent::ResourceCommandCompleted {
        request_id,
        provider_id,
        resource_id,
        resource_name,
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
    let initial = refresh_request(app.update(AppEvent::ProviderDiscovered(docker_discovery())));
    app.update(refresh_completed(
        initial,
        Ok(snapshot(&[
            ("container-a", "api", "nginx:1.27"),
            ("container-b", "worker", "alpine:3.21"),
        ])),
    ));
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
fn switching_provider_workspaces_keeps_an_in_flight_resource_command() {
    let mut app = App::new();
    let initial = refresh_request(app.update(AppEvent::ProviderDiscovered(docker_discovery())));
    app.update(refresh_completed(
        initial,
        Ok(snapshot(&[("container-a", "api", "nginx:1.27")])),
    ));
    let restart = command_request(app.invoke(Command::Resource(ResourceCommand::Restart)));
    app.update(AppEvent::ProviderDiscovered(incus_discovery()));
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
fn a_running_resource_command_shows_a_status_identifying_provider_resource_and_command() {
    let mut app = App::new();
    let initial = refresh_request(app.update(AppEvent::ProviderDiscovered(docker_discovery())));
    app.update(refresh_completed(
        initial,
        Ok(snapshot(&[("container-a", "api", "nginx:1.27")])),
    ));

    app.invoke(Command::Resource(ResourceCommand::Restart));

    let screen = render_to_text(app.state(), 160, 24);
    assert!(
        screen.contains("Running Docker restart for api (container-a)"),
        "rendered screen:\n{screen}"
    );
}

#[test]
fn a_successful_resource_command_refreshes_only_its_own_provider_workspace() {
    let mut app = App::new();
    let initial = refresh_request(app.update(AppEvent::ProviderDiscovered(docker_discovery())));
    app.update(refresh_completed(
        initial,
        Ok(snapshot(&[("container-a", "api", "nginx:1.27")])),
    ));
    let restart = command_request(app.invoke(Command::Resource(ResourceCommand::Restart)));
    app.update(AppEvent::ProviderDiscovered(incus_discovery()));
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
    let initial = refresh_request(app.update(AppEvent::ProviderDiscovered(docker_discovery())));
    app.update(refresh_completed(
        initial,
        Ok(snapshot(&[("container-a", "api", "nginx:1.27")])),
    ));
    let restart = command_request(app.invoke(Command::Resource(ResourceCommand::Restart)));
    app.update(AppEvent::ProviderDiscovered(incus_discovery()));
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
    let initial = refresh_request(app.update(AppEvent::ProviderDiscovered(docker_discovery())));
    app.update(refresh_completed(
        initial,
        Ok(snapshot(&[("container-a", "api", "nginx:1.27")])),
    ));
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
    let initial = refresh_request(app.update(AppEvent::ProviderDiscovered(docker_discovery())));
    app.update(refresh_completed(
        initial,
        Ok(snapshot(&[("container-a", "api", "nginx:1.27")])),
    ));
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
    let initial = refresh_request(app.update(AppEvent::ProviderDiscovered(docker_discovery())));
    app.update(refresh_completed(
        initial,
        Ok(snapshot(&[("container-a", "api", "nginx:1.27")])),
    ));
    let restart = command_request(app.invoke(Command::Resource(ResourceCommand::Restart)));
    assert!(
        render_to_text(app.state(), 160, 24).contains("Running Docker restart for api"),
        "the dispatched Resource Command is visible while it runs"
    );

    let refresh = refresh_request(app.update(command_completed(restart, Ok(()))));
    app.update(refresh_completed(
        refresh,
        Ok(snapshot(&[("container-a", "api", "nginx:1.28")])),
    ));

    let screen = render_to_text(app.state(), 160, 24);
    assert!(
        !screen.contains("Running Docker restart for api"),
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
    let initial = refresh_request(app.update(AppEvent::ProviderDiscovered(docker_discovery())));
    app.update(refresh_completed(
        initial,
        Ok(snapshot(&[("container-a", "api", "nginx:1.27")])),
    ));
    let restart = command_request(app.invoke(Command::Resource(ResourceCommand::Restart)));
    let stale_docker = refresh_request(app.invoke(Command::Refresh));
    app.update(AppEvent::ProviderDiscovered(incus_discovery()));
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
    assert!(current_screen.contains("Running Docker restart for api (container-a)"));

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
    let initial = refresh_request(app.update(AppEvent::ProviderDiscovered(docker_discovery())));
    app.update(refresh_completed(
        initial,
        Ok(snapshot(&[("container-a", "api", "nginx:1.27")])),
    ));

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
    let initial = refresh_request(paused.update(AppEvent::ProviderDiscovered(docker_discovery())));
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
    let initial = refresh_request(running.update(AppEvent::ProviderDiscovered(docker_discovery())));
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

#[test]
fn question_mark_closes_the_help_overlay_when_it_is_already_open() {
    let mut app = App::new();
    let initial = refresh_request(app.update(AppEvent::ProviderDiscovered(docker_discovery())));
    app.update(refresh_completed(
        initial,
        Ok(snapshot(&[("container-a", "api", "nginx:1.27")])),
    ));

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

#[test]
fn unavailable_resource_command_is_disabled_in_help_and_does_not_dispatch() {
    let mut app = App::new();
    let initial = refresh_request(app.update(AppEvent::ProviderDiscovered(docker_discovery())));
    app.update(refresh_completed(
        initial,
        Ok(snapshot(&[("container-a", "api", "nginx:1.27")])),
    ));

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
    let initial = refresh_request(app.update(AppEvent::ProviderDiscovered(docker_discovery())));
    app.update(refresh_completed(
        initial,
        Ok(snapshot(&[("container-a", "api", "nginx:1.27")])),
    ));

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
            resource_id,
            resource_name,
            command: ResourceCommand::Delete,
            ..
        }] if provider_id == &ProviderId::new("docker")
            && resource_id == &ResourceId::new("container-a")
            && resource_name == "api"
    ));
}

#[test]
fn confirming_a_running_resource_warns_it_is_stopped_and_dispatches_its_state() {
    let mut app = App::new();
    let initial = refresh_request(app.update(AppEvent::ProviderDiscovered(docker_discovery())));
    app.update(refresh_completed(
        initial,
        Ok(snapshot(&[("container-a", "api", "nginx:1.27")])),
    ));

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
            state: ResourceState::Running,
            ..
        }]
    ));
}

/// A paused Resource is not running, so the prompt must not claim it is — but
/// removing it still stops it, and the deletion still has to force.
#[test]
fn confirming_a_paused_resource_warns_it_is_stopped_and_dispatches_its_state() {
    let mut app = App::new();
    let initial = refresh_request(app.update(AppEvent::ProviderDiscovered(docker_discovery())));
    app.update(refresh_completed(
        initial,
        Ok(paused_snapshot(&[("container-a", "api", "nginx:1.27")])),
    ));

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
            state: ResourceState::Paused,
            ..
        }]
    ));
}

#[test]
fn confirming_a_stopped_resource_promises_no_stop_and_dispatches_its_state() {
    let mut app = App::new();
    let initial = refresh_request(app.update(AppEvent::ProviderDiscovered(docker_discovery())));
    app.update(refresh_completed(
        initial,
        Ok(stopped_snapshot(&[("container-a", "api", "nginx:1.27")])),
    ));

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
            state: ResourceState::Stopped,
            ..
        }]
    ));
}

#[test]
fn n_cancels_delete_confirmation_without_dispatching() {
    let mut app = App::new();
    let initial = refresh_request(app.update(AppEvent::ProviderDiscovered(docker_discovery())));
    app.update(refresh_completed(
        initial,
        Ok(snapshot(&[("container-a", "api", "nginx:1.27")])),
    ));

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
    let initial = refresh_request(app.update(AppEvent::ProviderDiscovered(docker_discovery())));
    app.update(refresh_completed(
        initial,
        Ok(snapshot(&[("container-a", "api", "nginx:1.27")])),
    ));

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
fn providers_render_in_one_row_above_the_full_width_workspace() {
    let mut app = App::new();
    let initial = refresh_request(app.update(AppEvent::ProviderDiscovered(docker_discovery())));
    app.update(refresh_completed(
        initial,
        Ok(snapshot(&[("container-a", "api", "nginx:1.27")])),
    ));

    let screen = render_to_text(app.state(), 100, 24);
    let mut lines = screen.lines();
    assert!(
        lines
            .next()
            .expect("provider row")
            .starts_with("[1] Providers  [ Docker ]")
    );
    assert!(
        lines
            .next()
            .expect("workspace row")
            .starts_with("┌ Docker ")
    );
}

#[test]
fn numbered_panels_render_their_navigation_shortcuts() {
    let mut app = App::new();
    let initial = refresh_request(app.update(AppEvent::ProviderDiscovered(docker_discovery())));
    app.update(refresh_completed(
        initial,
        Ok(snapshot(&[("container-a", "api", "nginx:1.27")])),
    ));

    let screen = render_to_text(app.state(), 100, 24);
    assert!(screen.starts_with("[1] Providers"));
    assert!(screen.contains("[2] Containers"));
}

#[test]
fn resource_panel_keeps_its_navigation_shortcut_while_loading_or_unavailable() {
    let mut loading_app = App::new();
    loading_app.update(AppEvent::ProviderDiscovered(docker_discovery()));
    assert!(render_to_text(loading_app.state(), 100, 24).contains("[2] Resources"));

    let mut unavailable = docker_discovery();
    unavailable.error = Some(WorkspaceError::new("Docker is unavailable"));
    let mut error_app = App::new();
    error_app.update(AppEvent::ProviderDiscovered(unavailable));
    assert!(render_to_text(error_app.state(), 100, 24).contains("[2] Error"));
}

#[test]
fn each_resource_status_is_coloured_by_its_resource_state() {
    for (state, status, colour) in [
        (ResourceState::Running, "running", Color::Green),
        (ResourceState::Stopped, "exited", Color::DarkGray),
        (ResourceState::Paused, "paused", Color::Yellow),
        (ResourceState::Transitioning, "restarting", Color::Blue),
        (ResourceState::Broken, "dead", Color::Red),
        // An unrecognised Provider status stays neutral rather than borrowing
        // the colour of a state Virtui understands.
        (ResourceState::Unknown, "teleporting", Color::Reset),
    ] {
        let mut app = App::new();
        let initial = refresh_request(app.update(AppEvent::ProviderDiscovered(docker_discovery())));
        app.update(refresh_completed(
            initial,
            Ok(container_snapshot(
                &[("container-a", "api", "nginx:1.27")],
                state,
            )),
        ));

        assert_eq!(
            foreground_of(app.state(), 100, 24, status),
            colour,
            "{state:?} status should be rendered in {colour:?}"
        );
    }
}

#[test]
fn a_resource_name_is_left_uncoloured_by_its_resource_state() {
    let mut app = App::new();
    let initial = refresh_request(app.update(AppEvent::ProviderDiscovered(docker_discovery())));
    app.update(refresh_completed(
        initial,
        Ok(snapshot(&[("container-a", "api", "nginx:1.27")])),
    ));

    assert_eq!(foreground_of(app.state(), 100, 24, "api"), Color::Reset);
}

#[test]
fn bracket_keys_switch_the_active_workspace() {
    let mut app = App::new();
    let initial = refresh_request(app.update(AppEvent::ProviderDiscovered(docker_discovery())));
    app.update(refresh_completed(
        initial,
        Ok(snapshot(&[("container-a", "api", "nginx:1.27")])),
    ));
    assert!(
        app.update(AppEvent::ProviderDiscovered(fixture_discovery()))
            .is_empty(),
        "inactive workspaces remain idle"
    );

    let (_, requests) = handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char(']'), KeyModifiers::NONE),
    );
    assert_eq!(requests.len(), 1, "new Active Workspace is refreshed");
    assert!(
        render_to_text(app.state(), 100, 24).starts_with("[1] Providers  Docker   [ Fixture ]")
    );

    let (_, requests) = handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('['), KeyModifiers::NONE),
    );
    assert_eq!(requests.len(), 1);
    assert!(
        render_to_text(app.state(), 100, 24).starts_with("[1] Providers  [ Docker ]   Fixture")
    );
}

#[test]
fn numbered_provider_panel_activates_incus_and_requests_its_refresh() {
    let mut app = App::new();
    let initial = refresh_request(app.update(AppEvent::ProviderDiscovered(docker_discovery())));
    app.update(refresh_completed(
        initial,
        Ok(snapshot(&[("container-a", "api", "nginx:1.27")])),
    ));
    assert!(
        app.update(AppEvent::ProviderDiscovered(incus_discovery()))
            .is_empty(),
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
    assert!(render_to_text(app.state(), 100, 24).starts_with("[1] Providers  Docker   [ Incus ]"));
}

#[test]
fn late_docker_result_cannot_replace_the_active_incus_workspace() {
    let mut app = App::new();
    let stale_docker =
        refresh_request(app.update(AppEvent::ProviderDiscovered(docker_discovery())));
    app.update(AppEvent::ProviderDiscovered(incus_discovery()));
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
    assert!(current_screen.contains("[ Incus ]"));
    assert!(current_screen.contains("gateway"));
    assert!(!current_screen.contains("nginx:1.27"));
}

fn snapshot(containers: &[(&str, &str, &str)]) -> WorkspaceSnapshot {
    container_snapshot(containers, ResourceState::Running)
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
    let (status, available_commands) = match state {
        ResourceState::Running => (
            "running",
            vec![
                ResourceCommand::Stop,
                ResourceCommand::Restart,
                ResourceCommand::Delete,
            ],
        ),
        ResourceState::Stopped => (
            "exited",
            vec![ResourceCommand::Start, ResourceCommand::Delete],
        ),
        ResourceState::Paused => (
            "paused",
            vec![ResourceCommand::Resume, ResourceCommand::Delete],
        ),
        ResourceState::Transitioning => ("restarting", vec![ResourceCommand::Delete]),
        ResourceState::Broken => ("dead", vec![ResourceCommand::Delete]),
        ResourceState::Unknown => ("teleporting", vec![ResourceCommand::Delete]),
    };
    WorkspaceSnapshot {
        panels: vec![ResourcePanel {
            title: "Containers".to_owned(),
            resources: containers
                .iter()
                .map(|(id, name, image)| Resource {
                    id: ResourceId((*id).to_owned()),
                    name: (*name).to_owned(),
                    status: Some(status.to_owned()),
                    state,
                    fields: vec![("Image".to_owned(), (*image).to_owned())],
                    available_commands: available_commands.clone(),
                })
                .collect(),
        }],
    }
}

fn incus_snapshot(instances: &[(&str, &str, &str)]) -> WorkspaceSnapshot {
    WorkspaceSnapshot {
        panels: vec![ResourcePanel {
            title: "Instances".to_owned(),
            resources: instances
                .iter()
                .map(|(id, name, status)| {
                    let running = status.eq_ignore_ascii_case("running");
                    Resource {
                        id: ResourceId((*id).to_owned()),
                        name: (*name).to_owned(),
                        status: Some((*status).to_owned()),
                        state: if running {
                            ResourceState::Running
                        } else {
                            ResourceState::Stopped
                        },
                        fields: vec![("Type".to_owned(), "container".to_owned())],
                        available_commands: if running {
                            vec![
                                ResourceCommand::Stop,
                                ResourceCommand::Restart,
                                ResourceCommand::Delete,
                            ]
                        } else {
                            vec![ResourceCommand::Start, ResourceCommand::Delete]
                        },
                    }
                })
                .collect(),
        }],
    }
}

fn refresh_request(requests: Vec<ProviderRequest>) -> ProviderRequest {
    requests.into_iter().next().expect("refresh request")
}

fn command_request(requests: Vec<ProviderRequest>) -> ProviderRequest {
    requests
        .into_iter()
        .next()
        .expect("Resource Command request")
}

fn command_completed(request: ProviderRequest, result: Result<(), WorkspaceError>) -> AppEvent {
    match request {
        ProviderRequest::ExecuteResourceCommand {
            request_id,
            provider_id,
            resource_id,
            resource_name,
            command,
            ..
        } => AppEvent::ResourceCommandCompleted {
            request_id,
            provider_id,
            resource_id,
            resource_name,
            command,
            result,
        },
        ProviderRequest::RefreshWorkspace { .. } => panic!("expected Resource Command request"),
    }
}

fn refresh_completed(
    request: ProviderRequest,
    result: Result<WorkspaceSnapshot, WorkspaceError>,
) -> AppEvent {
    match request {
        ProviderRequest::RefreshWorkspace {
            request_id,
            provider_id,
        } => AppEvent::RefreshCompleted {
            request_id,
            provider_id,
            result,
        },
        ProviderRequest::ExecuteResourceCommand { .. } => panic!("expected refresh request"),
    }
}

#[test]
fn refresh_preserves_container_selection_by_stable_identity() {
    let mut app = App::new();
    let initial = refresh_request(app.update(AppEvent::ProviderDiscovered(docker_discovery())));
    app.update(refresh_completed(
        initial,
        Ok(snapshot(&[
            ("container-a", "api", "nginx:1.27"),
            ("container-b", "worker", "alpine:3.21"),
        ])),
    ));
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
    let initial = refresh_request(app.update(AppEvent::ProviderDiscovered(docker_discovery())));
    app.update(refresh_completed(
        initial,
        Ok(snapshot(&[
            ("container-a", "api", "nginx:1.27"),
            ("container-b", "worker", "alpine:3.21"),
        ])),
    ));

    app.invoke(Command::SelectNext);
    let worker = render_to_text(app.state(), 100, 24);
    assert!(worker.contains("Image: alpine:3.21"));

    app.invoke(Command::SelectPrevious);
    let api = render_to_text(app.state(), 100, 24);
    assert!(api.contains("Image: nginx:1.27"));
}

#[test]
fn automatic_and_manual_refreshes_do_not_overlap() {
    let mut app = App::new();
    let initial = refresh_request(app.update(AppEvent::ProviderDiscovered(docker_discovery())));

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
    let initial = refresh_request(app.update(AppEvent::ProviderDiscovered(docker_discovery())));
    app.update(refresh_completed(
        initial,
        Ok(snapshot(&[
            ("c0", "r0", "i0"),
            ("c1", "r1", "i1"),
            ("c2", "r2", "i2"),
            ("c3", "r3", "i3"),
            ("c4", "r4", "i4"),
            ("c5", "r5", "i5"),
            ("c6", "r6", "i6"),
        ])),
    ));

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
    let initial = refresh_request(app.update(AppEvent::ProviderDiscovered(docker_discovery())));
    app.update(refresh_completed(
        initial,
        Ok(snapshot(&[
            ("c0", "r0", "i0"),
            ("c1", "r1", "i1"),
            ("c2", "r2", "i2"),
            ("c3", "r3", "i3"),
            ("c4", "r4", "i4"),
            ("c5", "r5", "i5"),
            ("c6", "r6", "i6"),
        ])),
    ));
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

/// `ctrl+c` is reserved by the registry, so it quits even from inside a modal
/// that swallows every other key.
#[test]
fn ctrl_c_quits_even_while_a_confirmation_modal_is_open() {
    let mut app = App::new();
    let initial = refresh_request(app.update(AppEvent::ProviderDiscovered(docker_discovery())));
    app.update(refresh_completed(
        initial,
        Ok(snapshot(&[("container-a", "api", "nginx:1.27")])),
    ));
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
    let initial = refresh_request(app.update(AppEvent::ProviderDiscovered(docker_discovery())));
    app.update(refresh_completed(
        initial,
        Ok(snapshot(&[("container-a", "api", "nginx:1.27")])),
    ));
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
    let initial = refresh_request(app.update(AppEvent::ProviderDiscovered(docker_discovery())));
    app.update(refresh_completed(
        initial,
        Ok(snapshot(&[("container-a", "api", "nginx:1.27")])),
    ));

    let (control, requests) = handle_key(&mut app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

    assert_eq!(control, ShellControl::Continue);
    assert!(requests.is_empty());
}

#[test]
fn an_overridden_focus_key_renders_its_effective_hint() {
    let registry =
        CommandRegistry::effective(&[("focus.providers".to_owned(), vec!["9".to_owned()])])
            .expect("a valid override");
    let mut app = App::with_registry(registry);
    let initial = refresh_request(app.update(AppEvent::ProviderDiscovered(docker_discovery())));
    app.update(refresh_completed(
        initial,
        Ok(snapshot(&[("container-a", "api", "nginx:1.27")])),
    ));

    let screen = render_to_text(app.state(), 100, 24);
    assert!(
        screen.starts_with("[9] Providers"),
        "the panel hint follows the effective binding:\n{screen}"
    );
    assert!(
        !screen.contains("[1] Providers"),
        "the replaced default hint is gone:\n{screen}"
    );
}

#[test]
fn an_unbound_focus_command_omits_its_inline_hint() {
    let registry =
        CommandRegistry::effective(&[("focus.resources".to_owned(), vec![])]).expect("unbinding");
    let mut app = App::with_registry(registry);
    let initial = refresh_request(app.update(AppEvent::ProviderDiscovered(docker_discovery())));
    app.update(refresh_completed(
        initial,
        Ok(snapshot(&[("container-a", "api", "nginx:1.27")])),
    ));

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
        ("resource.restart".to_owned(), vec!["x".to_owned()]),
        ("focus.providers".to_owned(), vec!["9".to_owned()]),
    ])
    .expect("a valid override set");
    let mut app = App::with_registry(registry);
    let initial = refresh_request(app.update(AppEvent::ProviderDiscovered(docker_discovery())));
    app.update(refresh_completed(
        initial,
        Ok(snapshot(&[("container-a", "api", "nginx:1.27")])),
    ));

    // The inline hint follows the override.
    let screen = render_to_text(app.state(), 100, 24);
    assert!(screen.starts_with("[9] Providers"), "rendered:\n{screen}");

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
