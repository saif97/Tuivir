use std::{collections::VecDeque, future::Future, pin::Pin, sync::Mutex};

use virtui::{
    app::{App, AppEvent},
    cli::{CliRunner, ProcessError, ProcessFailure, ProcessOutput, ProcessSpec},
    docker::DockerWorkspace,
    provider::{
        ProviderRequest, ProviderWorkspace, ResourceCommand, ResourceId, WorkspaceError,
        WorkspaceSnapshot,
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
        ProviderRequest::ExecuteResourceCommand { .. } => panic!("expected refresh request"),
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
        .execute_command(&cli, &ResourceId::new("container-a"), ResourceCommand::Stop)
        .await
        .expect("Docker stop succeeds");
}

#[tokio::test]
async fn docker_delete_generates_the_expected_cli_request() {
    let cli = FixtureCli::new([(
        ProcessSpec::new("docker", &["container", "rm", "container-a"]),
        success("container-a\n"),
    )]);

    DockerWorkspace
        .execute_command(
            &cli,
            &ResourceId::new("container-a"),
            ResourceCommand::Delete,
        )
        .await
        .expect("Docker delete succeeds");
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
        failure("Error response from daemon: container is running"),
    )]);

    let error = DockerWorkspace
        .execute_command(
            &cli,
            &ResourceId::new("container-a"),
            ResourceCommand::Delete,
        )
        .await
        .expect_err("a non-zero exit is never a successful command");

    assert_eq!(
        error.message,
        "Error response from daemon: container is running"
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
