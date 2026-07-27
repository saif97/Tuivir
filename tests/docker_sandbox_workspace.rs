use std::{
    collections::VecDeque,
    future::Future,
    pin::Pin,
    sync::Mutex,
};

use virtui::{
    cli::{CliRunner, ProcessError, ProcessFailure, ProcessOutput, ProcessSpec},
    docker_sandbox::DockerSandboxWorkspace,
    provider::{ProviderId, ProviderWorkspace},
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

/// `sbx version` reports a version and a build commit on one line; only the
/// version identifies the Target Environment.
#[tokio::test]
async fn an_installed_docker_sandbox_reports_the_sbx_version_as_its_target_environment() {
    let cli = FixtureCli::new([
        (
            ProcessSpec::new("sbx", &["version"]),
            success("sbx version: v0.37.0 8b65b864b0d49c29f05a55170d6b5eea4c0d11e7\n"),
        ),
        (
            ProcessSpec::new("sbx", &["ls", "--json"]),
            success(include_str!("fixtures/docker-sandbox/sandboxes.json")),
        ),
    ]);

    let discovered = DockerSandboxWorkspace
        .discover(&cli)
        .await
        .expect("the fixture represents an installed sbx");

    assert_eq!(discovered.id, ProviderId::new("docker-sandbox"));
    assert_eq!(discovered.name, "Docker Sandbox");
    assert_eq!(discovered.target_environment, "v0.37.0");
    assert_eq!(discovered.error, None);
}

fn failure(stderr: &str) -> Result<ProcessOutput, ProcessError> {
    Err(ProcessError::Exited(ProcessFailure {
        exit_code: Some(1),
        stdout: String::new(),
        stderr: stderr.to_owned(),
    }))
}

/// An installed sbx whose daemon is down or whose Docker login has lapsed is a
/// Provider the user can act on, so it stays on screen instead of vanishing
/// the way an uninstalled one does.
#[tokio::test]
async fn installed_docker_sandbox_that_cannot_list_stays_visible_with_an_actionable_error() {
    let cli = FixtureCli::new([
        (
            ProcessSpec::new("sbx", &["version"]),
            success("sbx version: v0.37.0 8b65b864b0d49c29f05a55170d6b5eea4c0d11e7\n"),
        ),
        (
            ProcessSpec::new("sbx", &["ls", "--json"]),
            failure("Error: not signed in to Docker"),
        ),
    ]);

    let discovered = DockerSandboxWorkspace
        .discover(&cli)
        .await
        .expect("an installed sbx is never omitted");

    assert_eq!(discovered.name, "Docker Sandbox");
    assert_eq!(discovered.target_environment, "unavailable");
    assert_eq!(
        discovered.error.expect("an unusable provider explains itself").message,
        "Error: not signed in to Docker. Run `sbx ls` to verify sandboxd is running and you are signed in to Docker."
    );
}

#[tokio::test]
async fn docker_sandbox_is_omitted_when_its_cli_is_absent() {
    let cli = FixtureCli::new([(
        ProcessSpec::new("sbx", &["version"]),
        Err(ProcessError::ExecutableNotFound),
    )]);

    assert!(DockerSandboxWorkspace.discover(&cli).await.is_none());
}
