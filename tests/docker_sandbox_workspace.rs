use std::{
    collections::VecDeque,
    future::Future,
    pin::Pin,
    sync::Mutex,
};

use virtui::{
    cli::{CliRunner, ProcessError, ProcessFailure, ProcessOutput, ProcessSpec},
    docker_sandbox::DockerSandboxWorkspace,
    provider::{ProviderId, ProviderWorkspace, ResourceState},
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

/// sbx resolves a sandbox by name and by nothing else — the UUID it also
/// reports addresses no command — so the name is the Resource's identity.
#[tokio::test]
async fn sandboxes_become_resources_identified_by_name() {
    let cli = FixtureCli::new([(
        ProcessSpec::new("sbx", &["ls", "--json"]),
        success(include_str!("fixtures/docker-sandbox/sandboxes.json")),
    )]);

    let snapshot = DockerSandboxWorkspace
        .refresh(&cli)
        .await
        .expect("the fixture lists sandboxes");

    let panel = snapshot.panels.first().expect("a Sandboxes panel");
    assert_eq!(panel.title, "Sandboxes");
    assert_eq!(
        panel
            .resources
            .iter()
            .map(|resource| (
                resource.id.0.as_str(),
                resource.name.as_str(),
                resource.status.as_deref(),
                resource.state
            ))
            .collect::<Vec<_>>(),
        [
            (
                "claude-virtui",
                "claude-virtui",
                Some("running"),
                ResourceState::Running
            ),
            (
                "shell-dotfiles",
                "shell-dotfiles",
                Some("stopped"),
                ResourceState::Stopped
            ),
        ]
    );
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

/// A binary that exists but cannot be executed is installed, not absent, so it
/// is reported rather than silently dropped.
#[tokio::test]
async fn an_sbx_that_cannot_be_started_names_docker_sandbox_in_the_error() {
    let cli = FixtureCli::new([(
        ProcessSpec::new("sbx", &["version"]),
        Err(ProcessError::SpawnFailed("permission denied".to_owned())),
    )]);

    let discovered = DockerSandboxWorkspace
        .discover(&cli)
        .await
        .expect("a CLI that exists is never omitted");

    assert_eq!(
        discovered
            .error
            .expect("a provider that cannot start explains itself")
            .message,
        "Docker Sandbox CLI could not be started: permission denied. Run `sbx ls` to verify sandboxd is running and you are signed in to Docker."
    );
}

#[tokio::test]
async fn a_silent_version_probe_failure_still_explains_itself() {
    let cli = FixtureCli::new([(
        ProcessSpec::new("sbx", &["version"]),
        Err(ProcessError::Exited(ProcessFailure {
            exit_code: Some(1),
            stdout: String::new(),
            stderr: String::new(),
        })),
    )]);

    let discovered = DockerSandboxWorkspace
        .discover(&cli)
        .await
        .expect("a CLI that ran is never omitted");

    assert_eq!(
        discovered
            .error
            .expect("a silent failure still explains itself")
            .message,
        "Docker Sandbox could not report its version. Run `sbx ls` to verify sandboxd is running and you are signed in to Docker."
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
