use tuivir::{
    domain::{ProviderId, ProviderVersion},
    infrastructure::{
        process::{ProcessError, ProcessSpec},
        provider::{AppleContainerWorkspace, ProviderWorkspace},
    },
};

mod common;
use common::{FixtureCli, success};

const VERSION: &[&str] = &["system", "version", "--format", "json"];

#[tokio::test]
async fn discovery_distinguishes_missing_cli_from_installed_cli_without_a_server() {
    let absent = FixtureCli::new([(
        ProcessSpec::new("container", VERSION),
        Err(ProcessError::ExecutableNotFound),
    )]);
    assert!(AppleContainerWorkspace.discover(&absent).await.is_none());
    let cli = FixtureCli::new([(
        ProcessSpec::new("container", VERSION),
        success(include_str!("fixtures/apple-container/version.json")),
    )]);
    let discovery = AppleContainerWorkspace.discover(&cli).await.unwrap();
    assert_eq!(
        discovery.provider().id(),
        &ProviderId::new("apple-container")
    );
    assert_eq!(discovery.provider().name(), "Apple Container");
    assert_eq!(
        discovery.provider().version(),
        Some(&ProviderVersion::new("1.5.0"))
    );
    assert_eq!(discovery.provider().target_environment(), None);
    assert_eq!(discovery.error(), None);
    cli.assert_exhausted();
}

const CONTAINERS: &[&str] = &["list", "--all", "--format", "json"];
const IMAGES: &[&str] = &["image", "list", "--format", "json"];
const NETWORKS: &[&str] = &["network", "list", "--format", "json"];
const VOLUMES: &[&str] = &["volume", "list", "--format", "json"];

fn listings(containers: &str, images: &str, networks: &str, volumes: &str) -> FixtureCli {
    FixtureCli::new([
        (
            ProcessSpec::new("container", CONTAINERS),
            success(containers),
        ),
        (ProcessSpec::new("container", IMAGES), success(images)),
        (ProcessSpec::new("container", NETWORKS), success(networks)),
        (ProcessSpec::new("container", VOLUMES), success(volumes)),
    ])
}

#[tokio::test]
async fn empty_native_panels_remain_visible_in_provider_order() {
    let cli = listings("[]", "[]", "[]", "[]");
    let snapshot = AppleContainerWorkspace.refresh(&cli).await.unwrap();
    assert_eq!(
        snapshot
            .panels
            .iter()
            .map(|p| (p.id.0.as_str(), p.title.as_str(), p.resources.len()))
            .collect::<Vec<_>>(),
        [
            ("containers", "Containers", 0),
            ("images", "Images", 0),
            ("networks", "Networks", 0),
            ("volumes", "Volumes", 0),
        ]
    );
    cli.assert_exhausted();
}

#[tokio::test]
async fn native_container_states_control_available_lifecycle_commands() {
    use tuivir::{application::ResourceCommand, domain::ResourceState};
    let cli = listings(
        include_str!("fixtures/apple-container/containers.json"),
        "[]",
        "[]",
        "[]",
    );
    let snapshot = AppleContainerWorkspace.refresh(&cli).await.unwrap();
    let resources = &snapshot.panels[0].resources;
    assert_eq!(resources.len(), 4);
    for (name, state, commands) in [
        (
            "web",
            ResourceState::Running,
            &[
                ResourceCommand::Stop,
                ResourceCommand::Restart,
                ResourceCommand::Delete,
            ][..],
        ),
        (
            "idle",
            ResourceState::Stopped,
            &[ResourceCommand::Start, ResourceCommand::Delete][..],
        ),
        (
            "shutting-down",
            ResourceState::Transitioning,
            &[ResourceCommand::Delete][..],
        ),
        (
            "future",
            ResourceState::Unknown,
            &[ResourceCommand::Delete][..],
        ),
    ] {
        let resource = resources.iter().find(|r| r.id.0 == name).unwrap();
        assert_eq!(resource.name, name);
        assert_eq!(resource.state, Some(state));
        assert_eq!(resource.available_commands, commands);
        assert!(
            resource
                .fields
                .contains(&("Image", "docker.io/library/alpine:3.23".into()))
        );
        assert!(resource.fields.contains(&("CPUs", "2".into())));
        assert!(resource.fields.contains(&("Memory", "1024 MiB".into())));
    }
    assert_eq!(resources[3].status.as_deref(), Some("future-state"));
    cli.assert_exhausted();
}

#[tokio::test]
async fn images_keep_reference_identity_without_a_lifecycle_state() {
    use tuivir::application::ResourceCommand;
    let cli = listings(
        "[]",
        include_str!("fixtures/apple-container/images.json"),
        "[]",
        "[]",
    );
    let snapshot = AppleContainerWorkspace.refresh(&cli).await.unwrap();
    let image = &snapshot.panels[1].resources[0];
    assert_eq!(image.id.0, "docker.io/library/alpine:3.23");
    assert_eq!(image.name, "docker.io/library/alpine:3.23");
    assert_eq!(image.state, None);
    assert_eq!(image.status, None);
    assert_eq!(image.available_commands, &[ResourceCommand::Delete]);
    assert!(image.fields.contains(&("Size", "8 MiB".into())));
    assert!(image.fields.contains(&("Platforms", "linux/arm64".into())));
    cli.assert_exhausted();
}

#[tokio::test]
async fn network_protection_uses_builtin_metadata_instead_of_the_name() {
    use tuivir::application::ResourceCommand;
    let cli = listings(
        "[]",
        "[]",
        include_str!("fixtures/apple-container/networks.json"),
        "[]",
    );
    let snapshot = AppleContainerWorkspace.refresh(&cli).await.unwrap();
    let networks = &snapshot.panels[2].resources;
    assert_eq!(networks.len(), 2);
    assert_eq!(networks[0].name, "system-renamed");
    assert!(networks[0].available_commands.is_empty());
    assert_eq!(networks[1].id.0, "default");
    assert_eq!(networks[1].available_commands, &[ResourceCommand::Delete]);
    for network in networks {
        assert_eq!(network.state, None);
        assert_eq!(network.status, None);
        assert!(
            network
                .fields
                .contains(&("IPv4 Subnet", "192.168.64.0/24".into()))
        );
        assert!(network.fields.contains(&("Mode", "nat".into())));
    }
    cli.assert_exhausted();
}

#[tokio::test]
async fn volumes_are_independently_managed_stateless_resources() {
    use tuivir::application::ResourceCommand;
    let cli = listings(
        "[]",
        "[]",
        "[]",
        include_str!("fixtures/apple-container/volumes.json"),
    );
    let snapshot = AppleContainerWorkspace.refresh(&cli).await.unwrap();
    let volume = &snapshot.panels[3].resources[0];
    assert_eq!(volume.id.0, "data");
    assert_eq!(volume.state, None);
    assert_eq!(volume.status, None);
    assert_eq!(volume.available_commands, &[ResourceCommand::Delete]);
    assert!(volume.fields.contains(&("Driver", "local".into())));
    assert!(volume.fields.contains(&("Format", "ext4".into())));
    assert!(volume.fields.contains(&("Size", "1024 MiB".into())));
    cli.assert_exhausted();
}

#[tokio::test]
async fn builtin_runtime_discovers_apple_container_and_renders_all_empty_panels() {
    use std::sync::Arc;
    use tuivir::{
        application::App, infrastructure::runtime::ProviderRuntime, presentation::render_to_text,
    };
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
            ProcessSpec::new("incus", &["project", "get-current"]),
            Err(ProcessError::ExecutableNotFound),
        ),
        (
            ProcessSpec::new("sbx", &["version"]),
            Err(ProcessError::ExecutableNotFound),
        ),
        (
            ProcessSpec::new("tart", &["--version"]),
            Err(ProcessError::ExecutableNotFound),
        ),
        (
            ProcessSpec::new("container", VERSION),
            success(include_str!("fixtures/apple-container/version.json")),
        ),
        (ProcessSpec::new("container", CONTAINERS), success("[]")),
        (ProcessSpec::new("container", IMAGES), success("[]")),
        (ProcessSpec::new("container", NETWORKS), success("[]")),
        (ProcessSpec::new("container", VOLUMES), success("[]")),
    ]);
    let cli = Arc::new(cli);
    let runtime = ProviderRuntime::with_builtin_providers(cli.clone());
    let discoveries = runtime.discover().await;
    assert_eq!(discoveries.len(), 1);
    let mut app = App::new();
    let request =
        common::refresh_request(app.update(discoveries.into_iter().next().unwrap().into_event()));
    let (events, mut completions) = tokio::sync::mpsc::unbounded_channel();
    runtime.dispatch(request, events);
    app.update(
        tokio::time::timeout(std::time::Duration::from_secs(1), completions.recv())
            .await
            .unwrap()
            .unwrap(),
    );
    let screen = render_to_text(app.state(), 120, 36);
    for title in [
        "Apple Container",
        "Containers",
        "Images",
        "Networks",
        "Volumes",
    ] {
        assert!(screen.contains(title), "{screen}");
    }
    for index in 0..4 {
        app.invoke(tuivir::application::Command::FocusResourcePanel(index));
        assert_eq!(
            app.state()
                .active_workspace()
                .unwrap()
                .focused_resource_panel_index(),
            Some(index)
        );
    }
    cli.assert_exhausted();
}

#[tokio::test]
async fn lifecycle_start_and_stop_use_reported_state_and_literal_identifiers() {
    use tuivir::{application::ResourceCommand, domain::ResourceState};
    let name = "--odd name;$(touch nope)";
    let target = common::resource_target("containers", name);
    let cli = FixtureCli::new([
        (
            ProcessSpec::new("container", &["start", "--", name]),
            success(""),
        ),
        (
            ProcessSpec::new("container", &["stop", "--", name]),
            success(""),
        ),
    ]);
    for (command, state) in [
        (ResourceCommand::Start, ResourceState::Stopped),
        (ResourceCommand::Stop, ResourceState::Running),
    ] {
        AppleContainerWorkspace
            .execute_command(&cli, &target, command, Some(state))
            .await
            .unwrap();
    }
    cli.assert_exhausted();
}

#[tokio::test]
async fn restart_stops_then_starts_and_reports_either_failure() {
    use tuivir::{application::ResourceCommand, domain::ResourceState};
    let target = common::resource_target("containers", "web");
    let stop = ProcessSpec::new("container", &["stop", "--", "web"]);
    let start = ProcessSpec::new("container", &["start", "--", "web"]);
    let cli = FixtureCli::new([(stop.clone(), success("")), (start.clone(), success(""))]);
    AppleContainerWorkspace
        .execute_command(
            &cli,
            &target,
            ResourceCommand::Restart,
            Some(ResourceState::Running),
        )
        .await
        .unwrap();
    cli.assert_exhausted();
    let stop_failure = FixtureCli::new([(stop.clone(), common::failure("cannot stop VM"))]);
    let error = AppleContainerWorkspace
        .execute_command(
            &stop_failure,
            &target,
            ResourceCommand::Restart,
            Some(ResourceState::Running),
        )
        .await
        .unwrap_err();
    assert!(error.message.contains("cannot stop VM"));
    assert!(error.message.contains("Restart Stop"));
    stop_failure.assert_exhausted();
    let start_failure = FixtureCli::new([
        (stop, success("")),
        (start, common::failure("cannot start VM")),
    ]);
    let error = AppleContainerWorkspace
        .execute_command(
            &start_failure,
            &target,
            ResourceCommand::Restart,
            Some(ResourceState::Running),
        )
        .await
        .unwrap_err();
    assert!(error.message.contains("cannot start VM"));
    assert!(error.message.contains("Restart Start"));
    start_failure.assert_exhausted();
}

#[tokio::test]
async fn container_delete_uses_force_only_outside_positively_stopped_state() {
    use tuivir::{application::ResourceCommand, domain::ResourceState};
    let name = "--unusual name";
    let target = common::resource_target("containers", name);
    for (state, args) in [
        (ResourceState::Stopped, vec!["delete", "--", name]),
        (
            ResourceState::Running,
            vec!["delete", "--force", "--", name],
        ),
        (
            ResourceState::Transitioning,
            vec!["delete", "--force", "--", name],
        ),
        (
            ResourceState::Unknown,
            vec!["delete", "--force", "--", name],
        ),
        (ResourceState::Paused, vec!["delete", "--force", "--", name]),
        (ResourceState::Broken, vec!["delete", "--force", "--", name]),
    ] {
        let cli = FixtureCli::new([(ProcessSpec::new("container", &args), success(""))]);
        AppleContainerWorkspace
            .execute_command(&cli, &target, ResourceCommand::Delete, Some(state))
            .await
            .unwrap();
        cli.assert_exhausted();
    }
}

#[tokio::test]
async fn network_and_volume_delete_preserve_dependency_refusals() {
    use tuivir::application::ResourceCommand;
    for (panel, kind) in [("networks", "network"), ("volumes", "volume")] {
        let name = "--literal name";
        let target = common::resource_target(panel, name);
        let deletion = ProcessSpec::new("container", &[kind, "delete", "--", name]);
        let cli = FixtureCli::new([(deletion.clone(), success(""))]);
        AppleContainerWorkspace
            .execute_command(&cli, &target, ResourceCommand::Delete, None)
            .await
            .unwrap();
        cli.assert_exhausted();
        let refused = FixtureCli::new([(deletion, common::failure("Resource in use by web"))]);
        let error = AppleContainerWorkspace
            .execute_command(&refused, &target, ResourceCommand::Delete, None)
            .await
            .unwrap_err();
        assert!(error.message.contains("Resource in use by web"));
        refused.assert_exhausted();
    }
}

#[tokio::test]
async fn image_delete_requires_verified_absence_of_the_requested_reference() {
    use tuivir::application::ResourceCommand;
    let name = "docker.io/library/alpine:3.23";
    let target = common::resource_target("images", name);
    let delete = ProcessSpec::new("container", &["image", "delete", "--", name]);
    let inspect = ProcessSpec::new("container", &["image", "inspect", "--", name]);
    let absent = FixtureCli::new([
        (delete.clone(), success("")),
        (
            inspect.clone(),
            common::failure("Error: image not found: docker.io/library/alpine:3.23\n"),
        ),
    ]);
    AppleContainerWorkspace
        .execute_command(&absent, &target, ResourceCommand::Delete, None)
        .await
        .unwrap();
    absent.assert_exhausted();
    for output in ["[]", include_str!("fixtures/apple-container/images.json")] {
        let retained = FixtureCli::new([
            (delete.clone(), success("")),
            (inspect.clone(), success(output)),
        ]);
        let error = AppleContainerWorkspace
            .execute_command(&retained, &target, ResourceCommand::Delete, None)
            .await
            .unwrap_err();
        assert!(error.message.contains("retained"));
        assert!(error.message.contains(name));
        retained.assert_exhausted();
    }
    for verification in [
        common::failure("XPC connection error"),
        common::failure("Error: image not found: another:latest"),
        common::failure(
            "Error: image not found: docker.io/library/alpine:3.23 (cause: permission denied)",
        ),
        success("not JSON"),
    ] {
        let cli = FixtureCli::new([
            (delete.clone(), success("")),
            (inspect.clone(), verification),
        ]);
        let error = AppleContainerWorkspace
            .execute_command(&cli, &target, ResourceCommand::Delete, None)
            .await
            .unwrap_err();
        assert!(
            error.message.contains("could not be verified"),
            "{}",
            error.message
        );
        cli.assert_exhausted();
    }
    let refused = FixtureCli::new([(delete, common::failure("image in use"))]);
    assert!(
        AppleContainerWorkspace
            .execute_command(&refused, &target, ResourceCommand::Delete, None)
            .await
            .unwrap_err()
            .message
            .contains("image in use")
    );
    refused.assert_exhausted();
}

#[tokio::test]
async fn every_resource_kind_offers_its_own_native_inspection() {
    use tuivir::domain::DetailViewId;
    let cli = listings(
        include_str!("fixtures/apple-container/containers.json"),
        include_str!("fixtures/apple-container/images.json"),
        include_str!("fixtures/apple-container/networks.json"),
        include_str!("fixtures/apple-container/volumes.json"),
    );
    let snapshot = AppleContainerWorkspace.refresh(&cli).await.unwrap();
    for (panel, prefix, fixture) in [
        (
            "containers",
            vec!["inspect"],
            include_str!("fixtures/apple-container/containers.json"),
        ),
        (
            "images",
            vec!["image", "inspect"],
            include_str!("fixtures/apple-container/images.json"),
        ),
        (
            "networks",
            vec!["network", "inspect"],
            include_str!("fixtures/apple-container/networks.json"),
        ),
        (
            "volumes",
            vec!["volume", "inspect"],
            include_str!("fixtures/apple-container/volumes.json"),
        ),
    ] {
        let resource_panel = snapshot.panels.iter().find(|p| p.id.0 == panel).unwrap();
        assert!(
            resource_panel
                .detail_views
                .iter()
                .any(|v| v.id.0 == "inspect" && v.title == "Inspect")
        );
        let name = "--literal name";
        let target = common::resource_target(panel, name);
        let mut args = prefix;
        args.extend(["--", name]);
        let cli = FixtureCli::new([(ProcessSpec::new("container", &args), success(fixture))]);
        let details = AppleContainerWorkspace
            .load_details(&cli, &target, &DetailViewId::new("inspect"))
            .await
            .unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&details.lines.join("\n")).unwrap(),
            serde_json::from_str::<serde_json::Value>(fixture).unwrap()
        );
        cli.assert_exhausted();
    }
}

#[tokio::test]
async fn process_and_boot_logs_are_separate_bounded_non_following_views() {
    use tuivir::{domain::DetailViewId, infrastructure::process::ProcessOutput};
    let snapshot = AppleContainerWorkspace
        .refresh(&listings("[]", "[]", "[]", "[]"))
        .await
        .unwrap();
    let views = &snapshot.panels[0].detail_views;
    for (view, title, args, stdout) in [
        (
            "logs",
            "Logs",
            vec!["logs", "-n", "200", "--", "web"],
            "process output\n",
        ),
        (
            "boot-logs",
            "Boot Logs",
            vec!["logs", "--boot", "-n", "200", "--", "web"],
            "VM boot output\n",
        ),
    ] {
        assert!(views.iter().any(|v| v.id.0 == view && v.title == title));
        let target = common::resource_target("containers", "web");
        let spec = ProcessSpec::new("container", &args);
        let cli = FixtureCli::new([(
            spec.clone(),
            Ok(ProcessOutput {
                stdout: stdout.into(),
                stderr: "process stderr\n".into(),
            }),
        )]);
        let details = AppleContainerWorkspace
            .load_details(&cli, &target, &DetailViewId::new(view))
            .await
            .unwrap();
        assert_eq!(details.lines, vec![stdout.trim(), "process stderr"]);
        cli.assert_exhausted();
        let unavailable =
            FixtureCli::new([(spec, common::failure("log file not found: never started"))]);
        assert!(
            AppleContainerWorkspace
                .load_details(&unavailable, &target, &DetailViewId::new(view))
                .await
                .unwrap_err()
                .message
                .contains("never started")
        );
        unavailable.assert_exhausted();
    }
}

#[tokio::test]
async fn finite_stats_preserve_native_metrics_and_explain_empty_samples() {
    use tuivir::domain::DetailViewId;
    let snapshot = AppleContainerWorkspace
        .refresh(&listings("[]", "[]", "[]", "[]"))
        .await
        .unwrap();
    assert_eq!(
        snapshot.panels[0]
            .detail_views
            .iter()
            .map(|v| v.title.as_str())
            .collect::<Vec<_>>(),
        ["Logs", "Boot Logs", "Stats", "Inspect"]
    );
    let target = common::resource_target("containers", "web");
    let spec = ProcessSpec::new(
        "container",
        &["stats", "--no-stream", "--format", "json", "--", "web"],
    );
    let fixture = include_str!("fixtures/apple-container/stats.json");
    let cli = FixtureCli::new([(spec.clone(), success(fixture))]);
    let details = AppleContainerWorkspace
        .load_details(&cli, &target, &DetailViewId::new("stats"))
        .await
        .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&details.lines.join("\n")).unwrap(),
        serde_json::from_str::<serde_json::Value>(fixture).unwrap()
    );
    cli.assert_exhausted();
    let empty = FixtureCli::new([(spec.clone(), success("[]"))]);
    let details = AppleContainerWorkspace
        .load_details(&empty, &target, &DetailViewId::new("stats"))
        .await
        .unwrap();
    assert_eq!(details.lines, ["Statistics unavailable for Container web."]);
    empty.assert_exhausted();
    let failed = FixtureCli::new([(spec, common::failure("stats failed: no such container"))]);
    assert!(
        AppleContainerWorkspace
            .load_details(&failed, &target, &DetailViewId::new("stats"))
            .await
            .unwrap_err()
            .message
            .contains("stats failed: no such container")
    );
    failed.assert_exhausted();
}

#[tokio::test]
async fn only_running_containers_offer_an_interactive_bin_sh_session() {
    use tuivir::application::ResourceShellProcess;
    let cli = listings(
        include_str!("fixtures/apple-container/containers.json"),
        "[]",
        "[]",
        "[]",
    );
    let snapshot = AppleContainerWorkspace.refresh(&cli).await.unwrap();
    let resources = &snapshot.panels[0].resources;
    assert_eq!(
        resources[0].shell,
        Some(ResourceShellProcess::new(
            "container",
            &["exec", "--interactive", "--tty", "--", "web", "/bin/sh"]
        ))
    );
    for resource in &resources[1..] {
        assert_eq!(resource.shell, None);
    }
    cli.assert_exhausted();
}

#[tokio::test]
async fn malformed_native_details_are_errors_instead_of_valid_empty_diagnostics() {
    use tuivir::domain::DetailViewId;
    for (panel, args) in [
        ("containers", vec!["inspect", "--", "web"]),
        ("images", vec!["image", "inspect", "--", "web"]),
        ("networks", vec!["network", "inspect", "--", "web"]),
        ("volumes", vec!["volume", "inspect", "--", "web"]),
    ] {
        for output in ["not JSON", "{}", "[42]", "[{}]"] {
            let cli = FixtureCli::new([(ProcessSpec::new("container", &args), success(output))]);
            let error = AppleContainerWorkspace
                .load_details(
                    &cli,
                    &common::resource_target(panel, "web"),
                    &DetailViewId::new("inspect"),
                )
                .await
                .unwrap_err();
            assert!(error.message.contains("malformed or incompatible data"));
            cli.assert_exhausted();
        }
    }
}

#[tokio::test]
async fn installed_provider_diagnostics_remain_actionable_without_a_version_gate() {
    for response in [
        common::failure("unsupported macOS"),
        success("not JSON"),
        success("[]"),
        success(r#"[{"appName":"container","version":""}]"#),
    ] {
        let cli = FixtureCli::new([(ProcessSpec::new("container", VERSION), response)]);
        let discovery = AppleContainerWorkspace
            .discover(&cli)
            .await
            .expect("installed CLI remains visible");
        assert_eq!(
            discovery.provider().id(),
            &ProviderId::new("apple-container")
        );
        let error = &discovery.error().unwrap().message;
        assert!(error.contains("container system version --format json"));
        assert!(error.contains("Apple Silicon with macOS 26"));
        cli.assert_exhausted();
    }
    let cli = FixtureCli::new([(
        ProcessSpec::new("container", VERSION),
        success(
            r#"[{"appName":"container-apiserver","version":"1.5.0"},{"appName":"container","version":"9.0.0"}]"#,
        ),
    )]);
    let discovery = AppleContainerWorkspace.discover(&cli).await.unwrap();
    assert_eq!(
        discovery.provider().version(),
        Some(&ProviderVersion::new("9.0.0"))
    );
    assert!(discovery.error().is_none());
    cli.assert_exhausted();
}

#[tokio::test]
async fn listing_failures_preserve_native_diagnostics_and_reject_bad_shapes() {
    for (args, payload) in [
        (CONTAINERS, "[{}]"),
        (IMAGES, "[{}]"),
        (NETWORKS, "[{}]"),
        (VOLUMES, "[{}]"),
    ] {
        let mut responses = Vec::new();
        for listing in [CONTAINERS, IMAGES, NETWORKS, VOLUMES] {
            responses.push((
                ProcessSpec::new("container", listing),
                success(if listing == args { payload } else { "[]" }),
            ));
            if listing == args {
                break;
            }
        }
        let cli = FixtureCli::new(responses);
        let error = AppleContainerWorkspace.refresh(&cli).await.unwrap_err();
        assert!(error.message.contains("malformed or incompatible data"));
        cli.assert_exhausted();
    }
    for response in [
        common::failure("XPC connection error: service stopped"),
        common::failure_on_stdout("permission denied"),
        common::silent_failure(1),
        success("not JSON"),
        success("{}"),
        Err(ProcessError::SpawnFailed("host refused execution".into())),
    ] {
        let cli = FixtureCli::new([(ProcessSpec::new("container", CONTAINERS), response)]);
        let error = AppleContainerWorkspace.refresh(&cli).await.unwrap_err();
        assert!(error.message.contains("container system start"));
        assert!(error.message.contains("refresh the Workspace"));
        cli.assert_exhausted();
    }
}
