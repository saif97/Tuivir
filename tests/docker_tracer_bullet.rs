use std::{collections::VecDeque, future::Future, pin::Pin, sync::Mutex};

use vertui::{
    app::{App, AppEvent},
    cli::{CliError, CliOutput, CliRunner, CommandSpec},
    docker::DockerWorkspace,
    provider::{ProviderAction, ProviderWorkspace},
    ui::render_to_text,
};

struct FixtureCli {
    responses: Mutex<VecDeque<(CommandSpec, Result<CliOutput, CliError>)>>,
}

impl FixtureCli {
    fn new(
        responses: impl IntoIterator<Item = (CommandSpec, Result<CliOutput, CliError>)>,
    ) -> Self {
        Self {
            responses: Mutex::new(responses.into_iter().collect()),
        }
    }
}

impl CliRunner for FixtureCli {
    fn run<'a>(
        &'a self,
        command: CommandSpec,
    ) -> Pin<Box<dyn Future<Output = Result<CliOutput, CliError>> + Send + 'a>> {
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

fn success(stdout: &str) -> Result<CliOutput, CliError> {
    Ok(CliOutput {
        success: true,
        stdout: stdout.to_owned(),
        stderr: String::new(),
    })
}

fn failure(stderr: &str) -> Result<CliOutput, CliError> {
    Ok(CliOutput {
        success: false,
        stdout: String::new(),
        stderr: stderr.to_owned(),
    })
}

#[tokio::test]
async fn discovered_docker_workspace_renders_target_environment_and_containers() {
    let cli = FixtureCli::new([
        (
            CommandSpec::new("docker", &["context", "show"]),
            success("desktop-linux\n"),
        ),
        (
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
            ),
            success(include_str!("fixtures/docker/containers.jsonl")),
        ),
    ]);
    let docker = DockerWorkspace::new();

    let discovered = docker
        .discover(&cli)
        .await
        .expect("the fixture represents an installed Docker CLI");
    let mut app = App::new();
    let actions = app.update(AppEvent::ProviderDiscovered(discovered));
    let request = actions
        .into_iter()
        .map(|action| match action {
            ProviderAction::RefreshWorkspace(request) => request,
        })
        .next()
        .expect("discovery requests the first workspace refresh");

    let snapshot = docker.refresh(&cli).await;
    app.update(AppEvent::RefreshCompleted {
        request,
        result: snapshot,
    });

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
            CommandSpec::new("docker", &["context", "show"]),
            success("colima\n"),
        ),
        (
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
            ),
            success(""),
        ),
    ]);
    let docker = DockerWorkspace::new();
    let discovered = docker.discover(&cli).await.expect("Docker is installed");
    let mut app = App::new();
    let request = app
        .update(AppEvent::ProviderDiscovered(discovered))
        .into_iter()
        .map(|action| match action {
            ProviderAction::RefreshWorkspace(request) => request,
        })
        .next()
        .expect("initial refresh");
    app.update(AppEvent::RefreshCompleted {
        request,
        result: docker.refresh(&cli).await,
    });

    let screen = render_to_text(app.state(), 100, 24);
    assert!(screen.contains("Target: colima"));
    assert!(screen.contains("No Docker containers found"));
    assert!(!screen.contains("unavailable"));
}

#[tokio::test]
async fn installed_but_unreachable_docker_stays_visible_with_actionable_error() {
    let cli = FixtureCli::new([(
        CommandSpec::new("docker", &["context", "show"]),
        failure("Cannot connect to the Docker daemon"),
    )]);
    let docker = DockerWorkspace::new();
    let discovered = docker.discover(&cli).await.expect("Docker is installed");
    let mut app = App::new();

    let actions = app.update(AppEvent::ProviderDiscovered(discovered));

    assert!(
        actions.is_empty(),
        "an unreachable provider is not refreshed"
    );
    let screen = render_to_text(app.state(), 100, 24);
    assert!(screen.contains("Docker provider is unavailable"));
    assert!(screen.contains("Cannot connect to the Docker daemon"));
    assert!(screen.contains("docker context show"));
}

#[tokio::test]
async fn failed_container_refresh_identifies_docker_operation_and_target() {
    let cli = FixtureCli::new([
        (
            CommandSpec::new("docker", &["context", "show"]),
            success("desktop-linux\n"),
        ),
        (
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
            ),
            failure(include_str!("fixtures/docker/daemon-unreachable.stderr")),
        ),
    ]);
    let docker = DockerWorkspace::new();
    let discovered = docker.discover(&cli).await.expect("Docker is installed");
    let mut app = App::new();
    let request = app
        .update(AppEvent::ProviderDiscovered(discovered))
        .into_iter()
        .map(|action| match action {
            ProviderAction::RefreshWorkspace(request) => request,
        })
        .next()
        .expect("initial refresh");
    app.update(AppEvent::RefreshCompleted {
        request,
        result: docker.refresh(&cli).await,
    });

    let screen = render_to_text(app.state(), 140, 24);
    assert!(screen.contains("Target: desktop-linux"));
    assert!(screen.contains("permission denied connecting to Docker socket"));
    assert!(screen.contains("Run `docker"));
    assert!(screen.contains("container ls --all`"));
}

#[tokio::test]
async fn malformed_docker_output_becomes_an_actionable_workspace_error() {
    let cli = FixtureCli::new([(
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
        ),
        success(include_str!("fixtures/docker/malformed-containers.jsonl")),
    )]);
    let docker = DockerWorkspace::new();

    let error = docker
        .refresh(&cli)
        .await
        .expect_err("fixture is malformed");

    assert!(error.message.contains("malformed container data"));
    assert!(error.message.contains("docker container ls"));
}
