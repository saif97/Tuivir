use std::sync::Arc;
use tuivir::{
    application::{App, ResourceCommand, ResourceShellProcess},
    domain::{DetailViewId, ProviderId, ProviderVersion, ResourcePanelId, ResourceState},
    infrastructure::{
        process::ProcessSpec,
        provider::{ProviderWorkspace, TartWorkspace},
        runtime::ProviderRuntime,
    },
    presentation::render_to_text,
};

mod common;
use common::{FixtureCli, failure, success};
use tuivir::infrastructure::process::ProcessError;

const LIST: &[&str] = &["list", "--format", "json"];

#[tokio::test]
async fn builtin_runtime_discovers_tart_and_renders_its_empty_workspace() {
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
            success("2.31.0\n"),
        ),
        (ProcessSpec::new("tart", LIST), success("[]")),
    ]);
    let runtime = ProviderRuntime::with_builtin_providers(Arc::new(cli));
    let discoveries = runtime.discover().await;
    assert_eq!(discoveries.len(), 1);
    assert_eq!(discoveries[0].provider().id(), &ProviderId::new("tart"));
    let mut app = App::new();
    let requests = app.update(discoveries.into_iter().next().unwrap().into_event());
    let (events, mut completions) = tokio::sync::mpsc::unbounded_channel();
    runtime.dispatch(common::refresh_request(requests), events);
    let event = tokio::time::timeout(std::time::Duration::from_secs(1), completions.recv())
        .await
        .unwrap()
        .unwrap();
    app.update(event);
    let screen = render_to_text(app.state(), 100, 24);
    assert!(screen.contains("Tart"), "{screen}");
    assert!(screen.contains("VMs"), "{screen}");
    assert!(screen.contains("Images"), "{screen}");
    assert!(!screen.contains("No providers discovered"), "{screen}");
}

#[tokio::test]
async fn tart_distinguishes_an_absent_cli_from_an_installed_but_unusable_cli() {
    let absent = FixtureCli::new([(
        ProcessSpec::new("tart", &["--version"]),
        Err(ProcessError::ExecutableNotFound),
    )]);
    assert!(TartWorkspace.discover(&absent).await.is_none());
    let broken = FixtureCli::new([(
        ProcessSpec::new("tart", &["--version"]),
        failure("unsupported macOS version"),
    )]);
    let discovery = TartWorkspace
        .discover(&broken)
        .await
        .expect("installed Tart");
    let message = &discovery.error().unwrap().message;
    assert!(message.contains("unsupported macOS version"));
    assert!(message.contains("tart --version"));
}

#[tokio::test]
async fn refresh_errors_preserve_cli_diagnostics_and_reject_malformed_or_unknown_sources() {
    let denied = FixtureCli::new([(ProcessSpec::new("tart", LIST), failure("permission denied"))]);
    assert!(
        TartWorkspace
            .refresh(&denied)
            .await
            .unwrap_err()
            .message
            .contains("permission denied")
    );
    for listing in [
        "not json",
        "{}",
        r#"[{"Source":"remote","Name":"vm","Disk":50,"Size":2,"Accessed":"today","State":"stopped"}]"#,
        r#"[{"Source":"local"}]"#,
    ] {
        let cli = FixtureCli::new([(ProcessSpec::new("tart", LIST), success(listing))]);
        assert!(
            TartWorkspace
                .refresh(&cli)
                .await
                .unwrap_err()
                .message
                .contains("tart list --format json")
        );
    }
}

#[tokio::test]
async fn a_failed_stop_prevents_deletion_and_preserves_the_failure() {
    let cli = FixtureCli::new([(
        ProcessSpec::new("tart", &["stop", "--", "dev"]),
        failure("VM termination failed"),
    )]);
    let error = TartWorkspace
        .execute_command(
            &cli,
            &common::resource_target("vms", "dev"),
            ResourceCommand::Delete,
            Some(ResourceState::Running),
        )
        .await
        .unwrap_err();
    assert_eq!(error.message, "VM termination failed");
}

#[tokio::test]
async fn deleting_a_stopped_vm_or_cached_image_never_stops_another_resource() {
    for (panel, name, state) in [
        ("vms", "--unusual VM", Some(ResourceState::Stopped)),
        ("images", "ghcr.io/cirruslabs/macos:latest", None),
    ] {
        let cli = FixtureCli::new([(
            ProcessSpec::new("tart", &["delete", "--", name]),
            success(""),
        )]);
        TartWorkspace
            .execute_command(
                &cli,
                &common::resource_target(panel, name),
                ResourceCommand::Delete,
                state,
            )
            .await
            .unwrap();
        cli.assert_exhausted();
    }
}

#[tokio::test]
async fn unsupported_commands_and_missing_vm_state_are_rejected_without_running_a_cli() {
    let cli = FixtureCli::new([]);
    for (panel, command, state) in [
        ("vms", ResourceCommand::Delete, None),
        (
            "vms",
            ResourceCommand::Restart,
            Some(ResourceState::Running),
        ),
        ("vms", ResourceCommand::Start, Some(ResourceState::Running)),
        ("vms", ResourceCommand::Resume, Some(ResourceState::Stopped)),
        ("images", ResourceCommand::Start, None),
        ("other", ResourceCommand::Delete, None),
    ] {
        assert!(
            TartWorkspace
                .execute_command(&cli, &common::resource_target(panel, "dev"), command, state)
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn unknown_detail_views_are_rejected_and_snapshot_info_needs_no_cli() {
    let cli = FixtureCli::new([]);
    assert!(
        TartWorkspace
            .load_details(
                &cli,
                &common::resource_target("images", "cached"),
                &DetailViewId::new("config")
            )
            .await
            .is_err()
    );
    assert!(
        TartWorkspace
            .load_details(
                &cli,
                &common::resource_target("vms", "dev"),
                &DetailViewId::new("logs")
            )
            .await
            .is_err()
    );
    assert!(
        TartWorkspace
            .load_details(
                &cli,
                &common::resource_target("images", "cached"),
                &DetailViewId::new("info")
            )
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn malformed_config_or_a_disappeared_vm_reports_a_detail_error() {
    for response in [
        success("[]"),
        success("invalid json"),
        failure("VM does not exist"),
    ] {
        let cli = FixtureCli::new([(
            ProcessSpec::new("tart", &["get", "--format", "json", "--", "dev"]),
            response,
        )]);
        assert!(
            TartWorkspace
                .load_details(
                    &cli,
                    &common::resource_target("vms", "dev"),
                    &DetailViewId::new("config")
                )
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn only_running_local_vms_offer_a_guest_agent_shell() {
    let cli = FixtureCli::new([(
        ProcessSpec::new("tart", LIST),
        success(
            r#"[
            {"Source":"local","Name":"dev","Disk":50,"Size":10,"Accessed":"today","State":"running"},
            {"Source":"local","Name":"saved","Disk":50,"Size":10,"Accessed":"today","State":"suspended"},
            {"Source":"local","Name":"off","Disk":50,"Size":10,"Accessed":"today","State":"stopped"},
            {"Source":"local","Name":"future","Disk":50,"Size":10,"Accessed":"today","State":"new-state"}
        ]"#,
        ),
    )]);
    let snapshot = TartWorkspace.refresh(&cli).await.unwrap();
    assert_eq!(
        snapshot.panels[0].resources[0].shell,
        Some(ResourceShellProcess::new(
            "tart",
            &["exec", "-i", "-t", "--", "dev", "/bin/sh"]
        ))
    );
    for resource in &snapshot.panels[0].resources[1..] {
        assert_eq!(resource.shell, None);
    }
    assert_eq!(
        snapshot.panels[0].resources[3].state,
        Some(ResourceState::Unknown)
    );
    assert_eq!(
        snapshot.panels[0].resources[3].available_commands,
        &[ResourceCommand::Delete]
    );
}

#[tokio::test]
async fn older_tart_listings_use_running_only_when_state_is_absent() {
    let cli = FixtureCli::new([(
        ProcessSpec::new("tart", LIST),
        success(
            r#"[
            {"Source":"local","Name":"on","Disk":50,"Size":10,"Accessed":"today","Running":true},
            {"Source":"local","Name":"off","Disk":50,"Size":10,"Accessed":"today","Running":false},
            {"Source":"local","Name":"future","Disk":50,"Size":10,"Accessed":"today","Running":false,"State":"new-state"}
        ]"#,
        ),
    )]);
    let snapshot = TartWorkspace.refresh(&cli).await.expect("legacy listing");
    assert_eq!(
        snapshot.panels[0]
            .resources
            .iter()
            .map(|resource| resource.state)
            .collect::<Vec<_>>(),
        [
            Some(ResourceState::Running),
            Some(ResourceState::Stopped),
            Some(ResourceState::Unknown)
        ]
    );
}

#[tokio::test]
async fn vm_config_loads_only_the_selected_vm_and_preserves_tarts_native_json() {
    let config = r#"{"OS":"darwin","CPU":4,"Memory":8192,"Disk":null,"DiskFormat":"asif","Size":"20.125","Display":"1920x1080","Running":true,"State":"running"}"#;
    let cli = FixtureCli::new([(
        ProcessSpec::new("tart", &["get", "--format", "json", "--", "macOS dev"]),
        success(config),
    )]);
    let details = TartWorkspace
        .load_details(
            &cli,
            &common::resource_target("vms", "macOS dev"),
            &DetailViewId::new("config"),
        )
        .await
        .expect("VM Config");
    let value: serde_json::Value = serde_json::from_str(&details.lines.join("\n")).unwrap();
    assert_eq!(value["Memory"], 8192);
    assert_eq!(value["Size"], "20.125");
    assert_eq!(value["Disk"], serde_json::Value::Null);
}

#[tokio::test]
async fn tart_stops_a_running_vm_before_deleting_it() {
    let cli = FixtureCli::new([
        (
            ProcessSpec::new("tart", &["stop", "--", "macOS dev"]),
            success(""),
        ),
        (
            ProcessSpec::new("tart", &["delete", "--", "macOS dev"]),
            success(""),
        ),
    ]);
    TartWorkspace
        .execute_command(
            &cli,
            &common::resource_target("vms", "macOS dev"),
            ResourceCommand::Delete,
            Some(ResourceState::Running),
        )
        .await
        .expect("confirmed deletion stops and deletes the VM");
    cli.assert_exhausted();
}

#[tokio::test]
async fn tart_start_and_resume_launch_headless_vms_without_waiting_for_shutdown() {
    for (command, state) in [
        (ResourceCommand::Start, ResourceState::Stopped),
        (ResourceCommand::Resume, ResourceState::Paused),
    ] {
        let cli = FixtureCli::new([(
            ProcessSpec::background("tart", &["run", "--no-graphics", "--", "macOS dev"]),
            success(""),
        )]);
        TartWorkspace
            .execute_command(
                &cli,
                &common::resource_target("vms", "macOS dev"),
                command,
                Some(state),
            )
            .await
            .expect("VM launch");
        cli.assert_exhausted();
    }
}

#[tokio::test]
async fn tart_separates_local_vms_from_cached_images_and_preserves_native_summary_fields() {
    let cli = FixtureCli::new([(
        ProcessSpec::new("tart", LIST),
        success(
            r#"[
          {"Source":"local","Name":"macOS dev","Disk":50,"Size":21,"Accessed":"2026-10-04T00:00:00Z","Running":true,"State":"running"},
          {"Source":"local","Name":"saved","Disk":null,"Size":8,"Accessed":"2026-10-03T00:00:00Z","Running":false,"State":"suspended"},
          {"Source":"OCI","Name":"ghcr.io/cirruslabs/macos:latest","Disk":50,"Size":19,"Accessed":"2026-10-02T00:00:00Z","Running":false,"State":"stopped"}
        ]"#,
        ),
    )]);
    let snapshot = TartWorkspace
        .refresh(&cli)
        .await
        .expect("valid Tart listing");
    let vms = &snapshot.panels[0];
    assert_eq!(vms.id, ResourcePanelId::new("vms"));
    assert_eq!(vms.title, "VMs");
    assert_eq!(vms.resources[0].id.0, "macOS dev");
    assert_eq!(vms.resources[0].state, Some(ResourceState::Running));
    assert_eq!(
        vms.resources[0].available_commands,
        &[ResourceCommand::Stop, ResourceCommand::Delete]
    );
    assert!(vms.resources[0].fields.contains(&("Disk", "50 GB".into())));
    assert_eq!(vms.resources[1].state, Some(ResourceState::Paused));
    assert_eq!(
        vms.resources[1].available_commands,
        &[ResourceCommand::Resume, ResourceCommand::Delete]
    );
    assert!(
        !vms.resources[1]
            .fields
            .iter()
            .any(|(label, _)| *label == "Disk")
    );
    let images = &snapshot.panels[1];
    assert_eq!(images.id, ResourcePanelId::new("images"));
    assert_eq!(images.title, "Images");
    assert_eq!(images.resources[0].state, None);
    assert_eq!(images.resources[0].status, None);
    assert_eq!(
        images.resources[0].available_commands,
        &[ResourceCommand::Delete]
    );
    assert_eq!(images.resources[0].shell, None);
    let info = snapshot
        .snapshot_detail(
            &common::resource_target("images", "ghcr.io/cirruslabs/macos:latest"),
            &DetailViewId::new("info"),
        )
        .expect("cached image Info");
    assert!(info.lines.contains(&"Size: 19 GB".into()));
}

#[tokio::test]
async fn tart_discovery_reports_its_version_without_inventing_a_target_environment() {
    let cli = FixtureCli::new([(
        ProcessSpec::new("tart", &["--version"]),
        success("2.31.0\n"),
    )]);
    let discovery = TartWorkspace.discover(&cli).await.expect("installed Tart");
    assert_eq!(discovery.provider().id(), &ProviderId::new("tart"));
    assert_eq!(discovery.provider().name(), "Tart");
    assert_eq!(
        discovery.provider().version(),
        Some(&ProviderVersion::new("2.31.0"))
    );
    assert_eq!(discovery.provider().target_environment(), None);
    assert_eq!(discovery.error(), None);
}
