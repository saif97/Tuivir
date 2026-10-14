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
