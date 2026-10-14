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
