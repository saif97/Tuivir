use std::sync::Arc;

use virtui::{
    application::{App, InteractiveShellProcess},
    cli::{ProcessError, ProcessOutput, ProcessSpec},
    incus::IncusWorkspace,
    presentation::render_to_text,
    provider::{
        DetailViewId, ProviderWorkspace, ResourceCommand, ResourceId, ResourcePanelId,
        ResourceState, ResourceTarget,
    },
    runtime::ProviderRuntime,
};

mod common;
use common::{FixtureCli, failure, failure_on_stdout, refresh_completed, success};

fn target(panel_id: &str, resource_id: &str) -> ResourceTarget {
    ResourceTarget::new(ResourcePanelId::new(panel_id), ResourceId::new(resource_id))
}

fn silent_failure() -> Result<ProcessOutput, ProcessError> {
    common::silent_failure(1)
}

#[tokio::test]
async fn incus_start_generates_the_expected_cli_request() {
    let cli = FixtureCli::new([(
        ProcessSpec::new("incus", &["start", "instance-a"]),
        success(""),
    )]);

    IncusWorkspace
        .execute_command(
            &cli,
            &target("instances", "instance-a"),
            ResourceCommand::Start,
            ResourceState::Stopped,
        )
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
        .execute_command(
            &cli,
            &target("instances", "instance-a"),
            ResourceCommand::Stop,
            ResourceState::Running,
        )
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
            &target("instances", "instance-a"),
            ResourceCommand::Restart,
            ResourceState::Running,
        )
        .await
        .expect("Incus restart succeeds");
}

#[tokio::test]
async fn incus_resume_generates_the_expected_cli_request() {
    let cli = FixtureCli::new([(
        ProcessSpec::new("incus", &["unfreeze", "instance-a"]),
        success(""),
    )]);

    IncusWorkspace
        .execute_command(
            &cli,
            &target("instances", "instance-a"),
            ResourceCommand::Resume,
            ResourceState::Paused,
        )
        .await
        .expect("Incus resume succeeds");
}

#[tokio::test]
async fn deleting_a_stopped_instance_generates_the_expected_cli_request() {
    let cli = FixtureCli::new([(
        ProcessSpec::new("incus", &["delete", "instance-a"]),
        success(""),
    )]);

    IncusWorkspace
        .execute_command(
            &cli,
            &target("instances", "instance-a"),
            ResourceCommand::Delete,
            ResourceState::Stopped,
        )
        .await
        .expect("Incus delete succeeds");
}

/// Incus deletes an instance without `--force` only when it is stopped; a
/// frozen or transitioning one is refused outright.
#[tokio::test]
async fn deleting_an_instance_that_is_not_stopped_forces_removal() {
    for state in [
        ResourceState::Paused,
        ResourceState::Transitioning,
        ResourceState::Broken,
        ResourceState::Unknown,
    ] {
        let cli = FixtureCli::new([(
            ProcessSpec::new("incus", &["delete", "--force", "instance-a"]),
            success(""),
        )]);

        IncusWorkspace
            .execute_command(
                &cli,
                &target("instances", "instance-a"),
                ResourceCommand::Delete,
                state,
            )
            .await
            .unwrap_or_else(|error| panic!("force delete from {state:?} succeeds: {error:?}"));
    }
}

/// The Instances panel advertises Incus's own views rather than borrowing
/// Docker's names for them.
#[tokio::test]
async fn the_instances_panel_declares_incuss_native_detail_views() {
    let cli = FixtureCli::new([(
        ProcessSpec::new("incus", &["list", "--format=json"]),
        success(include_str!("fixtures/incus/instances.json")),
    )]);

    let snapshot = IncusWorkspace
        .refresh(&cli)
        .await
        .expect("fixture lists instances");

    let panel = snapshot.panels.first().expect("an Instances panel");
    assert_eq!(
        panel
            .detail_views
            .iter()
            .map(|view| (view.id.0.as_str(), view.title.as_str()))
            .collect::<Vec<_>>(),
        [
            ("info", "Info"),
            ("config", "Config"),
            ("console-log", "Console Log")
        ]
    );
}

/// Each declared view runs exactly one Incus command. The fixture answers one
/// request and panics on any other, so a view that loaded more than the one on
/// screen would fail here.
#[tokio::test]
async fn each_detail_view_runs_its_own_incus_command() {
    for (view, expected) in [
        ("info", ProcessSpec::new("incus", &["info", "gateway"])),
        (
            "config",
            ProcessSpec::new("incus", &["config", "show", "gateway"]),
        ),
        (
            "console-log",
            ProcessSpec::new("incus", &["console", "--show-log", "gateway"]),
        ),
    ] {
        let cli = FixtureCli::new([(expected, success("first line\nsecond line\n"))]);

        let details = IncusWorkspace
            .load_details(
                &cli,
                &target("instances", "gateway"),
                &DetailViewId::new(view),
            )
            .await
            .unwrap_or_else(|error| panic!("Incus {view} loads: {error:?}"));

        assert_eq!(details.lines, ["first line", "second line"], "view {view}");
    }
}

#[tokio::test]
async fn an_instance_with_no_console_log_loads_empty_details() {
    let cli = FixtureCli::new([(
        ProcessSpec::new("incus", &["console", "--show-log", "gateway"]),
        success(""),
    )]);

    let details = IncusWorkspace
        .load_details(
            &cli,
            &target("instances", "gateway"),
            &DetailViewId::new("console-log"),
        )
        .await
        .expect("no output is not a failure");

    assert!(details.is_empty());
}

#[tokio::test]
async fn a_failed_detail_view_reports_what_incus_wrote_to_stderr() {
    let cli = FixtureCli::new([(
        ProcessSpec::new("incus", &["info", "gateway"]),
        failure("Error: Instance not found"),
    )]);

    let error = IncusWorkspace
        .load_details(
            &cli,
            &target("instances", "gateway"),
            &DetailViewId::new("info"),
        )
        .await
        .expect_err("a non-zero exit is never loaded details");

    assert_eq!(error.message, "Error: Instance not found");
}

/// Provider output is displayed, not reformatted, so the nesting Incus laid
/// out survives into the panel.
#[tokio::test]
async fn info_output_reaches_the_panel_line_for_line() {
    let cli = FixtureCli::new([(
        ProcessSpec::new("incus", &["info", "gateway"]),
        success(include_str!("fixtures/incus/instance-info.txt")),
    )]);

    let details = IncusWorkspace
        .load_details(
            &cli,
            &target("instances", "gateway"),
            &DetailViewId::new("info"),
        )
        .await
        .expect("fixture describes the instance");

    assert_eq!(details.lines.len(), 20);
    assert_eq!(
        details.lines.first().map(String::as_str),
        Some("Name: gateway")
    );
    assert!(
        details
            .lines
            .contains(&"        inet: 10.62.14.31/24 (global)".to_owned()),
        "indentation is preserved: {:?}",
        details.lines
    );
    // A blank separator line is part of what Incus laid out.
    assert!(details.lines.contains(&String::new()));
}

#[tokio::test]
async fn a_detail_view_incus_never_declared_is_refused_without_running_anything() {
    // The fixture panics on any CLI request, so a view resolved to a command
    // would fail here rather than return.
    let cli = FixtureCli::new([]);

    let error = IncusWorkspace
        .load_details(
            &cli,
            &target("instances", "gateway"),
            &DetailViewId::new("stats"),
        )
        .await
        .expect_err("Incus declares no stats view");

    assert_eq!(
        error.message,
        "Incus has no stats view for instance gateway"
    );
}

#[tokio::test]
async fn a_silent_detail_failure_names_the_view_and_instance() {
    let cli = FixtureCli::new([(
        ProcessSpec::new("incus", &["config", "show", "gateway"]),
        silent_failure(),
    )]);

    let error = IncusWorkspace
        .load_details(
            &cli,
            &target("instances", "gateway"),
            &DetailViewId::new("config"),
        )
        .await
        .expect_err("a non-zero exit is never loaded details");

    assert_eq!(
        error.message,
        "Incus could not load config for instance gateway"
    );
}

#[tokio::test]
async fn incus_maps_every_instance_status_into_the_shared_vocabulary() {
    let cli = FixtureCli::new([(
        ProcessSpec::new("incus", &["list", "--format=json"]),
        success(include_str!("fixtures/incus/mixed-state-instances.json")),
    )]);

    let snapshot = IncusWorkspace
        .refresh(&cli)
        .await
        .expect("fixture lists instances");

    let states = snapshot
        .targets()
        .map(|(_, resource)| resource)
        .map(|resource| {
            (
                resource.name.as_str(),
                resource.state.expect("instances have lifecycle state"),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        states,
        [
            ("api", ResourceState::Running),
            ("database", ResourceState::Stopped),
            ("cache", ResourceState::Paused),
            ("builder", ResourceState::Transitioning),
            ("gateway", ResourceState::Transitioning),
            ("broken", ResourceState::Broken),
            // A status this Incus release never returned still has to land
            // somewhere honest rather than masquerade as stopped.
            ("future", ResourceState::Unknown),
        ]
    );
}

/// `incus exec` reaches only into a running instance, so no other state may
/// carry the Interactive Shell that runs it. An Incus instance is a whole
/// system, so its shell is the root login shell — the same one `incus shell`
/// itself expands to — rather than the bare `/bin/sh` a minimal Docker image
/// forces.
#[tokio::test]
async fn only_a_running_instance_carries_an_interactive_shell() {
    let cli = FixtureCli::new([(
        ProcessSpec::new("incus", &["list", "--format=json"]),
        success(include_str!("fixtures/incus/mixed-state-instances.json")),
    )]);

    let snapshot = IncusWorkspace
        .refresh(&cli)
        .await
        .expect("fixture lists instances");

    let shells = snapshot
        .resources()
        .filter_map(|resource| {
            resource
                .shell
                .as_ref()
                .map(|shell| (resource.name.as_str(), shell.clone()))
        })
        .collect::<Vec<_>>();
    assert_eq!(
        shells,
        [(
            "api",
            InteractiveShellProcess::new("incus", &["exec", "api", "--", "su", "-l"]),
        )]
    );
}

/// `incus unfreeze` succeeds only against a frozen instance, so no other state
/// may offer the Command that runs it.
#[tokio::test]
async fn only_a_frozen_instance_offers_the_resume_command() {
    let cli = FixtureCli::new([(
        ProcessSpec::new("incus", &["list", "--format=json"]),
        success(include_str!("fixtures/incus/mixed-state-instances.json")),
    )]);

    let snapshot = IncusWorkspace
        .refresh(&cli)
        .await
        .expect("fixture lists instances");

    let resumable = snapshot
        .targets()
        .map(|(_, resource)| resource)
        .filter(|resource| {
            resource
                .available_commands
                .contains(&ResourceCommand::Resume)
        })
        .map(|resource| resource.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(resumable, ["cache"]);

    let frozen = snapshot
        .targets()
        .map(|(_, resource)| resource)
        .find(|resource| resource.name == "cache")
        .expect("fixture has a frozen instance");
    assert_eq!(
        frozen.available_commands,
        [ResourceCommand::Resume, ResourceCommand::Delete]
    );
}

#[tokio::test]
async fn deleting_a_running_instance_forces_removal_without_a_second_query() {
    // The fixture answers exactly one CLI request and panics on any other, so
    // this also proves the Resource State travels with the request instead of
    // being rediscovered through the Incus CLI.
    let cli = FixtureCli::new([(
        ProcessSpec::new("incus", &["delete", "--force", "instance-a"]),
        success(""),
    )]);

    IncusWorkspace
        .execute_command(
            &cli,
            &target("instances", "instance-a"),
            ResourceCommand::Delete,
            ResourceState::Running,
        )
        .await
        .expect("Incus force delete succeeds");
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
        .update(discovered.into_event())
        .into_iter()
        .next()
        .expect("discovery requests the first workspace refresh");
    app.update(refresh_completed(request, incus.refresh(&cli).await));

    let screen = render_to_text(app.state(), 100, 24);
    assert!(screen.contains("Incus"));
    assert!(screen.contains("[ Incus · local / production ]"));
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
        .update(discovered.into_event())
        .into_iter()
        .next()
        .expect("initial refresh");
    app.update(refresh_completed(request, incus.refresh(&cli).await));

    let screen = render_to_text(app.state(), 100, 24);
    assert!(screen.contains("[ Incus · local / default ]"));
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
    let requests = app.update(discovered.into_event());

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

    assert_eq!(discovered.provider().name(), "Incus");
    let error = discovered.error().expect("the provider exposes its error");
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
            &target("instances", "instance-a"),
            ResourceCommand::Restart,
            ResourceState::Running,
        )
        .await
        .expect_err("a non-zero exit is never a successful command");

    assert_eq!(error.message, "Incus could not restart instance instance-a");
}

#[tokio::test]
async fn a_failed_command_reports_what_incus_wrote_to_stderr() {
    let cli = FixtureCli::new([(
        ProcessSpec::new("incus", &["delete", "instance-a"]),
        failure("Error: Failed to destroy the instance storage volume"),
    )]);

    let error = IncusWorkspace
        .execute_command(
            &cli,
            &target("instances", "instance-a"),
            ResourceCommand::Delete,
            ResourceState::Stopped,
        )
        .await
        .expect_err("a non-zero exit is never a successful command");

    assert_eq!(
        error.message,
        "Error: Failed to destroy the instance storage volume"
    );
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
        .execute_command(
            &cli,
            &target("instances", "instance-a"),
            ResourceCommand::Stop,
            ResourceState::Running,
        )
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

    let error = discovered.error().expect("the provider exposes its error");
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

    let error = discovered.error().expect("the provider exposes its error");
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
        (
            ProcessSpec::new("sbx", &["version"]),
            Err(ProcessError::ExecutableNotFound),
        ),
    ]);
    let runtime = ProviderRuntime::with_builtin_providers(Arc::new(cli));

    let discovered = runtime.discover().await;

    assert_eq!(discovered.len(), 1);
    assert_eq!(discovered[0].provider().name(), "Incus");
    assert_eq!(
        discovered[0].provider().target_environment(),
        &"local / default"
    );
}
