use virtui::{
    application::{App, Command, InteractiveShellProcess, Resource, ResourceCommand},
    domain::{DetailViewId, ResourceId, ResourcePanelId, ResourceState, ResourceTarget},
    infrastructure::{
        process::{ProcessError, ProcessOutput, ProcessSpec},
        provider::{DockerWorkspace, ProviderWorkspace},
    },
    presentation::render_to_text,
};

mod common;
use common::{
    FixtureCli, command_completed, failure, failure_on_stdout, refresh_completed, resource_target,
    silent_failure, success,
};

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

fn image_ls() -> ProcessSpec {
    ProcessSpec::new(
        "docker",
        &["image", "ls", "--no-trunc", "--format", "{{json .}}"],
    )
}

fn volume_ls() -> ProcessSpec {
    ProcessSpec::new("docker", &["volume", "ls", "--format", "{{json .}}"])
}

fn empty_volume_ls() -> (ProcessSpec, Result<ProcessOutput, ProcessError>) {
    (volume_ls(), success(""))
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
            &resource_target("containers", "container-a"),
            ResourceCommand::Restart,
            Some(ResourceState::Running),
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
            &resource_target("containers", "container-a"),
            ResourceCommand::Start,
            Some(ResourceState::Stopped),
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
            &resource_target("containers", "container-a"),
            ResourceCommand::Stop,
            Some(ResourceState::Running),
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
            &resource_target("containers", "container-a"),
            ResourceCommand::Resume,
            Some(ResourceState::Paused),
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
            &resource_target("containers", "container-a"),
            ResourceCommand::Delete,
            Some(ResourceState::Stopped),
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
            &resource_target("containers", "container-a"),
            ResourceCommand::Delete,
            Some(ResourceState::Running),
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
                &resource_target("containers", "container-a"),
                ResourceCommand::Delete,
                Some(state),
            )
            .await
            .unwrap_or_else(|error| panic!("force delete from {state:?} succeeds: {error:?}"));
    }
}

#[tokio::test]
async fn docker_maps_every_container_state_into_the_shared_vocabulary() {
    let cli = FixtureCli::new([
        (
            container_ls(),
            success(include_str!("fixtures/docker/mixed-state-containers.jsonl")),
        ),
        (image_ls(), success("")),
        empty_volume_ls(),
    ]);

    let snapshot = DockerWorkspace
        .refresh(&cli)
        .await
        .expect("fixture lists containers");

    let states = snapshot
        .targets()
        .map(|(_, resource)| resource)
        .map(|resource| {
            (
                resource.id.0.as_str(),
                resource.state.expect("containers have lifecycle state"),
            )
        })
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

/// `docker exec` attaches only to a running container, so no other state may
/// carry the Interactive Shell that runs it. A container without one offers the
/// operation nowhere, which is how an unsupported shell stays absent.
#[tokio::test]
async fn only_a_running_container_carries_an_interactive_shell() {
    let cli = FixtureCli::new([
        (
            container_ls(),
            success(include_str!("fixtures/docker/mixed-state-containers.jsonl")),
        ),
        (image_ls(), success("")),
        empty_volume_ls(),
    ]);

    let snapshot = DockerWorkspace
        .refresh(&cli)
        .await
        .expect("fixture lists containers");

    let shells = snapshot
        .resources()
        .filter_map(|resource| {
            resource
                .shell
                .as_ref()
                .map(|shell| (resource.id.0.as_str(), shell.clone()))
        })
        .collect::<Vec<_>>();
    assert_eq!(
        shells,
        [(
            "container-running",
            InteractiveShellProcess::new(
                "docker",
                &["exec", "-it", "container-running", "/bin/sh"],
            ),
        )]
    );
}

/// `docker container unpause` succeeds only against a paused container, so no
/// other state may offer the Command that runs it.
#[tokio::test]
async fn only_a_paused_container_offers_the_resume_command() {
    let cli = FixtureCli::new([
        (
            container_ls(),
            success(include_str!("fixtures/docker/mixed-state-containers.jsonl")),
        ),
        (image_ls(), success("")),
        empty_volume_ls(),
    ]);

    let snapshot = DockerWorkspace
        .refresh(&cli)
        .await
        .expect("fixture lists containers");

    let resumable = snapshot
        .targets()
        .map(|(_, resource)| resource)
        .filter(|resource| {
            resource
                .available_commands
                .contains(&ResourceCommand::Resume)
        })
        .map(|resource| resource.id.0.as_str())
        .collect::<Vec<_>>();
    assert_eq!(resumable, ["container-paused"]);

    let paused = snapshot
        .targets()
        .map(|(_, resource)| resource)
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
    let cli = FixtureCli::new([
        (
            container_ls(),
            success(include_str!("fixtures/docker/containers.jsonl")),
        ),
        (image_ls(), success("")),
        empty_volume_ls(),
    ]);

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

#[tokio::test]
async fn docker_declares_images_after_containers_with_native_stateless_rows() {
    let cli = FixtureCli::new([
        (
            container_ls(),
            success(include_str!("fixtures/docker/containers.jsonl")),
        ),
        (
            image_ls(),
            success(include_str!("fixtures/docker/images.jsonl")),
        ),
        empty_volume_ls(),
    ]);

    let snapshot = DockerWorkspace
        .refresh(&cli)
        .await
        .expect("fixtures list Docker resources");

    assert_eq!(
        snapshot
            .panels
            .iter()
            .map(|panel| (panel.id.0.as_str(), panel.title.as_str()))
            .collect::<Vec<_>>(),
        [
            ("containers", "Containers"),
            ("images", "Images"),
            ("volumes", "Volumes"),
        ]
    );
    let images = &snapshot.panels[1];
    assert_eq!(
        images
            .detail_views
            .iter()
            .map(|view| (view.id.0.as_str(), view.title.as_str()))
            .collect::<Vec<_>>(),
        [("inspect", "Inspect")]
    );
    let nginx = &images.resources[0];
    assert_eq!(nginx.name, "nginx:1.27");
    assert_eq!(nginx.state, None, "an image has no lifecycle state");
    assert_eq!(nginx.status, None);
    assert!(nginx.available_commands.is_empty());
    assert_eq!(
        nginx.fields,
        [
            ("Repository", "nginx".to_owned()),
            ("Tag", "1.27".to_owned()),
            (
                "Identity",
                "sha256:1111111111111111111111111111111111111111111111111111111111111111"
                    .to_owned()
            ),
            ("Size", "192MB".to_owned()),
        ]
    );
}

#[tokio::test]
async fn docker_declares_volumes_after_images_with_native_stateless_rows() {
    let cli = FixtureCli::new([
        (container_ls(), success("")),
        (image_ls(), success("")),
        (
            volume_ls(),
            success(include_str!("fixtures/docker/volumes.jsonl")),
        ),
    ]);

    let snapshot = DockerWorkspace
        .refresh(&cli)
        .await
        .expect("fixture lists Docker resources");

    assert_eq!(
        snapshot
            .panels
            .iter()
            .map(|panel| (panel.id.0.as_str(), panel.title.as_str()))
            .collect::<Vec<_>>(),
        [
            ("containers", "Containers"),
            ("images", "Images"),
            ("volumes", "Volumes"),
        ]
    );
    let volumes = &snapshot.panels[2];
    assert_eq!(
        volumes
            .detail_views
            .iter()
            .map(|view| (view.id.0.as_str(), view.title.as_str()))
            .collect::<Vec<_>>(),
        [("inspect", "Inspect")]
    );
    assert_eq!(
        volumes
            .resources
            .iter()
            .map(|volume| (
                volume.id.0.as_str(),
                volume.name.as_str(),
                volume.secondary_text.as_deref(),
                volume.state,
                volume.status.as_deref(),
                volume.available_commands,
            ))
            .collect::<Vec<_>>(),
        [
            (
                "anonymous-volume",
                "anonymous-volume",
                Some("local"),
                None,
                None,
                &[ResourceCommand::Delete][..],
            ),
            (
                "named-volume",
                "named-volume",
                Some("nfs"),
                None,
                None,
                &[ResourceCommand::Delete][..],
            ),
        ]
    );
}

/// Docker lists one row per tag, so a twice-tagged image repeats its digest.
/// Identifying rows by the digest would make two Resources indistinguishable:
/// both would draw as selected, and selection could never move past the first.
#[tokio::test]
async fn images_sharing_a_digest_are_distinct_resources() {
    let cli = FixtureCli::new([
        (container_ls(), success("")),
        (
            image_ls(),
            success(include_str!("fixtures/docker/images.jsonl")),
        ),
        empty_volume_ls(),
    ]);

    let snapshot = DockerWorkspace
        .refresh(&cli)
        .await
        .expect("fixtures list Docker images");

    let images = &snapshot.panels[1];
    let ids = images
        .resources
        .iter()
        .map(|resource| resource.id.0.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        ids,
        [
            "nginx:1.27",
            "nginx:latest",
            "example/worker:latest",
            // An untagged image has only its digest to be known by.
            "sha256:3333333333333333333333333333333333333333333333333333333333333333",
        ]
    );
    let unique = ids.iter().collect::<std::collections::HashSet<_>>();
    assert_eq!(unique.len(), ids.len(), "every image row is addressable");
    let identity = |resource: &Resource| {
        resource
            .fields
            .iter()
            .find(|(label, _)| *label == "Identity")
            .map(|(_, value)| value.clone())
            .expect("an image reports its Identity")
    };
    let (first, second) = (&images.resources[0], &images.resources[1]);
    assert_eq!(
        identity(first),
        identity(second),
        "the two tags really are one image"
    );
    assert_ne!(first.id, second.id, "yet they are separate Resources");
}

#[tokio::test]
async fn docker_keeps_an_empty_images_panel() {
    let cli = FixtureCli::new([
        (container_ls(), success("")),
        (image_ls(), success("\n")),
        empty_volume_ls(),
    ]);

    let snapshot = DockerWorkspace
        .refresh(&cli)
        .await
        .expect("empty image output is valid");

    assert_eq!(snapshot.panels.len(), 3);
    assert!(snapshot.panels[1].resources.is_empty());
    assert_eq!(snapshot.panels[1].id, ResourcePanelId::new("images"));
    assert!(snapshot.panels[2].resources.is_empty());
    assert_eq!(snapshot.panels[2].id, ResourcePanelId::new("volumes"));
}

#[tokio::test]
async fn malformed_volume_output_becomes_an_actionable_workspace_error() {
    let cli = FixtureCli::new([
        (container_ls(), success("")),
        (image_ls(), success("")),
        (volume_ls(), success("not a Docker volume row\n")),
    ]);

    let error = DockerWorkspace
        .refresh(&cli)
        .await
        .expect_err("malformed volume output cannot become Resources");

    assert!(error.message.contains("malformed volume data"));
    assert!(error.message.contains("docker volume ls"));
}

#[tokio::test]
async fn malformed_image_output_becomes_an_actionable_workspace_error() {
    let cli = FixtureCli::new([
        (container_ls(), success("")),
        (
            image_ls(),
            success(include_str!("fixtures/docker/malformed-images.jsonl")),
        ),
        empty_volume_ls(),
    ]);

    let error = DockerWorkspace
        .refresh(&cli)
        .await
        .expect_err("malformed image output cannot become Resources");

    assert!(error.message.contains("malformed image data"));
    assert!(error.message.contains("docker image ls"));
}

#[tokio::test]
async fn image_inspect_is_routed_through_the_images_panel() {
    let identity = "sha256:1111111111111111111111111111111111111111111111111111111111111111";
    let cli = FixtureCli::new([(
        ProcessSpec::new("docker", &["image", "inspect", identity]),
        success("[{\"Id\":\"sha256:111\"}]\n"),
    )]);

    let details = DockerWorkspace
        .load_details(
            &cli,
            &ResourceTarget::new(ResourcePanelId::new("images"), ResourceId::new(identity)),
            &DetailViewId::new("inspect"),
        )
        .await
        .expect("Docker image inspect loads");

    assert_eq!(details.lines, ["[{\"Id\":\"sha256:111\"}]"]);
}

#[tokio::test]
async fn volume_inspect_and_plain_delete_are_routed_through_the_volumes_panel() {
    let cli = FixtureCli::new([
        (
            ProcessSpec::new("docker", &["volume", "inspect", "named-volume"]),
            success("[{\"Name\":\"named-volume\"}]\n"),
        ),
        (
            ProcessSpec::new("docker", &["volume", "rm", "named-volume"]),
            success("named-volume\n"),
        ),
    ]);
    let target = resource_target("volumes", "named-volume");

    let details = DockerWorkspace
        .load_details(&cli, &target, &DetailViewId::new("inspect"))
        .await
        .expect("Docker volume inspect loads");
    assert_eq!(details.lines, ["[{\"Name\":\"named-volume\"}]"]);

    DockerWorkspace
        .execute_command(&cli, &target, ResourceCommand::Delete, None)
        .await
        .expect("Docker removes a stateless volume without force");
}

#[tokio::test]
async fn deleting_a_volume_through_the_workspace_refreshes_its_panel() {
    let cli = FixtureCli::new([
        (
            ProcessSpec::new("docker", &["context", "show"]),
            success("default\n"),
        ),
        (container_ls(), success("")),
        (image_ls(), success("")),
        (
            volume_ls(),
            success(include_str!("fixtures/docker/volumes.jsonl")),
        ),
        (
            ProcessSpec::new("docker", &["volume", "rm", "anonymous-volume"]),
            success("anonymous-volume\n"),
        ),
        (container_ls(), success("")),
        (image_ls(), success("")),
        (volume_ls(), success("")),
    ]);
    let docker = DockerWorkspace;
    let discovered = docker.discover(&cli).await.expect("Docker is installed");
    let mut app = App::new();
    let refresh = app
        .update(discovered.into_event())
        .into_iter()
        .next()
        .expect("initial refresh");
    app.update(refresh_completed(refresh, docker.refresh(&cli).await));
    assert!(render_to_text(app.state(), 100, 24).contains("anonymous-volume · local"));

    app.invoke(Command::FocusResourcePanel(2));
    assert!(
        app.invoke(Command::Resource(ResourceCommand::Delete))
            .is_empty()
    );
    let confirmation = render_to_text(app.state(), 100, 24);
    assert!(confirmation.contains("anonymous-volume"));
    assert!(confirmation.contains("permanently removed"));

    let deletion = app
        .invoke(Command::Confirm)
        .into_iter()
        .next()
        .expect("confirmation dispatches deletion");
    let (request_id, provider_id, target, command, state) = match deletion {
        virtui::application::ProviderRequest::ExecuteResourceCommand {
            request_id,
            provider_id,
            target,
            command,
            state,
        } => (request_id, provider_id, target, command, state),
        other => panic!("expected deletion request, got {other:?}"),
    };
    docker
        .execute_command(&cli, &target, command, state)
        .await
        .expect("volume deletion succeeds");
    let follow_up = app.update(command_completed(
        virtui::application::ProviderRequest::ExecuteResourceCommand {
            request_id,
            provider_id,
            target,
            command,
            state,
        },
        Ok(()),
    ));
    let refresh = follow_up
        .into_iter()
        .next()
        .expect("deletion refreshes workspace");
    app.update(refresh_completed(refresh, docker.refresh(&cli).await));
    assert!(!render_to_text(app.state(), 100, 24).contains("anonymous-volume"));
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
                &resource_target("containers", "container-a"),
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
            &resource_target("containers", "container-a"),
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
            &resource_target("containers", "container-a"),
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
            &resource_target("containers", "container-a"),
            &DetailViewId::new("inspect"),
        )
        .await
        .expect_err("a non-zero exit is never loaded details");

    assert_eq!(error.message, "Error: No such container: container-a");
}

/// Provider output is displayed, not reformatted, so structure the Provider
/// laid out — indentation included — survives into the panel.
#[tokio::test]
async fn inspect_output_reaches_the_panel_line_for_line() {
    let cli = FixtureCli::new([(
        ProcessSpec::new("docker", &["container", "inspect", "container-a"]),
        success(include_str!("fixtures/docker/container-inspect.json")),
    )]);

    let details = DockerWorkspace
        .load_details(
            &cli,
            &resource_target("containers", "container-a"),
            &DetailViewId::new("inspect"),
        )
        .await
        .expect("fixture inspects the container");

    assert_eq!(details.lines.len(), 26);
    assert_eq!(details.lines.first().map(String::as_str), Some("["));
    assert_eq!(details.lines.last().map(String::as_str), Some("]"));
    assert!(
        details
            .lines
            .contains(&"            \"Status\": \"running\",".to_owned()),
        "indentation is preserved: {:?}",
        details.lines
    );
}

#[tokio::test]
async fn a_detail_view_docker_never_declared_is_refused_without_running_anything() {
    // The fixture panics on any CLI request, so a view resolved to a command
    // would fail here rather than return.
    let cli = FixtureCli::new([]);

    let error = DockerWorkspace
        .load_details(
            &cli,
            &resource_target("containers", "container-a"),
            &DetailViewId::new("processes"),
        )
        .await
        .expect_err("Docker declares no processes view");

    assert_eq!(
        error.message,
        "Docker has no processes view for container container-a"
    );
}

#[tokio::test]
async fn a_detail_view_loaded_without_the_docker_cli_names_docker() {
    let cli = FixtureCli::new([(
        ProcessSpec::new(
            "docker",
            &["container", "logs", "--tail", "200", "container-a"],
        ),
        Err(ProcessError::ExecutableNotFound),
    )]);

    let error = DockerWorkspace
        .load_details(
            &cli,
            &resource_target("containers", "container-a"),
            &DetailViewId::new("logs"),
        )
        .await
        .expect_err("a missing CLI is never loaded details");

    assert_eq!(error.message, "Docker CLI is no longer available");
}

#[tokio::test]
async fn a_silent_detail_failure_names_the_view_and_container() {
    let cli = FixtureCli::new([(
        ProcessSpec::new(
            "docker",
            &["container", "stats", "--no-stream", "container-a"],
        ),
        silent_failure(125),
    )]);

    let error = DockerWorkspace
        .load_details(
            &cli,
            &resource_target("containers", "container-a"),
            &DetailViewId::new("stats"),
        )
        .await
        .expect_err("a non-zero exit is never loaded details");

    assert_eq!(
        error.message,
        "Docker could not load stats for container container-a"
    );
}

#[tokio::test]
async fn a_silent_command_failure_names_the_operation_and_container() {
    let cli = FixtureCli::new([(
        ProcessSpec::new("docker", &["container", "restart", "container-a"]),
        silent_failure(125),
    )]);

    let error = DockerWorkspace
        .execute_command(
            &cli,
            &resource_target("containers", "container-a"),
            ResourceCommand::Restart,
            Some(ResourceState::Running),
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
            &resource_target("containers", "container-a"),
            ResourceCommand::Delete,
            Some(ResourceState::Stopped),
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
        (image_ls(), success("")),
        empty_volume_ls(),
    ]);
    let docker = DockerWorkspace;

    let discovered = docker
        .discover(&cli)
        .await
        .expect("the fixture represents an installed Docker CLI");
    let mut app = App::new();
    let requests = app.update(discovered.into_event());
    let request = requests
        .into_iter()
        .next()
        .expect("discovery requests the first workspace refresh");

    let snapshot = docker.refresh(&cli).await;
    app.update(refresh_completed(request, snapshot));

    let screen = render_to_text(app.state(), 100, 24);
    assert!(screen.contains("Docker"));
    assert!(screen.contains("[1] Docker"));
    assert!(screen.contains("Target: desktop-linux"));
    assert!(screen.contains("Containers"), "rendered screen:\n{screen}");
    assert!(screen.contains("api"));
    assert!(screen.contains("nginx:1.27"));
    assert!(screen.contains("● api"));
    assert!(screen.contains("worker"));
    assert!(screen.contains("○ worker"));
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
        (image_ls(), success("")),
        empty_volume_ls(),
    ]);
    let docker = DockerWorkspace;
    let discovered = docker.discover(&cli).await.expect("Docker is installed");
    let mut app = App::new();
    let request = app
        .update(discovered.into_event())
        .into_iter()
        .next()
        .expect("initial refresh");
    app.update(refresh_completed(request, docker.refresh(&cli).await));

    let screen = render_to_text(app.state(), 100, 24);
    assert!(screen.contains("[1] Docker"));
    assert!(screen.contains("Target: colima"));
    assert!(screen.contains("No resources"));
    assert!(!screen.contains("No Docker containers found"));
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

    let requests = app.update(discovered.into_event());

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

    let error = discovered.error().expect("the provider exposes its error");
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
        silent_failure(125),
    )]);

    let discovered = DockerWorkspace
        .discover(&cli)
        .await
        .expect("Docker is installed");

    let error = discovered.error().expect("the provider exposes its error");
    assert!(
        error
            .message
            .contains("Docker could not report its current context"),
        "workspace error: {}",
        error.message
    );
    assert_eq!(discovered.provider().target_environment(), None);
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
        .update(discovered.into_event())
        .into_iter()
        .next()
        .expect("initial refresh");
    app.update(refresh_completed(request, docker.refresh(&cli).await));

    let screen = render_to_text(app.state(), 140, 24);
    assert!(screen.contains("[1] Docker"));
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
    let cli = FixtureCli::new([
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
            success(include_str!("fixtures/docker/malformed-containers.jsonl")),
        ),
        (image_ls(), success("")),
        empty_volume_ls(),
    ]);
    let docker = DockerWorkspace;

    let error = docker
        .refresh(&cli)
        .await
        .expect_err("fixture is malformed");

    assert!(error.message.contains("malformed container data"));
    assert!(error.message.contains("docker container ls"));
}
