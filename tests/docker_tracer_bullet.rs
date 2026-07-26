use std::{collections::VecDeque, future::Future, pin::Pin, sync::Mutex};

use virtui::{
    app::{App, AppEvent},
    cli::{CliRunner, ProcessError, ProcessFailure, ProcessOutput, ProcessSpec},
    docker::DockerWorkspace,
    provider::{
        DetailViewId, ProviderRequest, ProviderWorkspace, ResourceCommand, ResourceId,
        ResourceState, WorkspaceError, WorkspaceSnapshot,
    },
    ui::render_to_text,
};

struct FixtureCli {
    responses: Mutex<VecDeque<(ProcessSpec, Result<ProcessOutput, ProcessError>)>>,
}

impl FixtureCli {
    fn new(
        responses: impl IntoIterator<Item = (ProcessSpec, Result<ProcessOutput, ProcessError>)>,
    ) -> Self {
        Self {
            responses: Mutex::new(responses.into_iter().collect()),
        }
    }
}

impl CliRunner for FixtureCli {
    fn run<'a>(
        &'a self,
        command: ProcessSpec,
    ) -> Pin<Box<dyn Future<Output = Result<ProcessOutput, ProcessError>> + Send + 'a>> {
        Box::pin(async move {
            let (expected, response) = self
                .responses
                .lock()
                .expect("fixture queue lock")
                .pop_front()
                .expect("unexpected CLI command");
            assert_eq!(command, expected);
            response
        })
    }
}

fn success(stdout: &str) -> Result<ProcessOutput, ProcessError> {
    Ok(ProcessOutput {
        stdout: stdout.to_owned(),
        stderr: String::new(),
    })
}

fn failure(stderr: &str) -> Result<ProcessOutput, ProcessError> {
    Err(ProcessError::Exited(ProcessFailure {
        exit_code: Some(1),
        stdout: String::new(),
        stderr: stderr.to_owned(),
    }))
}

fn failure_on_stdout(stdout: &str) -> Result<ProcessOutput, ProcessError> {
    Err(ProcessError::Exited(ProcessFailure {
        exit_code: Some(1),
        stdout: stdout.to_owned(),
        stderr: String::new(),
    }))
}

fn silent_failure() -> Result<ProcessOutput, ProcessError> {
    Err(ProcessError::Exited(ProcessFailure {
        exit_code: Some(125),
        stdout: String::new(),
        stderr: String::new(),
    }))
}

fn container_ls() -> ProcessSpec {
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
        other => panic!("expected refresh request, got {other:?}"),
    }
}

#[tokio::test]
async fn docker_restart_generates_the_expected_cli_request() {
    let cli = FixtureCli::new([(
        ProcessSpec::new("docker", &["container", "restart", "container-a"]),
        success("container-a\n"),
    )]);

    DockerWorkspace
        .execute_command(
            &cli,
            &ResourceId::new("container-a"),
            ResourceCommand::Restart,
            ResourceState::Running,
        )
        .await
        .expect("Docker restart succeeds");
}

#[tokio::test]
async fn docker_start_generates_the_expected_cli_request() {
    let cli = FixtureCli::new([(
        ProcessSpec::new("docker", &["container", "start", "container-a"]),
        success("container-a\n"),
    )]);

    DockerWorkspace
        .execute_command(
            &cli,
            &ResourceId::new("container-a"),
            ResourceCommand::Start,
            ResourceState::Stopped,
        )
        .await
        .expect("Docker start succeeds");
}

#[tokio::test]
async fn docker_stop_generates_the_expected_cli_request() {
    let cli = FixtureCli::new([(
        ProcessSpec::new("docker", &["container", "stop", "container-a"]),
        success("container-a\n"),
    )]);

    DockerWorkspace
        .execute_command(
            &cli,
            &ResourceId::new("container-a"),
            ResourceCommand::Stop,
            ResourceState::Running,
        )
        .await
        .expect("Docker stop succeeds");
}

#[tokio::test]
async fn docker_resume_generates_the_expected_cli_request() {
    let cli = FixtureCli::new([(
        ProcessSpec::new("docker", &["container", "unpause", "container-a"]),
        success("container-a\n"),
    )]);

    DockerWorkspace
        .execute_command(
            &cli,
            &ResourceId::new("container-a"),
            ResourceCommand::Resume,
            ResourceState::Paused,
        )
        .await
        .expect("Docker resume succeeds");
}

#[tokio::test]
async fn deleting_a_stopped_container_generates_the_expected_cli_request() {
    let cli = FixtureCli::new([(
        ProcessSpec::new("docker", &["container", "rm", "container-a"]),
        success("container-a\n"),
    )]);

    DockerWorkspace
        .execute_command(
            &cli,
            &ResourceId::new("container-a"),
            ResourceCommand::Delete,
            ResourceState::Stopped,
        )
        .await
        .expect("Docker delete succeeds");
}

#[tokio::test]
async fn deleting_a_running_container_forces_removal_without_a_second_query() {
    // The fixture answers exactly one CLI request and panics on any other, so
    // this also proves the Resource State travels with the request instead of
    // being rediscovered through the Docker CLI.
    let cli = FixtureCli::new([(
        ProcessSpec::new("docker", &["container", "rm", "--force", "container-a"]),
        success("container-a\n"),
    )]);

    DockerWorkspace
        .execute_command(
            &cli,
            &ResourceId::new("container-a"),
            ResourceCommand::Delete,
            ResourceState::Running,
        )
        .await
        .expect("Docker force delete succeeds");
}

/// Docker removes a container without `--force` only from a settled, stopped
/// state. Every other state — verified against the daemon for `paused` — needs
/// the force the user already confirmed.
#[tokio::test]
async fn deleting_a_container_that_is_not_stopped_forces_removal() {
    for state in [
        ResourceState::Paused,
        ResourceState::Transitioning,
        ResourceState::Broken,
        ResourceState::Unknown,
    ] {
        let cli = FixtureCli::new([(
            ProcessSpec::new("docker", &["container", "rm", "--force", "container-a"]),
            success("container-a\n"),
        )]);

        DockerWorkspace
            .execute_command(
                &cli,
                &ResourceId::new("container-a"),
                ResourceCommand::Delete,
                state,
            )
            .await
            .unwrap_or_else(|error| panic!("force delete from {state:?} succeeds: {error:?}"));
    }
}

#[tokio::test]
async fn docker_maps_every_container_state_into_the_shared_vocabulary() {
    let cli = FixtureCli::new([(
        container_ls(),
        success(include_str!("fixtures/docker/mixed-state-containers.jsonl")),
    )]);

    let snapshot = DockerWorkspace
        .refresh(&cli)
        .await
        .expect("fixture lists containers");

    let states = snapshot
        .resources()
        .map(|resource| (resource.id.0.as_str(), resource.state))
        .collect::<Vec<_>>();
    assert_eq!(
        states,
        [
            ("container-running", ResourceState::Running),
            ("container-exited", ResourceState::Stopped),
            ("container-created", ResourceState::Stopped),
            ("container-paused", ResourceState::Paused),
            ("container-restarting", ResourceState::Transitioning),
            ("container-dead", ResourceState::Broken),
            // A state this Docker release never returned still has to land
            // somewhere honest rather than masquerade as stopped.
            ("container-future", ResourceState::Unknown),
        ]
    );
}

/// `docker container unpause` succeeds only against a paused container, so no
/// other state may offer the Command that runs it.
#[tokio::test]
async fn only_a_paused_container_offers_the_resume_command() {
    let cli = FixtureCli::new([(
        container_ls(),
        success(include_str!("fixtures/docker/mixed-state-containers.jsonl")),
    )]);

    let snapshot = DockerWorkspace
        .refresh(&cli)
        .await
        .expect("fixture lists containers");

    let resumable = snapshot
        .resources()
        .filter(|resource| {
            resource
                .available_commands
                .contains(&ResourceCommand::Resume)
        })
        .map(|resource| resource.id.0.as_str())
        .collect::<Vec<_>>();
    assert_eq!(resumable, ["container-paused"]);

    let paused = snapshot
        .resources()
        .find(|resource| resource.id.0 == "container-paused")
        .expect("fixture has a paused container");
    assert_eq!(
        paused.available_commands,
        [ResourceCommand::Resume, ResourceCommand::Delete]
    );
}

/// The Containers panel advertises Docker's own diagnostics, so the shell can
/// offer them without knowing what a container is.
#[tokio::test]
async fn the_containers_panel_declares_dockers_native_detail_views() {
    let cli = FixtureCli::new([(
        container_ls(),
        success(include_str!("fixtures/docker/containers.jsonl")),
    )]);

    let snapshot = DockerWorkspace
        .refresh(&cli)
        .await
        .expect("fixture lists containers");

    let panel = snapshot.panels.first().expect("a Containers panel");
    assert_eq!(
        panel
            .detail_views
            .iter()
            .map(|view| (view.id.0.as_str(), view.title.as_str()))
            .collect::<Vec<_>>(),
        [("logs", "Logs"), ("stats", "Stats"), ("inspect", "Inspect")]
    );
}

/// Each declared view runs exactly one Docker command. The fixture answers one
/// request and panics on any other, so a view that loaded more than the one on
/// screen would fail here.
#[tokio::test]
async fn each_detail_view_runs_its_own_docker_command() {
    for (view, expected) in [
        (
            "logs",
            ProcessSpec::new(
                "docker",
                &["container", "logs", "--tail", "200", "container-a"],
            ),
        ),
        (
            "stats",
            ProcessSpec::new(
                "docker",
                &["container", "stats", "--no-stream", "container-a"],
            ),
        ),
        (
            "inspect",
            ProcessSpec::new("docker", &["container", "inspect", "container-a"]),
        ),
    ] {
        let cli = FixtureCli::new([(expected, success("first line\nsecond line\n"))]);

        let details = DockerWorkspace
            .load_details(
                &cli,
                &ResourceId::new("container-a"),
                &DetailViewId::new(view),
            )
            .await
            .unwrap_or_else(|error| panic!("Docker {view} loads: {error:?}"));

        assert_eq!(details.lines, ["first line", "second line"], "view {view}");
    }
}

/// A container writes its log to whichever stream it chose, so Logs must show
/// both rather than silently drop everything written to stderr.
#[tokio::test]
async fn container_logs_include_what_the_container_wrote_to_stderr() {
    let cli = FixtureCli::new([(
        ProcessSpec::new(
            "docker",
            &["container", "logs", "--tail", "200", "container-a"],
        ),
        Ok(ProcessOutput {
            stdout: "listening on port 80\n".to_owned(),
            stderr: "warning: cache is cold\n".to_owned(),
        }),
    )]);

    let details = DockerWorkspace
        .load_details(
            &cli,
            &ResourceId::new("container-a"),
            &DetailViewId::new("logs"),
        )
        .await
        .expect("Docker logs load");

    assert_eq!(
        details.lines,
        ["listening on port 80", "warning: cache is cold"]
    );
}

#[tokio::test]
async fn a_container_that_has_logged_nothing_loads_empty_details() {
    let cli = FixtureCli::new([(
        ProcessSpec::new(
            "docker",
            &["container", "logs", "--tail", "200", "container-a"],
        ),
        success(""),
    )]);

    let details = DockerWorkspace
        .load_details(
            &cli,
            &ResourceId::new("container-a"),
            &DetailViewId::new("logs"),
        )
        .await
        .expect("no output is not a failure");

    assert!(details.is_empty());
}

#[tokio::test]
async fn a_failed_detail_view_reports_what_docker_wrote_to_stderr() {
    let cli = FixtureCli::new([(
        ProcessSpec::new("docker", &["container", "inspect", "container-a"]),
        failure("Error: No such container: container-a"),
    )]);

    let error = DockerWorkspace
        .load_details(
            &cli,
            &ResourceId::new("container-a"),
            &DetailViewId::new("inspect"),
        )
        .await
        .expect_err("a non-zero exit is never loaded details");

    assert_eq!(error.message, "Error: No such container: container-a");
}

#[tokio::test]
async fn a_silent_command_failure_names_the_operation_and_container() {
    let cli = FixtureCli::new([(
        ProcessSpec::new("docker", &["container", "restart", "container-a"]),
        silent_failure(),
    )]);

    let error = DockerWorkspace
        .execute_command(
            &cli,
            &ResourceId::new("container-a"),
            ResourceCommand::Restart,
            ResourceState::Running,
        )
        .await
        .expect_err("a non-zero exit is never a successful command");

    assert_eq!(
        error.message,
        "Docker could not restart container container-a"
    );
}

#[tokio::test]
async fn a_failed_command_reports_what_docker_wrote_to_stderr() {
    let cli = FixtureCli::new([(
        ProcessSpec::new("docker", &["container", "rm", "container-a"]),
        failure(
            "Error response from daemon: removal of container container-a is already in progress",
        ),
    )]);

    let error = DockerWorkspace
        .execute_command(
            &cli,
            &ResourceId::new("container-a"),
            ResourceCommand::Delete,
            ResourceState::Stopped,
        )
        .await
        .expect_err("a non-zero exit is never a successful command");

    assert_eq!(
        error.message,
        "Error response from daemon: removal of container container-a is already in progress"
    );
}

#[tokio::test]
async fn discovered_docker_workspace_renders_target_environment_and_containers() {
    let cli = FixtureCli::new([
        (
            ProcessSpec::new("docker", &["context", "show"]),
            success("desktop-linux\n"),
        ),
        (
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
            ),
            success(include_str!("fixtures/docker/containers.jsonl")),
        ),
    ]);
    let docker = DockerWorkspace;

    let discovered = docker
        .discover(&cli)
        .await
        .expect("the fixture represents an installed Docker CLI");
    let mut app = App::new();
    let requests = app.update(AppEvent::ProviderDiscovered(discovered));
    let request = requests
        .into_iter()
        .next()
        .expect("discovery requests the first workspace refresh");

    let snapshot = docker.refresh(&cli).await;
    app.update(refresh_completed(request, snapshot));

    let screen = render_to_text(app.state(), 100, 24);
    assert!(screen.contains("Providers"));
    assert!(screen.contains("Docker"));
    assert!(screen.contains("Target: desktop-linux"));
    assert!(screen.contains("Containers"), "rendered screen:\n{screen}");
    assert!(screen.contains("api"));
    assert!(screen.contains("nginx:1.27"));
    assert!(screen.contains("running"));
    assert!(screen.contains("worker"));
    assert!(screen.contains("exited"));
}

#[tokio::test]
async fn reachable_docker_without_containers_renders_a_distinct_empty_state() {
    let cli = FixtureCli::new([
        (
            ProcessSpec::new("docker", &["context", "show"]),
            success("colima\n"),
        ),
        (
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
            ),
            success(""),
        ),
    ]);
    let docker = DockerWorkspace;
    let discovered = docker.discover(&cli).await.expect("Docker is installed");
    let mut app = App::new();
    let request = app
        .update(AppEvent::ProviderDiscovered(discovered))
        .into_iter()
        .next()
        .expect("initial refresh");
    app.update(refresh_completed(request, docker.refresh(&cli).await));

    let screen = render_to_text(app.state(), 100, 24);
    assert!(screen.contains("Target: colima"));
    assert!(screen.contains("No Docker containers found"));
    assert!(!screen.contains("unavailable"));
}

#[tokio::test]
async fn installed_but_unreachable_docker_stays_visible_with_actionable_error() {
    let cli = FixtureCli::new([(
        ProcessSpec::new("docker", &["context", "show"]),
        failure("Cannot connect to the Docker daemon"),
    )]);
    let docker = DockerWorkspace;
    let discovered = docker.discover(&cli).await.expect("Docker is installed");
    let mut app = App::new();

    let requests = app.update(AppEvent::ProviderDiscovered(discovered));

    assert!(
        requests.is_empty(),
        "an unreachable provider is not refreshed"
    );
    let screen = render_to_text(app.state(), 100, 24);
    assert!(screen.contains("Docker provider is unavailable"));
    assert!(screen.contains("Cannot connect to the Docker daemon"));
    assert!(screen.contains("docker context show"));
}

#[tokio::test]
async fn unreachable_docker_without_stderr_reports_what_it_printed_on_stdout() {
    let cli = FixtureCli::new([(
        ProcessSpec::new("docker", &["context", "show"]),
        failure_on_stdout("the context store is unreadable\n"),
    )]);

    let discovered = DockerWorkspace
        .discover(&cli)
        .await
        .expect("Docker is installed");

    let error = discovered.error.expect("the provider exposes its error");
    assert!(
        error.message.contains("the context store is unreadable"),
        "workspace error: {}",
        error.message
    );
    assert!(error.message.contains("docker context show"));
}

#[tokio::test]
async fn docker_that_fails_silently_at_discovery_still_explains_itself() {
    let cli = FixtureCli::new([(
        ProcessSpec::new("docker", &["context", "show"]),
        silent_failure(),
    )]);

    let discovered = DockerWorkspace
        .discover(&cli)
        .await
        .expect("Docker is installed");

    let error = discovered.error.expect("the provider exposes its error");
    assert!(
        error
            .message
            .contains("Docker could not report its current context"),
        "workspace error: {}",
        error.message
    );
    assert_eq!(discovered.target_environment, "unavailable");
}

#[tokio::test]
async fn failed_container_refresh_identifies_docker_command_and_target() {
    let cli = FixtureCli::new([
        (
            ProcessSpec::new("docker", &["context", "show"]),
            success("desktop-linux\n"),
        ),
        (
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
            ),
            failure(include_str!("fixtures/docker/daemon-unreachable.stderr")),
        ),
    ]);
    let docker = DockerWorkspace;
    let discovered = docker.discover(&cli).await.expect("Docker is installed");
    let mut app = App::new();
    let request = app
        .update(AppEvent::ProviderDiscovered(discovered))
        .into_iter()
        .next()
        .expect("initial refresh");
    app.update(refresh_completed(request, docker.refresh(&cli).await));

    let screen = render_to_text(app.state(), 140, 24);
    assert!(screen.contains("Target: desktop-linux"));
    assert!(screen.contains("permission denied connecting to Docker socket"));
    assert!(screen.contains("Run `docker"));
    assert!(screen.contains("container ls --all`"));
}

#[tokio::test]
async fn failed_refresh_without_stderr_reports_what_docker_printed_on_stdout() {
    let cli = FixtureCli::new([(
        container_ls(),
        failure_on_stdout("the Docker daemon is restarting\n"),
    )]);

    let error = DockerWorkspace
        .refresh(&cli)
        .await
        .expect_err("a non-zero exit is never a successful refresh");

    assert!(
        error.message.contains("the Docker daemon is restarting"),
        "workspace error: {}",
        error.message
    );
    assert!(error.message.contains("docker container ls --all"));
}

#[tokio::test]
async fn a_docker_cli_that_cannot_be_started_names_docker_in_the_error() {
    let cli = FixtureCli::new([(
        container_ls(),
        Err(ProcessError::SpawnFailed(
            "Permission denied (os error 13)".to_owned(),
        )),
    )]);

    let error = DockerWorkspace
        .refresh(&cli)
        .await
        .expect_err("a CLI that never started is never a successful refresh");

    assert_eq!(
        error.message,
        "Docker CLI could not be started: Permission denied (os error 13)"
    );
}

#[tokio::test]
async fn malformed_docker_output_becomes_an_actionable_workspace_error() {
    let cli = FixtureCli::new([(
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
        ),
        success(include_str!("fixtures/docker/malformed-containers.jsonl")),
    )]);
    let docker = DockerWorkspace;

    let error = docker
        .refresh(&cli)
        .await
        .expect_err("fixture is malformed");

    assert!(error.message.contains("malformed container data"));
    assert!(error.message.contains("docker container ls"));
}
