use std::{
    collections::VecDeque,
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
};

use virtui::{
    app::{App, AppEvent},
    cli::{CliRunner, ProcessError, ProcessFailure, ProcessOutput, ProcessSpec},
    incus::IncusWorkspace,
    provider::{
        ProviderRequest, ProviderWorkspace, ResourceCommand, ResourceId, WorkspaceError,
        WorkspaceSnapshot,
    },
    runtime::ProviderRuntime,
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
        exit_code: Some(1),
        stdout: String::new(),
        stderr: String::new(),
    }))
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
async fn incus_start_generates_the_expected_cli_request() {
    let cli = FixtureCli::new([(
        ProcessSpec::new("incus", &["start", "instance-a"]),
        success(""),
    )]);

    IncusWorkspace
        .execute_command(&cli, &ResourceId::new("instance-a"), ResourceCommand::Start)
        .await
        .expect("Incus start succeeds");
}

#[tokio::test]
async fn incus_stop_generates_the_expected_cli_request() {
    let cli = FixtureCli::new([(
        ProcessSpec::new("incus", &["stop", "instance-a"]),
        success(""),
    )]);

    IncusWorkspace
        .execute_command(&cli, &ResourceId::new("instance-a"), ResourceCommand::Stop)
        .await
        .expect("Incus stop succeeds");
}

#[tokio::test]
async fn incus_restart_generates_the_expected_cli_request() {
    let cli = FixtureCli::new([(
        ProcessSpec::new("incus", &["restart", "instance-a"]),
        success(""),
    )]);

    IncusWorkspace
        .execute_command(
            &cli,
            &ResourceId::new("instance-a"),
            ResourceCommand::Restart,
        )
        .await
        .expect("Incus restart succeeds");
}

#[tokio::test]
async fn incus_delete_generates_the_expected_cli_request() {
    let cli = FixtureCli::new([(
        ProcessSpec::new("incus", &["delete", "instance-a"]),
        success(""),
    )]);

    IncusWorkspace
        .execute_command(
            &cli,
            &ResourceId::new("instance-a"),
            ResourceCommand::Delete,
        )
        .await
        .expect("Incus delete succeeds");
}

#[tokio::test]
async fn discovered_incus_workspace_renders_target_environment_and_instances() {
    let cli = FixtureCli::new([
        (
            ProcessSpec::new("incus", &["remote", "get-default"]),
            success("local\n"),
        ),
        (
            ProcessSpec::new("incus", &["project", "get-current"]),
            success("production\n"),
        ),
        (
            ProcessSpec::new("incus", &["list", "--format=json"]),
            success(include_str!("fixtures/incus/instances.json")),
        ),
    ]);
    let incus = IncusWorkspace;

    let discovered = incus
        .discover(&cli)
        .await
        .expect("the fixture represents an installed Incus CLI");
    let mut app = App::new();
    let request = app
        .update(AppEvent::ProviderDiscovered(discovered))
        .into_iter()
        .next()
        .expect("discovery requests the first workspace refresh");
    app.update(refresh_completed(request, incus.refresh(&cli).await));

    let screen = render_to_text(app.state(), 100, 24);
    assert!(screen.contains("Incus"));
    assert!(screen.contains("Target: local / production"));
    assert!(screen.contains("Instances"));
    assert!(screen.contains("api"));
    assert!(screen.contains("Running"));
    assert!(screen.contains("Type: container"));
    assert!(screen.contains("database"));
    assert!(screen.contains("Stopped"));
}

#[tokio::test]
async fn reachable_incus_without_instances_renders_a_distinct_empty_state() {
    let cli = FixtureCli::new([
        (
            ProcessSpec::new("incus", &["remote", "get-default"]),
            success("local\n"),
        ),
        (
            ProcessSpec::new("incus", &["project", "get-current"]),
            success("default\n"),
        ),
        (
            ProcessSpec::new("incus", &["list", "--format=json"]),
            success("[]"),
        ),
    ]);
    let incus = IncusWorkspace;
    let discovered = incus.discover(&cli).await.expect("Incus is installed");
    let mut app = App::new();
    let request = app
        .update(AppEvent::ProviderDiscovered(discovered))
        .into_iter()
        .next()
        .expect("initial refresh");
    app.update(refresh_completed(request, incus.refresh(&cli).await));

    let screen = render_to_text(app.state(), 100, 24);
    assert!(screen.contains("Target: local / default"));
    assert!(screen.contains("No Incus instances found"));
    assert!(!screen.contains("unavailable"));
}

#[tokio::test]
async fn incus_is_omitted_when_its_cli_is_absent() {
    let cli = FixtureCli::new([(
        ProcessSpec::new("incus", &["remote", "get-default"]),
        Err(ProcessError::ExecutableNotFound),
    )]);

    assert!(IncusWorkspace.discover(&cli).await.is_none());
}

#[tokio::test]
async fn installed_but_unreachable_incus_stays_visible_with_provider_specific_error() {
    let cli = FixtureCli::new([(
        ProcessSpec::new("incus", &["remote", "get-default"]),
        failure("Error: Incus configuration is not accessible"),
    )]);
    let incus = IncusWorkspace;

    let discovered = incus.discover(&cli).await.expect("Incus is installed");
    let mut app = App::new();
    let requests = app.update(AppEvent::ProviderDiscovered(discovered));

    assert!(
        requests.is_empty(),
        "an unreachable provider is not refreshed"
    );
    let screen = render_to_text(app.state(), 200, 24);
    assert!(screen.contains("Incus provider is unavailable"));
    assert!(screen.contains("Incus configuration is not accessible"));
    assert!(screen.contains("incus remote"));
    assert!(screen.contains("get-default"));
}

#[tokio::test]
async fn incus_with_unreadable_current_project_stays_visible() {
    let cli = FixtureCli::new([
        (
            ProcessSpec::new("incus", &["remote", "get-default"]),
            success("local\n"),
        ),
        (
            ProcessSpec::new("incus", &["project", "get-current"]),
            failure("Error: Incus project configuration is not accessible"),
        ),
    ]);
    let incus = IncusWorkspace;

    let discovered = incus.discover(&cli).await.expect("Incus is installed");

    assert_eq!(discovered.name, "Incus");
    let error = discovered.error.expect("the provider exposes its error");
    assert!(
        error
            .message
            .contains("Incus project configuration is not accessible")
    );
    assert!(error.message.contains("incus project get-current"));
}

#[tokio::test]
async fn failed_instance_refresh_identifies_incus_command_and_target() {
    let cli = FixtureCli::new([(
        ProcessSpec::new("incus", &["list", "--format=json"]),
        failure("Error: Unable to connect to Incus"),
    )]);
    let incus = IncusWorkspace;

    let error = incus
        .refresh(&cli)
        .await
        .expect_err("the fixture represents an unreachable Incus provider");

    assert!(error.message.contains("Unable to connect to Incus"));
    assert!(error.message.contains("incus list"));
    assert!(error.message.contains("Target Environment"));
}

#[tokio::test]
async fn a_silent_instance_refresh_failure_still_explains_itself() {
    let cli = FixtureCli::new([(
        ProcessSpec::new("incus", &["list", "--format=json"]),
        silent_failure(),
    )]);

    let error = IncusWorkspace
        .refresh(&cli)
        .await
        .expect_err("a non-zero exit is never a successful refresh");

    assert_eq!(
        error.message,
        "Incus could not list instances. Run `incus list` to verify access to the current Target Environment."
    );
}

#[tokio::test]
async fn a_failed_instance_refresh_reports_what_incus_printed_on_stdout() {
    let cli = FixtureCli::new([(
        ProcessSpec::new("incus", &["list", "--format=json"]),
        failure_on_stdout("Error: not authorized\n"),
    )]);

    let error = IncusWorkspace
        .refresh(&cli)
        .await
        .expect_err("a non-zero exit is never a successful refresh");

    assert!(
        error.message.contains("Error: not authorized"),
        "workspace error: {}",
        error.message
    );
    assert!(error.message.contains("incus list"));
}

#[tokio::test]
async fn a_silent_command_failure_names_the_operation_and_instance() {
    let cli = FixtureCli::new([(
        ProcessSpec::new("incus", &["restart", "instance-a"]),
        silent_failure(),
    )]);

    let error = IncusWorkspace
        .execute_command(
            &cli,
            &ResourceId::new("instance-a"),
            ResourceCommand::Restart,
        )
        .await
        .expect_err("a non-zero exit is never a successful command");

    assert_eq!(error.message, "Incus could not restart instance instance-a");
}

#[tokio::test]
async fn a_failed_command_reports_what_incus_wrote_to_stderr() {
    let cli = FixtureCli::new([(
        ProcessSpec::new("incus", &["delete", "instance-a"]),
        failure("Error: The instance is currently running"),
    )]);

    let error = IncusWorkspace
        .execute_command(
            &cli,
            &ResourceId::new("instance-a"),
            ResourceCommand::Delete,
        )
        .await
        .expect_err("a non-zero exit is never a successful command");

    assert_eq!(error.message, "Error: The instance is currently running");
}

#[tokio::test]
async fn an_incus_cli_that_cannot_be_started_names_incus_in_the_error() {
    let cli = FixtureCli::new([(
        ProcessSpec::new("incus", &["stop", "instance-a"]),
        Err(ProcessError::SpawnFailed(
            "Permission denied (os error 13)".to_owned(),
        )),
    )]);

    let error = IncusWorkspace
        .execute_command(&cli, &ResourceId::new("instance-a"), ResourceCommand::Stop)
        .await
        .expect_err("a CLI that never started is never a successful command");

    assert_eq!(
        error.message,
        "Incus CLI could not be started: Permission denied (os error 13)"
    );
}

#[tokio::test]
async fn a_silent_discovery_failure_names_the_probe_that_failed() {
    let cli = FixtureCli::new([(
        ProcessSpec::new("incus", &["remote", "get-default"]),
        silent_failure(),
    )]);

    let discovered = IncusWorkspace
        .discover(&cli)
        .await
        .expect("Incus is installed");

    let error = discovered.error.expect("the provider exposes its error");
    assert_eq!(
        error.message,
        "Incus could not report its default remote. Run `incus remote get-default` to verify the selected Target Environment."
    );
}

#[tokio::test]
async fn incus_that_disappears_during_discovery_stays_visible_with_an_error() {
    let cli = FixtureCli::new([
        (
            ProcessSpec::new("incus", &["remote", "get-default"]),
            success("local\n"),
        ),
        (
            ProcessSpec::new("incus", &["project", "get-current"]),
            Err(ProcessError::ExecutableNotFound),
        ),
    ]);

    let discovered = IncusWorkspace
        .discover(&cli)
        .await
        .expect("the initial probe already proved Incus was installed");

    let error = discovered.error.expect("the provider exposes its error");
    assert!(
        error.message.contains("Incus CLI is no longer available"),
        "workspace error: {}",
        error.message
    );
    assert!(error.message.contains("incus project get-current"));
}

#[tokio::test]
async fn runtime_with_builtin_providers_discovers_installed_incus() {
    let cli = FixtureCli::new([
        (
            ProcessSpec::new("docker", &["context", "show"]),
            Err(ProcessError::ExecutableNotFound),
        ),
        (
            ProcessSpec::new("incus", &["remote", "get-default"]),
            success("local\n"),
        ),
        (
            ProcessSpec::new("incus", &["project", "get-current"]),
            success("default\n"),
        ),
    ]);
    let runtime = ProviderRuntime::with_builtin_providers(Arc::new(cli));

    let discovered = runtime.discover().await;

    assert_eq!(discovered.len(), 1);
    assert_eq!(discovered[0].name, "Incus");
    assert_eq!(discovered[0].target_environment, "local / default");
}
