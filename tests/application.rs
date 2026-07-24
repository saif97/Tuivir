use std::{future::Future, pin::Pin, sync::Arc};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tokio::sync::{Notify, mpsc};
use vertui::{
    app::{App, AppEvent},
    cli::{CliError, CliOutput, CliRunner, CommandSpec},
    docker::DockerWorkspace,
    provider::{
        ProviderAction, ProviderDiscovery, ProviderId, Resource, ResourceId, ResourcePanel,
        WorkspaceError, WorkspaceSnapshot,
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
    let initial = refresh_action(app.update(AppEvent::ProviderDiscovered(docker_discovery())));
    app.update(refresh_completed(
        initial,
        Ok(snapshot(&[
            ("container-a", "api", "nginx:1.27"),
            ("container-b", "worker", "alpine:3.21"),
        ])),
    ));
    let refresh = refresh_action(app.update(AppEvent::ManualRefresh));
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let runtime = ProviderRuntime::new(
        vec![Arc::new(DockerWorkspace) as Arc<dyn vertui::provider::ProviderWorkspace>],
        Arc::new(DelayedCli {
            started: Arc::clone(&started),
            release: Arc::clone(&release),
        }),
    );
    let (events, mut completions) = mpsc::unbounded_channel();

    runtime.dispatch(refresh, events);
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
        vec![Arc::new(DockerWorkspace) as Arc<dyn vertui::provider::ProviderWorkspace>],
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
    app.update(refresh_completed(
        initial,
        Ok(snapshot(&[
            ("container-a", "api", "nginx:1.27"),
            ("container-b", "worker", "alpine:3.21"),
        ])),
    ));

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
    let initial = refresh_action(app.update(AppEvent::ProviderDiscovered(docker_discovery())));
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
fn bracket_keys_switch_the_active_workspace() {
    let mut app = App::new();
    let initial = refresh_action(app.update(AppEvent::ProviderDiscovered(docker_discovery())));
    app.update(refresh_completed(
        initial,
        Ok(snapshot(&[("container-a", "api", "nginx:1.27")])),
    ));
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
    assert!(
        render_to_text(app.state(), 100, 24).starts_with("[1] Providers  Docker   [ Fixture ]")
    );

    let (_, actions) = handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('['), KeyModifiers::NONE),
    );
    assert_eq!(actions.len(), 1);
    assert!(
        render_to_text(app.state(), 100, 24).starts_with("[1] Providers  [ Docker ]   Fixture")
    );
}

#[test]
fn numbered_provider_panel_activates_incus_and_requests_its_refresh() {
    let mut app = App::new();
    let initial = refresh_action(app.update(AppEvent::ProviderDiscovered(docker_discovery())));
    app.update(refresh_completed(
        initial,
        Ok(snapshot(&[("container-a", "api", "nginx:1.27")])),
    ));
    assert!(
        app.update(AppEvent::ProviderDiscovered(incus_discovery()))
            .is_empty(),
        "inactive workspaces remain idle"
    );

    let (_, focus_actions) = handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE),
    );
    assert!(focus_actions.is_empty());
    let (_, activation_actions) = handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
    );

    let action = activation_actions
        .into_iter()
        .next()
        .expect("activating Incus requests an immediate refresh");
    assert!(matches!(
        action,
        ProviderAction::RefreshWorkspace {
            provider_id,
            ..
        } if provider_id == ProviderId::new("incus")
    ));
    assert!(render_to_text(app.state(), 100, 24).starts_with("[1] Providers  Docker   [ Incus ]"));
}

#[test]
fn late_docker_result_cannot_replace_the_active_incus_workspace() {
    let mut app = App::new();
    let stale_docker = refresh_action(app.update(AppEvent::ProviderDiscovered(docker_discovery())));
    app.update(AppEvent::ProviderDiscovered(incus_discovery()));
    app.update(AppEvent::FocusProviders);
    let incus_request = refresh_action(app.update(AppEvent::SelectNextProvider));
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

fn incus_snapshot(instances: &[(&str, &str, &str)]) -> WorkspaceSnapshot {
    WorkspaceSnapshot {
        panels: vec![ResourcePanel {
            title: "Instances".to_owned(),
            resources: instances
                .iter()
                .map(|(id, name, status)| Resource {
                    id: ResourceId((*id).to_owned()),
                    name: (*name).to_owned(),
                    status: Some((*status).to_owned()),
                    fields: vec![("Type".to_owned(), "container".to_owned())],
                })
                .collect(),
        }],
    }
}

fn refresh_action(actions: Vec<ProviderAction>) -> ProviderAction {
    actions.into_iter().next().expect("refresh action")
}

fn refresh_completed(
    action: ProviderAction,
    result: Result<WorkspaceSnapshot, WorkspaceError>,
) -> AppEvent {
    match action {
        ProviderAction::RefreshWorkspace {
            request_id,
            provider_id,
        } => AppEvent::RefreshCompleted {
            request_id,
            provider_id,
            result,
        },
    }
}

#[test]
fn refresh_preserves_container_selection_by_stable_identity() {
    let mut app = App::new();
    let initial = refresh_action(app.update(AppEvent::ProviderDiscovered(docker_discovery())));
    app.update(refresh_completed(
        initial,
        Ok(snapshot(&[
            ("container-a", "api", "nginx:1.27"),
            ("container-b", "worker", "alpine:3.21"),
        ])),
    ));
    app.update(AppEvent::SelectNextResource);
    let refresh = refresh_action(app.update(AppEvent::ManualRefresh));

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
    let initial = refresh_action(app.update(AppEvent::ProviderDiscovered(docker_discovery())));
    app.update(refresh_completed(
        initial,
        Ok(snapshot(&[
            ("container-a", "api", "nginx:1.27"),
            ("container-b", "worker", "alpine:3.21"),
        ])),
    ));

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

    app.update(refresh_completed(
        initial,
        Ok(snapshot(&[("container-a", "api", "nginx:1.27")])),
    ));
    let automatic = refresh_action(app.update(AppEvent::RefreshTimerElapsed));
    assert!(app.update(AppEvent::RefreshTimerElapsed).is_empty());
    assert!(app.update(AppEvent::ManualRefresh).is_empty());

    app.update(refresh_completed(
        automatic,
        Ok(snapshot(&[("container-a", "api", "nginx:1.27")])),
    ));
    assert_eq!(app.update(AppEvent::ManualRefresh).len(), 1);
}
