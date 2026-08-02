use std::sync::Arc;

use virtui::{
    application::{App, Command, InteractiveShellProcess, ProviderRequest, ResourceCommand},
    domain::{
        DetailViewId, ProviderId, ProviderVersion, ResourceId, ResourcePanelId, ResourceState,
        ResourceTarget, TargetEnvironment,
    },
    infrastructure::runtime::ProviderRuntime,
    infrastructure::{
        process::{ProcessError, ProcessFailure, ProcessSpec},
        provider::{DockerSandboxWorkspace, ProviderWorkspace},
    },
    presentation::render_to_text,
};

mod common;
use common::{FixtureCli, failure, refresh_completed, success};

fn target(panel_id: &str, resource_id: &str) -> ResourceTarget {
    ResourceTarget::new(ResourcePanelId::new(panel_id), ResourceId::new(resource_id))
}

/// `sbx version` describes the installed Provider; it does not identify which
/// Target Environment its daemon currently operates.
#[tokio::test]
async fn docker_sandbox_keeps_provider_version_separate_from_its_target_environment() {
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

    assert_eq!(
        discovered.provider().id(),
        &ProviderId::new("docker-sandbox")
    );
    assert_eq!(discovered.provider().name(), "Docker Sandbox");
    assert_eq!(
        discovered.provider().target_environment(),
        &TargetEnvironment::new("local")
    );
    assert_eq!(
        discovered.provider().version(),
        Some(&ProviderVersion::new("v0.37.0"))
    );
    assert_eq!(discovered.error(), None);
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
    assert_eq!(panel.id, ResourcePanelId::new("sandboxes"));
    assert_eq!(panel.title, "Sandboxes");
    assert_eq!(
        panel
            .resources
            .iter()
            .map(|resource| (
                resource.id.0.as_str(),
                resource.name.as_str(),
                resource.status.as_deref(),
                resource.state.expect("sandboxes have lifecycle state")
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

/// sbx offers no pause and no restart, so nothing maps to Paused. A status
/// this workspace does not recognise stays Unknown rather than passing for
/// stopped, which is what keeps a destructive Command failing safe.
#[tokio::test]
async fn docker_sandbox_maps_every_status_into_the_shared_vocabulary() {
    let cli = FixtureCli::new([(
        ProcessSpec::new("sbx", &["ls", "--json"]),
        success(include_str!(
            "fixtures/docker-sandbox/mixed-state-sandboxes.json"
        )),
    )]);

    let snapshot = DockerSandboxWorkspace
        .refresh(&cli)
        .await
        .expect("the fixture lists sandboxes");

    assert_eq!(
        snapshot
            .targets()
            .map(|(_, resource)| resource)
            .map(|resource| {
                (
                    resource.name.as_str(),
                    resource.state.expect("sandboxes have lifecycle state"),
                )
            })
            .collect::<Vec<_>>(),
        [
            ("running-sandbox", ResourceState::Running),
            ("stopped-sandbox", ResourceState::Stopped),
            ("shouting-sandbox", ResourceState::Running),
            ("starting-sandbox", ResourceState::Unknown),
            ("unrecognised-sandbox", ResourceState::Unknown),
        ]
    );
}

/// The UUID addresses no sbx command, but it is what the user quotes in a bug
/// report, so it stays visible as a field rather than becoming the identity.
#[tokio::test]
async fn a_sandbox_carries_its_agent_uuid_and_workspaces_as_fields() {
    let cli = FixtureCli::new([(
        ProcessSpec::new("sbx", &["ls", "--json"]),
        success(include_str!("fixtures/docker-sandbox/sandboxes.json")),
    )]);

    let snapshot = DockerSandboxWorkspace
        .refresh(&cli)
        .await
        .expect("the fixture lists sandboxes");

    let sandbox = snapshot
        .targets()
        .map(|(_, resource)| resource)
        .find(|resource| resource.name == "claude-virtui")
        .expect("the fixture lists claude-virtui");
    assert_eq!(
        sandbox
            .fields
            .iter()
            .map(|(label, value)| (label.as_str(), value.as_str()))
            .collect::<Vec<_>>(),
        [
            ("Agent", "claude"),
            ("ID", "3f2a1c88-91b4-4d0e-9c77-1e5b0a6d2f43"),
            ("Workspaces", "/home/vibebox/projects/virtui"),
        ]
    );
}

/// A field with nothing behind it renders as a bare label in the summary, so
/// a sandbox mounting no host path leaves the row out rather than showing
/// "Workspaces:" with nothing after it.
#[tokio::test]
async fn a_sandbox_with_no_workspaces_omits_the_field_entirely() {
    let cli = FixtureCli::new([(
        ProcessSpec::new("sbx", &["ls", "--json"]),
        success(
            r#"{"sandboxes":[{"name":"no-workspace","id":"77777777-7777-4777-8777-777777777777","agent":"shell","status":"stopped","workspaces":[]}]}"#,
        ),
    )]);

    let snapshot = DockerSandboxWorkspace
        .refresh(&cli)
        .await
        .expect("the fixture lists one sandbox");

    let sandbox = snapshot
        .targets()
        .map(|(_, resource)| resource)
        .next()
        .expect("one sandbox");
    assert_eq!(
        sandbox
            .fields
            .iter()
            .map(|(label, _)| label.as_str())
            .collect::<Vec<_>>(),
        ["Agent", "ID"]
    );
}

/// `sbx run --name` is the documented way to reattach, but it opens an
/// interactive agent session that never exits, which would leave the request
/// pending forever. `sbx exec` starts a stopped sandbox before running its
/// command, and `-d` returns as soon as it has.
#[tokio::test]
async fn starting_a_sandbox_generates_the_expected_cli_request() {
    let cli = FixtureCli::new([(
        ProcessSpec::new("sbx", &["exec", "-d", "shell-dotfiles", "true"]),
        success(""),
    )]);

    DockerSandboxWorkspace
        .execute_command(
            &cli,
            &target("sandboxes", "shell-dotfiles"),
            ResourceCommand::Start,
            ResourceState::Stopped,
        )
        .await
        .expect("Docker Sandbox start succeeds");
}

#[tokio::test]
async fn stopping_a_sandbox_generates_the_expected_cli_request() {
    let cli = FixtureCli::new([(
        ProcessSpec::new("sbx", &["stop", "claude-virtui"]),
        success(""),
    )]);

    DockerSandboxWorkspace
        .execute_command(
            &cli,
            &target("sandboxes", "claude-virtui"),
            ResourceCommand::Stop,
            ResourceState::Running,
        )
        .await
        .expect("Docker Sandbox stop succeeds");
}

/// Unlike Docker and Incus, the force here is not about a running Resource.
/// `sbx rm` prompts for a confirmation it reads from a terminal Virtui never
/// gives it, so every deletion needs `--force` to proceed at all — including
/// one from a settled, stopped sandbox.
#[tokio::test]
async fn deleting_a_sandbox_always_forces_regardless_of_state() {
    for state in [
        ResourceState::Running,
        ResourceState::Stopped,
        ResourceState::Paused,
        ResourceState::Transitioning,
        ResourceState::Broken,
        ResourceState::Unknown,
    ] {
        let cli = FixtureCli::new([(
            ProcessSpec::new("sbx", &["rm", "--force", "claude-virtui"]),
            success(""),
        )]);

        DockerSandboxWorkspace
            .execute_command(
                &cli,
                &target("sandboxes", "claude-virtui"),
                ResourceCommand::Delete,
                state,
            )
            .await
            .unwrap_or_else(|error| panic!("delete from {state:?} succeeds: {error:?}"));
    }
}

/// sbx has no restart, and no pause to resume from. Neither Command is ever
/// offered, and the fixture panics on any CLI request, so reaching sbx with
/// one would fail here rather than return.
#[tokio::test]
async fn a_command_sbx_cannot_perform_is_refused_without_running_anything() {
    for command in [ResourceCommand::Restart, ResourceCommand::Resume] {
        let cli = FixtureCli::new([]);

        let error = DockerSandboxWorkspace
            .execute_command(
                &cli,
                &target("sandboxes", "claude-virtui"),
                command,
                ResourceState::Running,
            )
            .await
            .unwrap_err();

        assert_eq!(
            error.message,
            format!("Docker Sandbox cannot {command} sandbox claude-virtui")
        );
    }
}

/// `sbx exec` starts a stopped sandbox before running in it, so a sandbox does
/// not have to be running to carry an Interactive Shell — it only has to be one
/// Virtui recognises. That is a real difference from Docker and Incus, where
/// exec against anything but a running Resource simply fails, and the rule in
/// both cases is the same one: offer the shell exactly where it works.
///
/// A status this workspace cannot read is not a sandbox it can promise `sbx
/// exec` will reach, so those carry none.
#[tokio::test]
async fn every_recognised_sandbox_carries_an_interactive_shell() {
    let cli = FixtureCli::new([(
        ProcessSpec::new("sbx", &["ls", "--json"]),
        success(include_str!(
            "fixtures/docker-sandbox/mixed-state-sandboxes.json"
        )),
    )]);

    let snapshot = DockerSandboxWorkspace
        .refresh(&cli)
        .await
        .expect("the fixture lists sandboxes");

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
        [
            (
                "running-sandbox",
                InteractiveShellProcess::new("sbx", &["exec", "-it", "running-sandbox", "bash"],),
            ),
            (
                "stopped-sandbox",
                InteractiveShellProcess::new("sbx", &["exec", "-it", "stopped-sandbox", "bash"],),
            ),
            (
                "shouting-sandbox",
                InteractiveShellProcess::new("sbx", &["exec", "-it", "shouting-sandbox", "bash"],),
            ),
        ]
    );
}

/// Availability comes from the state the last refresh already reported, so
/// offering the Commands costs no extra sbx query. The fixture answers one
/// listing and panics on anything else, which is what proves it.
#[tokio::test]
async fn command_availability_follows_the_last_refreshed_state() {
    let cli = FixtureCli::new([(
        ProcessSpec::new("sbx", &["ls", "--json"]),
        success(include_str!(
            "fixtures/docker-sandbox/mixed-state-sandboxes.json"
        )),
    )]);

    let snapshot = DockerSandboxWorkspace
        .refresh(&cli)
        .await
        .expect("the fixture lists sandboxes");

    assert_eq!(
        snapshot
            .targets()
            .map(|(_, resource)| resource)
            .map(|resource| (resource.name.as_str(), resource.available_commands.clone()))
            .collect::<Vec<_>>(),
        [
            (
                "running-sandbox",
                vec![ResourceCommand::Stop, ResourceCommand::Delete]
            ),
            (
                "stopped-sandbox",
                vec![ResourceCommand::Start, ResourceCommand::Delete]
            ),
            (
                "shouting-sandbox",
                vec![ResourceCommand::Stop, ResourceCommand::Delete]
            ),
            // Not settled and stopped, so neither starting nor stopping
            // reliably applies. Deleting always does.
            ("starting-sandbox", vec![ResourceCommand::Delete]),
            ("unrecognised-sandbox", vec![ResourceCommand::Delete]),
        ]
    );
}

#[tokio::test]
async fn discovered_docker_sandbox_renders_target_environment_and_sandboxes() {
    let cli = FixtureCli::new([
        (
            ProcessSpec::new("sbx", &["version"]),
            success("sbx version: v0.37.0 8b65b864b0d49c29f05a55170d6b5eea4c0d11e7\n"),
        ),
        (
            ProcessSpec::new("sbx", &["ls", "--json"]),
            success(include_str!("fixtures/docker-sandbox/sandboxes.json")),
        ),
        (
            ProcessSpec::new("sbx", &["ls", "--json"]),
            success(include_str!("fixtures/docker-sandbox/sandboxes.json")),
        ),
    ]);
    let sandboxes = DockerSandboxWorkspace;

    let discovered = sandboxes
        .discover(&cli)
        .await
        .expect("the fixture represents an installed sbx");
    let mut app = App::new();
    let request = app
        .update(discovered.into_event())
        .into_iter()
        .next()
        .expect("discovery requests the first workspace refresh");
    app.update(refresh_completed(request, sandboxes.refresh(&cli).await));

    let screen = render_to_text(app.state(), 100, 24);
    assert!(screen.contains("Docker Sandbox"), "{screen}");
    assert!(screen.contains("[ Docker Sandbox · local ]"), "{screen}");
    assert!(screen.contains("Sandboxes"), "{screen}");
    assert!(screen.contains("claude-virtui"), "{screen}");
    assert!(screen.contains("running"), "{screen}");
    assert!(screen.contains("shell-dotfiles"), "{screen}");
    assert!(screen.contains("stopped"), "{screen}");
    assert!(screen.contains("Agent: claude"), "{screen}");
}

/// A user who has never created a sandbox sees an empty workspace, which must
/// not be confused with an sbx that could not be reached.
#[tokio::test]
async fn reachable_docker_sandbox_without_sandboxes_renders_a_distinct_empty_state() {
    let empty = r#"{"sandboxes": []}"#;
    let cli = FixtureCli::new([
        (
            ProcessSpec::new("sbx", &["version"]),
            success("sbx version: v0.37.0 8b65b864b0d49c29f05a55170d6b5eea4c0d11e7\n"),
        ),
        (ProcessSpec::new("sbx", &["ls", "--json"]), success(empty)),
        (ProcessSpec::new("sbx", &["ls", "--json"]), success(empty)),
    ]);
    let sandboxes = DockerSandboxWorkspace;

    let discovered = sandboxes.discover(&cli).await.expect("sbx is installed");
    let mut app = App::new();
    let request = app
        .update(discovered.into_event())
        .into_iter()
        .next()
        .expect("initial refresh");
    app.update(refresh_completed(request, sandboxes.refresh(&cli).await));

    let screen = render_to_text(app.state(), 100, 24);
    assert!(screen.contains("[ Docker Sandbox · local ]"), "{screen}");
    assert!(
        screen.contains("No Docker Sandbox sandboxes found"),
        "{screen}"
    );
    assert!(!screen.contains("unavailable"), "{screen}");
}

/// A Provider Workspace nothing constructs is a Provider the user never sees,
/// so registration is verified through the runtime that builds them.
#[tokio::test]
async fn runtime_with_builtin_providers_discovers_installed_docker_sandbox() {
    let cli = FixtureCli::new([
        (
            ProcessSpec::new("docker", &["context", "show"]),
            Err(ProcessError::ExecutableNotFound),
        ),
        (
            ProcessSpec::new("incus", &["remote", "get-default"]),
            Err(ProcessError::ExecutableNotFound),
        ),
        (
            ProcessSpec::new("sbx", &["version"]),
            success("sbx version: v0.37.0 8b65b864b0d49c29f05a55170d6b5eea4c0d11e7\n"),
        ),
        (
            ProcessSpec::new("sbx", &["ls", "--json"]),
            success(include_str!("fixtures/docker-sandbox/sandboxes.json")),
        ),
    ]);
    let runtime = ProviderRuntime::with_builtin_providers(Arc::new(cli));

    let discovered = runtime.discover().await;

    assert_eq!(discovered.len(), 1);
    assert_eq!(
        discovered[0].provider().id(),
        &ProviderId::new("docker-sandbox")
    );
    assert_eq!(discovered[0].provider().name(), "Docker Sandbox");
    assert_eq!(discovered[0].provider().target_environment(), &"local");
    assert_eq!(
        discovered[0].provider().version(),
        Some(&ProviderVersion::new("v0.37.0"))
    );
}

/// Closes the loop #29 asks for: an invoked Command, through the confirmation
/// the shell already imposes, out to the exact sbx arguments — rather than
/// calling the workspace directly and assuming the shell would agree.
#[tokio::test]
async fn deleting_a_sandbox_confirms_first_and_then_runs_the_expected_cli_request() {
    let cli = FixtureCli::new([
        (
            ProcessSpec::new("sbx", &["version"]),
            success("sbx version: v0.37.0 8b65b864b0d49c29f05a55170d6b5eea4c0d11e7\n"),
        ),
        (
            ProcessSpec::new("sbx", &["ls", "--json"]),
            success(include_str!("fixtures/docker-sandbox/sandboxes.json")),
        ),
        (
            ProcessSpec::new("sbx", &["ls", "--json"]),
            success(include_str!("fixtures/docker-sandbox/sandboxes.json")),
        ),
    ]);
    let sandboxes = DockerSandboxWorkspace;
    let discovered = sandboxes.discover(&cli).await.expect("sbx is installed");
    let mut app = App::new();
    let request = app
        .update(discovered.into_event())
        .into_iter()
        .next()
        .expect("initial refresh");
    app.update(refresh_completed(request, sandboxes.refresh(&cli).await));

    // Deleting asks first, and issues nothing until the user agrees.
    let pending = app.invoke(Command::Resource(ResourceCommand::Delete));
    assert!(pending.is_empty(), "deletion waits for confirmation");
    let screen = render_to_text(app.state(), 100, 24);
    assert!(screen.contains("Docker Sandbox"), "{screen}");
    assert!(screen.contains("claude-virtui"), "{screen}");

    let ProviderRequest::ExecuteResourceCommand {
        provider_id,
        target,
        command,
        state,
        ..
    } = app
        .invoke(Command::Confirm)
        .into_iter()
        .next()
        .expect("confirming dispatches the deletion")
    else {
        panic!("confirming a deletion executes a Resource Command");
    };
    assert_eq!(provider_id, ProviderId::new("docker-sandbox"));
    assert_eq!(
        target,
        ResourceTarget::new(
            ResourcePanelId::new("sandboxes"),
            ResourceId::new("claude-virtui"),
        )
    );
    assert_eq!(command, ResourceCommand::Delete);

    // The request the shell produced reaches sbx as the arguments #29 names.
    let executing = FixtureCli::new([(
        ProcessSpec::new("sbx", &["rm", "--force", "claude-virtui"]),
        success(""),
    )]);
    sandboxes
        .execute_command(&executing, &target, command, state)
        .await
        .expect("the confirmed deletion succeeds");
}

/// Availability is enforced by the shell, not merely advertised: a Command the
/// Resource never offered issues nothing at all.
#[tokio::test]
async fn starting_an_already_running_sandbox_is_not_offered_and_issues_nothing() {
    let cli = FixtureCli::new([
        (
            ProcessSpec::new("sbx", &["version"]),
            success("sbx version: v0.37.0 8b65b864b0d49c29f05a55170d6b5eea4c0d11e7\n"),
        ),
        (
            ProcessSpec::new("sbx", &["ls", "--json"]),
            success(include_str!("fixtures/docker-sandbox/sandboxes.json")),
        ),
        (
            ProcessSpec::new("sbx", &["ls", "--json"]),
            success(include_str!("fixtures/docker-sandbox/sandboxes.json")),
        ),
    ]);
    let sandboxes = DockerSandboxWorkspace;
    let discovered = sandboxes.discover(&cli).await.expect("sbx is installed");
    let mut app = App::new();
    let request = app
        .update(discovered.into_event())
        .into_iter()
        .next()
        .expect("initial refresh");
    app.update(refresh_completed(request, sandboxes.refresh(&cli).await));

    // claude-virtui is running, so Start was never among its Commands.
    let requests = app.invoke(Command::Resource(ResourceCommand::Start));

    assert!(
        requests.is_empty(),
        "a Command the Resource never offered issues nothing"
    );
}

#[tokio::test]
async fn a_failed_command_reports_what_sbx_wrote_to_stderr() {
    let cli = FixtureCli::new([(
        ProcessSpec::new("sbx", &["stop", "claude-virtui"]),
        failure("Error: sandbox 'claude-virtui' not found"),
    )]);

    let error = DockerSandboxWorkspace
        .execute_command(
            &cli,
            &target("sandboxes", "claude-virtui"),
            ResourceCommand::Stop,
            ResourceState::Running,
        )
        .await
        .expect_err("a non-zero exit is never a success");

    assert_eq!(error.message, "Error: sandbox 'claude-virtui' not found");
}

/// An sbx that fails without a word still has to say which Provider, which
/// sandbox, and which Command.
#[tokio::test]
async fn a_silent_command_failure_names_the_provider_command_and_sandbox() {
    let cli = FixtureCli::new([(
        ProcessSpec::new("sbx", &["rm", "--force", "claude-virtui"]),
        Err(ProcessError::Exited(ProcessFailure {
            exit_code: Some(1),
            stdout: String::new(),
            stderr: String::new(),
        })),
    )]);

    let error = DockerSandboxWorkspace
        .execute_command(
            &cli,
            &target("sandboxes", "claude-virtui"),
            ResourceCommand::Delete,
            ResourceState::Running,
        )
        .await
        .expect_err("a non-zero exit is never a success");

    assert_eq!(
        error.message,
        "Docker Sandbox could not delete sandbox claude-virtui"
    );
}

/// sbx offers no logs, stats, or console of its own, so the Sandboxes panel
/// declares only the one view it can actually answer rather than borrowing
/// Docker's names for diagnostics it does not have.
#[tokio::test]
async fn the_sandboxes_panel_declares_only_the_info_view() {
    let cli = FixtureCli::new([(
        ProcessSpec::new("sbx", &["ls", "--json"]),
        success(include_str!("fixtures/docker-sandbox/sandboxes.json")),
    )]);

    let snapshot = DockerSandboxWorkspace
        .refresh(&cli)
        .await
        .expect("the fixture lists sandboxes");

    let panel = snapshot.panels.first().expect("a Sandboxes panel");
    assert_eq!(
        panel
            .detail_views
            .iter()
            .map(|view| (view.id.0.as_str(), view.title.as_str()))
            .collect::<Vec<_>>(),
        [("info", "Info")]
    );
}

/// sbx has no per-sandbox inspect command, so Info is the sandbox's own row
/// from `sbx ls --json` — everything sbx knows about it — laid out for reading.
#[tokio::test]
async fn the_info_view_describes_the_selected_sandbox() {
    let cli = FixtureCli::new([(
        ProcessSpec::new("sbx", &["ls", "--json"]),
        success(include_str!("fixtures/docker-sandbox/sandboxes.json")),
    )]);

    let details = DockerSandboxWorkspace
        .load_details(
            &cli,
            &target("sandboxes", "claude-virtui"),
            &DetailViewId::new("info"),
        )
        .await
        .expect("the fixture lists claude-virtui");

    assert_eq!(
        details.lines,
        [
            "Name: claude-virtui",
            "ID: 3f2a1c88-91b4-4d0e-9c77-1e5b0a6d2f43",
            "Agent: claude",
            "Status: running",
            "Workspaces:",
            "  /home/vibebox/projects/virtui",
            "Ports:",
            "  127.0.0.1:32768 -> 9418/tcp",
        ]
    );
}

/// A stopped sandbox publishes nothing, so the Ports section is absent rather
/// than present and empty.
#[tokio::test]
async fn the_info_view_omits_ports_a_sandbox_does_not_publish() {
    let cli = FixtureCli::new([(
        ProcessSpec::new("sbx", &["ls", "--json"]),
        success(include_str!("fixtures/docker-sandbox/sandboxes.json")),
    )]);

    let details = DockerSandboxWorkspace
        .load_details(
            &cli,
            &target("sandboxes", "shell-dotfiles"),
            &DetailViewId::new("info"),
        )
        .await
        .expect("the fixture lists shell-dotfiles");

    assert_eq!(
        details.lines,
        [
            "Name: shell-dotfiles",
            "ID: 64081868-4588-4205-8edd-3a2e8e253a95",
            "Agent: shell",
            "Status: stopped",
            "Workspaces:",
            "  /home/vibebox/dotfiles",
        ]
    );
}

/// A sandbox deleted between the last refresh and opening its Info view is an
/// empty view, not a failure: the panel is about to drop it anyway.
#[tokio::test]
async fn the_info_view_of_a_vanished_sandbox_is_empty_rather_than_broken() {
    let cli = FixtureCli::new([(
        ProcessSpec::new("sbx", &["ls", "--json"]),
        success(include_str!("fixtures/docker-sandbox/sandboxes.json")),
    )]);

    let details = DockerSandboxWorkspace
        .load_details(
            &cli,
            &target("sandboxes", "deleted-since-the-last-refresh"),
            &DetailViewId::new("info"),
        )
        .await
        .expect("a sandbox that is gone is not a provider failure");

    assert!(details.is_empty());
}

/// The fixture panics on any CLI request, so a view resolved to a command
/// would fail here rather than return.
#[tokio::test]
async fn a_view_docker_sandbox_never_declared_is_refused_without_running_anything() {
    let cli = FixtureCli::new([]);

    let error = DockerSandboxWorkspace
        .load_details(
            &cli,
            &target("sandboxes", "claude-virtui"),
            &DetailViewId::new("logs"),
        )
        .await
        .expect_err("Docker Sandbox declares no logs view");

    assert_eq!(
        error.message,
        "Docker Sandbox has no logs view for sandbox claude-virtui"
    );
}

#[tokio::test]
async fn a_failed_info_view_reports_what_sbx_wrote_to_stderr() {
    let cli = FixtureCli::new([(
        ProcessSpec::new("sbx", &["ls", "--json"]),
        failure("Error: sandboxd is not running"),
    )]);

    let error = DockerSandboxWorkspace
        .load_details(
            &cli,
            &target("sandboxes", "claude-virtui"),
            &DetailViewId::new("info"),
        )
        .await
        .expect_err("a non-zero exit is never loaded details");

    assert_eq!(
        error.message,
        "Error: sandboxd is not running. Run `sbx ls` to verify access to the current Target Environment."
    );
}

#[tokio::test]
async fn a_failed_sandbox_refresh_identifies_the_command_and_target() {
    let cli = FixtureCli::new([(
        ProcessSpec::new("sbx", &["ls", "--json"]),
        failure("Error: sandboxd is not running"),
    )]);

    let error = DockerSandboxWorkspace
        .refresh(&cli)
        .await
        .expect_err("a non-zero exit is never a snapshot");

    assert_eq!(
        error.message,
        "Error: sandboxd is not running. Run `sbx ls` to verify access to the current Target Environment."
    );
}

/// Only a Provider that ran and refused has a Target Environment worth
/// verifying. Telling the user to run a CLI just reported absent, or one that
/// could not be started at all, sends them at a command that cannot answer.
#[tokio::test]
async fn an_sbx_that_is_gone_is_not_reported_by_asking_the_user_to_run_it() {
    for (error, expected) in [
        (
            ProcessError::ExecutableNotFound,
            "Docker Sandbox CLI is no longer available",
        ),
        (
            ProcessError::SpawnFailed("permission denied".to_owned()),
            "Docker Sandbox CLI could not be started: permission denied",
        ),
    ] {
        let cli = FixtureCli::new([(ProcessSpec::new("sbx", &["ls", "--json"]), Err(error))]);

        let failure = DockerSandboxWorkspace
            .refresh(&cli)
            .await
            .expect_err("an unrunnable CLI is never a snapshot");

        assert_eq!(failure.message, expected);
    }
}

#[tokio::test]
async fn a_silent_sandbox_refresh_failure_still_explains_itself() {
    let cli = FixtureCli::new([(
        ProcessSpec::new("sbx", &["ls", "--json"]),
        Err(ProcessError::Exited(ProcessFailure {
            exit_code: Some(1),
            stdout: String::new(),
            stderr: String::new(),
        })),
    )]);

    let error = DockerSandboxWorkspace
        .refresh(&cli)
        .await
        .expect_err("a non-zero exit is never a snapshot");

    assert_eq!(
        error.message,
        "Docker Sandbox could not list sandboxes. Run `sbx ls` to verify access to the current Target Environment."
    );
}

/// Truncated output is a failure the user can see, not an empty workspace that
/// silently hides every sandbox they own.
#[tokio::test]
async fn malformed_sandbox_output_is_reported_rather_than_read_as_empty() {
    let cli = FixtureCli::new([(
        ProcessSpec::new("sbx", &["ls", "--json"]),
        success(include_str!(
            "fixtures/docker-sandbox/malformed-sandboxes.json"
        )),
    )]);

    let error = DockerSandboxWorkspace
        .refresh(&cli)
        .await
        .expect_err("truncated JSON is never a snapshot");

    assert!(
        error
            .message
            .starts_with("Docker Sandbox returned malformed data:"),
        "the message names the provider and the problem: {}",
        error.message
    );
    assert!(
        error
            .message
            .ends_with(". Run `sbx ls` to verify access to the current Target Environment."),
        "the message stays actionable: {}",
        error.message
    );
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

    assert_eq!(discovered.provider().name(), "Docker Sandbox");
    assert_eq!(discovered.provider().target_environment(), &"unavailable");
    assert_eq!(
        discovered
            .error()
            .expect("an unusable provider explains itself")
            .message,
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
            .error()
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
            .error()
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
