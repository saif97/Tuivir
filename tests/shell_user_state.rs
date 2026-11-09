use tuivir::{
    application::{DefaultShellUser, ShellUserKey},
    domain::{ProviderId, ResourceId, ResourcePanelId, ResourceTarget, TargetEnvironment},
    infrastructure::{
        config::FileSystemReader,
        pane_boundary_state::Env,
        shell_user_state::{load, save},
    },
};

#[test]
fn default_shell_users_survive_restart_without_crossing_resource_or_environment_boundaries() {
    let directory = std::env::temp_dir().join(format!("tuivir-shell-users-{}", std::process::id()));
    let env = Env {
        xdg_state_home: Some(directory.clone()),
        home: None,
    };
    let key = |provider: &str, environment: &str, resource: &str| ShellUserKey {
        provider_id: ProviderId::new(provider),
        target_environment: Some(TargetEnvironment::new(environment)),
        target: ResourceTarget::new(
            ResourcePanelId::new("containers"),
            ResourceId::new(resource),
        ),
    };
    let preferences = vec![
        DefaultShellUser {
            key: key("docker", "local", "web"),
            username: "worker".into(),
        },
        DefaultShellUser {
            key: key("docker", "remote", "web"),
            username: "operator".into(),
        },
        DefaultShellUser {
            key: key("docker", "local", "db"),
            username: "postgres".into(),
        },
        DefaultShellUser {
            key: key("incus", "local", "web"),
            username: "ubuntu".into(),
        },
    ];
    save(&env, &FileSystemReader, &preferences).unwrap();
    assert_eq!(load(&env, &FileSystemReader), preferences);
    std::fs::remove_dir_all(directory).unwrap();
}
