use virtui::domain::{Provider, ProviderId, ProviderVersion, TargetEnvironment};

#[test]
fn a_provider_keeps_its_identity_environment_and_version_together() {
    let provider = Provider::new(
        ProviderId::new("docker-sandbox"),
        "Docker Sandbox",
        TargetEnvironment::new("local"),
        Some(ProviderVersion::new("v0.37.0")),
    );

    assert_eq!(provider.id(), &ProviderId::new("docker-sandbox"));
    assert_eq!(provider.name(), "Docker Sandbox");
    assert_eq!(
        provider.target_environment(),
        &TargetEnvironment::new("local")
    );
    assert_eq!(provider.version(), Some(&ProviderVersion::new("v0.37.0")));
}
