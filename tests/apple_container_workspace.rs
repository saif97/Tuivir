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
