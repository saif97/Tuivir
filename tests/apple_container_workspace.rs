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
