use std::{future::Future, pin::Pin, sync::Arc};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tokio::sync::{Notify, mpsc};
use vertui::{
    app::{App, AppEvent},
    cli::{CliError, CliOutput, CliRunner, CommandSpec},
    docker::DockerWorkspace,
    provider::{
        ProviderAction, ProviderDiscovery, ProviderId, ProviderRequest, Resource, ResourceId,
        ResourcePanel, WorkspaceSnapshot,
    },
    runtime::{ProviderRuntime, RefreshTimer, ShellControl, handle_key},
    ui::render_to_text,
};

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

impl CliRunner for MissingCli {
    fn run<'a>(
        &'a self,
        _command: CommandSpec,
    ) -> Pin<Box<dyn Future<Output = Result<CliOutput, CliError>> + Send + 'a>> {
        Box::pin(async { Err(CliError::NotFound) })
    }
}

impl CliRunner for DelayedCli {
    fn run<'a>(
        &'a self,
        command: CommandSpec,
    ) -> Pin<Box<dyn Future<Output = Result<CliOutput, CliError>> + Send + 'a>> {
        Box::pin(async move {
            assert_eq!(
                command,
                CommandSpec::new(
                    "docker",
                    [
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
            Ok(CliOutput {
                success: true,
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

#[tokio::test(start_paused = true)]
async fn active_workspace_refresh_is_due_every_two_seconds() {
    use std::time::Duration;

    let mut app = App::new();
    let initial = refresh_action(app.update(AppEvent::ProviderDiscovered(docker_discovery())));
    app.update(AppEvent::RefreshCompleted {
        request: initial,
        result: Ok(snapshot(&[("container-a", "api", "nginx:1.27")])),
    });
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
    let initial = refresh_action(app.update(AppEvent::ProviderDiscovered(docker_discovery())));
    app.update(AppEvent::RefreshCompleted {
        request: initial,
        result: Ok(snapshot(&[
            ("container-a", "api", "nginx:1.27"),
            ("container-b", "worker", "alpine:3.21"),
        ])),
    });
    let refresh = refresh_action(app.update(AppEvent::ManualRefresh));
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let runtime = ProviderRuntime::new(
        [Arc::new(DockerWorkspace::new()) as Arc<dyn vertui::provider::ProviderWorkspace>],
        Arc::new(DelayedCli {
            started: Arc::clone(&started),
            release: Arc::clone(&release),
        }),
    );
    let (events, mut completions) = mpsc::unbounded_channel();

    runtime.dispatch(ProviderAction::RefreshWorkspace(refresh), events);
    started.notified().await;
    app.update(AppEvent::SelectNextResource);
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
        [Arc::new(DockerWorkspace::new()) as Arc<dyn vertui::provider::ProviderWorkspace>],
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
    let initial = refresh_action(app.update(AppEvent::ProviderDiscovered(docker_discovery())));
    app.update(AppEvent::RefreshCompleted {
        request: initial,
        result: Ok(snapshot(&[
            ("container-a", "api", "nginx:1.27"),
            ("container-b", "worker", "alpine:3.21"),
        ])),
    });

    let (control, actions) = handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
    );
    assert_eq!(control, ShellControl::Continue);
    assert!(actions.is_empty());
    assert!(render_to_text(app.state(), 100, 24).contains("Image: alpine:3.21"));

    let (_, actions) = handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE),
    );
    assert_eq!(actions.len(), 1);

    let (control, _) = handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
    );
    assert_eq!(control, ShellControl::Quit);
}

#[test]
fn providers_render_in_one_row_above_the_full_width_workspace() {
    let mut app = App::new();
    let initial = refresh_action(app.update(AppEvent::ProviderDiscovered(docker_discovery())));
    app.update(AppEvent::RefreshCompleted {
        request: initial,
        result: Ok(snapshot(&[("container-a", "api", "nginx:1.27")])),
    });

    let screen = render_to_text(app.state(), 100, 24);
    let mut lines = screen.lines();
    assert!(
        lines
            .next()
            .expect("provider row")
            .starts_with("Providers  [ Docker ]")
    );
    assert!(
        lines
            .next()
            .expect("workspace row")
            .starts_with("┌ Docker ")
    );
}

#[test]
fn bracket_keys_switch_the_active_workspace() {
    let mut app = App::new();
    let initial = refresh_action(app.update(AppEvent::ProviderDiscovered(docker_discovery())));
    app.update(AppEvent::RefreshCompleted {
        request: initial,
        result: Ok(snapshot(&[("container-a", "api", "nginx:1.27")])),
    });
    assert!(
        app.update(AppEvent::ProviderDiscovered(fixture_discovery()))
            .is_empty(),
        "inactive workspaces remain idle"
    );

    let (_, actions) = handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char(']'), KeyModifiers::NONE),
    );
    assert_eq!(actions.len(), 1, "new Active Workspace is refreshed");
    assert!(render_to_text(app.state(), 100, 24).starts_with("Providers  Docker   [ Fixture ]"));

    let (_, actions) = handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('['), KeyModifiers::NONE),
    );
    assert_eq!(actions.len(), 1);
    assert!(render_to_text(app.state(), 100, 24).starts_with("Providers  [ Docker ]   Fixture"));
}

fn snapshot(containers: &[(&str, &str, &str)]) -> WorkspaceSnapshot {
    WorkspaceSnapshot {
        panels: vec![ResourcePanel {
            title: "Containers".to_owned(),
            resources: containers
                .iter()
                .map(|(id, name, image)| Resource {
                    id: ResourceId((*id).to_owned()),
                    name: (*name).to_owned(),
                    status: Some("running".to_owned()),
                    fields: vec![("Image".to_owned(), (*image).to_owned())],
                })
                .collect(),
        }],
    }
}

fn refresh_action(actions: Vec<ProviderAction>) -> ProviderRequest {
    actions
        .into_iter()
        .map(|action| match action {
            ProviderAction::RefreshWorkspace(request) => request,
        })
        .next()
        .expect("refresh action")
}

#[test]
fn refresh_preserves_container_selection_by_stable_identity() {
    let mut app = App::new();
    let initial = refresh_action(app.update(AppEvent::ProviderDiscovered(docker_discovery())));
    app.update(AppEvent::RefreshCompleted {
        request: initial,
        result: Ok(snapshot(&[
            ("container-a", "api", "nginx:1.27"),
            ("container-b", "worker", "alpine:3.21"),
        ])),
    });
    app.update(AppEvent::SelectNextResource);
    let refresh = refresh_action(app.update(AppEvent::ManualRefresh));

    app.update(AppEvent::RefreshCompleted {
        request: refresh,
        result: Ok(snapshot(&[
            ("container-b", "worker", "alpine:3.21"),
            ("container-c", "scheduler", "debian:bookworm"),
        ])),
    });

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
    let initial = refresh_action(app.update(AppEvent::ProviderDiscovered(docker_discovery())));
    app.update(AppEvent::RefreshCompleted {
        request: initial,
        result: Ok(snapshot(&[
            ("container-a", "api", "nginx:1.27"),
            ("container-b", "worker", "alpine:3.21"),
        ])),
    });

    app.update(AppEvent::SelectNextResource);
    let worker = render_to_text(app.state(), 100, 24);
    assert!(worker.contains("Image: alpine:3.21"));

    app.update(AppEvent::SelectPreviousResource);
    let api = render_to_text(app.state(), 100, 24);
    assert!(api.contains("Image: nginx:1.27"));
}

#[test]
fn automatic_and_manual_refreshes_do_not_overlap() {
    let mut app = App::new();
    let initial = refresh_action(app.update(AppEvent::ProviderDiscovered(docker_discovery())));

    assert!(app.update(AppEvent::RefreshTimerElapsed).is_empty());
    assert!(app.update(AppEvent::ManualRefresh).is_empty());

    app.update(AppEvent::RefreshCompleted {
        request: initial,
        result: Ok(snapshot(&[("container-a", "api", "nginx:1.27")])),
    });
    let automatic = refresh_action(app.update(AppEvent::RefreshTimerElapsed));
    assert!(app.update(AppEvent::RefreshTimerElapsed).is_empty());
    assert!(app.update(AppEvent::ManualRefresh).is_empty());

    app.update(AppEvent::RefreshCompleted {
        request: automatic,
        result: Ok(snapshot(&[("container-a", "api", "nginx:1.27")])),
    });
    assert_eq!(app.update(AppEvent::ManualRefresh).len(), 1);
}
