use std::{
    collections::VecDeque,
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
};

use virtui::{
    app::{App, AppEvent},
    cli::{CliError, CliOutput, CliRunner, CommandSpec},
    incus::IncusWorkspace,
    provider::{
        ProviderRequest, ProviderWorkspace, ResourceCommand, ResourceId, WorkspaceError,
        WorkspaceSnapshot,
    },
    runtime::ProviderRuntime,
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
        CommandSpec::new("incus", &["start", "instance-a"]),
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
        CommandSpec::new("incus", &["stop", "instance-a"]),
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
        CommandSpec::new("incus", &["restart", "instance-a"]),
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
        CommandSpec::new("incus", &["delete", "instance-a"]),
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
            CommandSpec::new("incus", &["remote", "get-default"]),
            success("local\n"),
        ),
        (
            CommandSpec::new("incus", &["project", "get-current"]),
            success("production\n"),
        ),
        (
            CommandSpec::new("incus", &["list", "--format=json"]),
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
            CommandSpec::new("incus", &["remote", "get-default"]),
            success("local\n"),
        ),
        (
            CommandSpec::new("incus", &["project", "get-current"]),
            success("default\n"),
        ),
        (
            CommandSpec::new("incus", &["list", "--format=json"]),
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
        CommandSpec::new("incus", &["remote", "get-default"]),
        Err(CliError::NotFound),
    )]);

    assert!(IncusWorkspace.discover(&cli).await.is_none());
}

#[tokio::test]
async fn installed_but_unreachable_incus_stays_visible_with_provider_specific_error() {
    let cli = FixtureCli::new([(
        CommandSpec::new("incus", &["remote", "get-default"]),
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
            CommandSpec::new("incus", &["remote", "get-default"]),
            success("local\n"),
        ),
        (
            CommandSpec::new("incus", &["project", "get-current"]),
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
        CommandSpec::new("incus", &["list", "--format=json"]),
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
async fn runtime_with_builtin_providers_discovers_installed_incus() {
    let cli = FixtureCli::new([
        (
            CommandSpec::new("docker", &["context", "show"]),
            Err(CliError::NotFound),
        ),
        (
            CommandSpec::new("incus", &["remote", "get-default"]),
            success("local\n"),
        ),
        (
            CommandSpec::new("incus", &["project", "get-current"]),
            success("default\n"),
        ),
    ]);
    let runtime = ProviderRuntime::with_builtin_providers(Arc::new(cli));

    let discovered = runtime.discover().await;

    assert_eq!(discovered.len(), 1);
    assert_eq!(discovered[0].name, "Incus");
    assert_eq!(discovered[0].target_environment, "local / default");
}
