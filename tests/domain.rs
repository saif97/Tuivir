use virtui::domain::{
    DetailViewId, Provider, ProviderId, ProviderVersion, ResourceId, ResourcePanelId,
    ResourceState, ResourceTarget,
};

#[test]
fn a_provider_can_omit_a_target_environment_without_losing_its_version() {
    let provider = Provider::new(
        ProviderId::new("docker-sandbox"),
        "Docker Sandbox",
        None,
        Some(ProviderVersion::new("v0.37.0")),
    );

    assert_eq!(provider.id(), &ProviderId::new("docker-sandbox"));
    assert_eq!(provider.name(), "Docker Sandbox");
    assert_eq!(provider.target_environment(), None);
    assert_eq!(provider.version(), Some(&ProviderVersion::new("v0.37.0")));
}

#[test]
fn a_resource_target_is_one_panel_qualified_domain_identity() {
    let containers = ResourceTarget::new(
        ResourcePanelId::new("containers"),
        ResourceId::new("shared-id"),
    );
    let images = ResourceTarget::new(ResourcePanelId::new("images"), ResourceId::new("shared-id"));

    assert_ne!(containers, images);
    assert_eq!(containers.to_string(), "shared-id");
    assert_eq!(ResourceState::Stopped, ResourceState::Stopped);
    assert_eq!(DetailViewId::new("inspect").to_string(), "inspect");
}
