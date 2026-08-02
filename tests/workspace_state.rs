use virtui::{
    domain::Provider,
    provider::{
        DetailView, ProviderId, ProviderRequestId, Resource, ResourceDetails, ResourceId,
        ResourcePanel, ResourcePanelId, ResourceTarget, TargetEnvironment, WorkspaceError,
        WorkspaceSnapshot,
    },
    workspace::{DetailContent, ProviderWorkspaceState, WorkspaceLoadState},
};

fn resource(id: &str, name: &str) -> Resource {
    Resource {
        id: ResourceId::new(id),
        name: name.to_owned(),
        status: None,
        state: None,
        fields: Vec::new(),
        available_commands: Vec::new(),
        shell: None,
    }
}

fn snapshot() -> WorkspaceSnapshot {
    WorkspaceSnapshot {
        panels: vec![
            ResourcePanel {
                id: ResourcePanelId::new("containers"),
                title: "Containers".to_owned(),
                detail_views: vec![DetailView::new("logs", "Logs")],
                resources: vec![resource("container-a", "api")],
            },
            ResourcePanel {
                id: ResourcePanelId::new("images"),
                title: "Images".to_owned(),
                detail_views: vec![DetailView::new("inspect", "Inspect")],
                resources: vec![resource("image-a", "alpine")],
            },
        ],
    }
}

fn workspace() -> ProviderWorkspaceState {
    ProviderWorkspaceState::new(
        Provider::new(
            ProviderId::new("docker"),
            "Docker",
            TargetEnvironment::new("desktop-linux"),
            None,
        ),
        None,
    )
}

#[test]
fn a_resource_target_looks_up_one_panel_qualified_resource() {
    let snapshot = snapshot();
    let target = ResourceTarget::new(
        ResourcePanelId::new("containers"),
        ResourceId::new("container-a"),
    );

    assert_eq!(
        snapshot
            .resource(&target)
            .map(|resource| resource.name.as_str()),
        Some("api")
    );
}

#[test]
fn reconciling_the_first_snapshot_projects_one_coherent_workspace_view() {
    let mut workspace = workspace();

    workspace.reconcile_snapshot(snapshot());

    let WorkspaceLoadState::Ready(snapshot) = workspace.load_state() else {
        panic!("the reconciled workspace is ready");
    };
    let view = workspace.view(snapshot);
    assert_eq!(view.id, &ProviderId::new("docker"));
    assert_eq!(
        view.target_environment,
        &TargetEnvironment::new("desktop-linux")
    );
    assert_eq!(
        view.focused_resource_panel,
        Some(&ResourcePanelId::new("containers"))
    );
    assert_eq!(view.panels.len(), 2);
    assert_eq!(
        view.panels[0].selected_resource,
        Some(&ResourceId::new("container-a"))
    );
    assert_eq!(
        view.selected_resource
            .map(|resource| resource.name.as_str()),
        Some("api")
    );
    assert_eq!(
        view.selected_detail_view.map(|view| view.title.as_str()),
        Some("Logs")
    );
}

#[test]
fn focusing_a_resource_panel_restores_that_panels_selection_and_detail_view() {
    let mut workspace = workspace();
    workspace.reconcile_snapshot(snapshot());

    assert!(workspace.focus_resource_panel(&ResourcePanelId::new("images")));

    let WorkspaceLoadState::Ready(snapshot) = workspace.load_state() else {
        panic!("the reconciled workspace is ready");
    };
    let view = workspace.view(snapshot);
    assert_eq!(
        view.focused_resource_panel,
        Some(&ResourcePanelId::new("images"))
    );
    assert_eq!(
        view.selected_resource
            .map(|resource| resource.name.as_str()),
        Some("alpine")
    );
    assert_eq!(
        view.selected_detail_view.map(|view| view.title.as_str()),
        Some("Inspect")
    );
    assert!(!workspace.focus_resource_panel(&ResourcePanelId::new("missing")));
}

#[test]
fn moving_resource_selection_updates_only_the_focused_panels_navigation() {
    let mut snapshot = snapshot();
    snapshot.panels[0]
        .resources
        .push(resource("container-b", "worker"));
    let mut workspace = workspace();
    workspace.reconcile_snapshot(snapshot);

    workspace.move_resource_selection(1);
    assert!(workspace.focus_resource_panel(&ResourcePanelId::new("images")));
    workspace.move_resource_selection(1);
    assert!(workspace.focus_resource_panel(&ResourcePanelId::new("containers")));

    let WorkspaceLoadState::Ready(snapshot) = workspace.load_state() else {
        panic!("the reconciled workspace is ready");
    };
    let view = workspace.view(snapshot);
    assert_eq!(
        view.selected_resource
            .map(|resource| resource.name.as_str()),
        Some("worker")
    );
    assert_eq!(view.panels[0].scroll, 1);
    assert_eq!(
        view.panels[1].selected_resource,
        Some(&ResourceId::new("image-a"))
    );
}

#[test]
fn moving_the_detail_view_wraps_through_the_focused_panels_views() {
    let mut snapshot = snapshot();
    snapshot.panels[0].detail_views = vec![
        DetailView::new("logs", "Logs"),
        DetailView::new("stats", "Stats"),
        DetailView::new("inspect", "Inspect"),
    ];
    let mut workspace = workspace();
    workspace.reconcile_snapshot(snapshot);

    workspace.move_detail_view(-1);

    let WorkspaceLoadState::Ready(snapshot) = workspace.load_state() else {
        panic!("the reconciled workspace is ready");
    };
    assert_eq!(
        workspace
            .view(snapshot)
            .selected_detail_view
            .map(|view| view.title.as_str()),
        Some("Inspect")
    );
}

#[test]
fn a_detail_result_is_accepted_only_for_the_still_visible_resource_and_view() {
    let mut snapshot = snapshot();
    snapshot.panels[0]
        .resources
        .push(resource("container-b", "worker"));
    let mut workspace = workspace();
    workspace.reconcile_snapshot(snapshot);
    let stale = workspace
        .start_visible_detail_load(ProviderRequestId::new(1))
        .expect("the selected Resource offers Logs");

    workspace.move_resource_selection(1);
    let current = workspace
        .start_visible_detail_load(ProviderRequestId::new(2))
        .expect("the newly selected Resource needs Logs");
    workspace.complete_detail_load(stale.completion(Ok(ResourceDetails::from_lines(["stale"]))));

    let WorkspaceLoadState::Ready(snapshot) = workspace.load_state() else {
        panic!("the reconciled workspace is ready");
    };
    let details = workspace
        .view(snapshot)
        .details
        .expect("the current detail remains visible");
    assert_eq!(details.resource_name, "worker");
    assert_eq!(details.content, &DetailContent::Loading);

    workspace.complete_detail_load(current.completion(Ok(ResourceDetails::from_lines(["fresh"]))));
    let WorkspaceLoadState::Ready(snapshot) = workspace.load_state() else {
        panic!("the reconciled workspace is ready");
    };
    let details = workspace.view(snapshot).details.expect("loaded details");
    assert_eq!(
        details.content,
        &DetailContent::Ready(ResourceDetails::from_lines(["fresh"]))
    );
}

#[test]
fn scrolling_details_clamps_to_the_visible_detail_content() {
    let mut workspace = workspace();
    workspace.reconcile_snapshot(snapshot());
    let load = workspace
        .start_visible_detail_load(ProviderRequestId::new(1))
        .expect("the selected Resource offers Logs");
    workspace.complete_detail_load(load.completion(Ok(ResourceDetails::from_lines(
        (0..15).map(|line| format!("line {line}")),
    ))));

    workspace.scroll_details(100);

    let WorkspaceLoadState::Ready(snapshot) = workspace.load_state() else {
        panic!("the reconciled workspace is ready");
    };
    assert_eq!(
        workspace.view(snapshot).details.expect("details").scroll,
        14
    );
    workspace.scroll_details(-100);
    let WorkspaceLoadState::Ready(snapshot) = workspace.load_state() else {
        panic!("the reconciled workspace is ready");
    };
    assert_eq!(workspace.view(snapshot).details.expect("details").scroll, 0);
}

#[test]
fn navigation_itself_refuses_the_detail_result_for_the_resource_left_behind() {
    let mut snapshot = snapshot();
    snapshot.panels[0]
        .resources
        .push(resource("container-b", "worker"));
    let mut workspace = workspace();
    workspace.reconcile_snapshot(snapshot);
    let stale = workspace
        .start_visible_detail_load(ProviderRequestId::new(1))
        .expect("the selected Resource offers Logs");

    workspace.move_resource_selection(1);
    workspace.complete_detail_load(stale.completion(Ok(ResourceDetails::from_lines(["stale"]))));
    workspace.move_resource_selection(-1);

    assert!(
        workspace
            .start_visible_detail_load(ProviderRequestId::new(2))
            .is_some(),
        "returning to the Resource reloads details whose result was refused"
    );
}

#[test]
fn recovery_from_a_refresh_error_restores_still_valid_navigation() {
    let mut workspace = workspace();
    workspace.reconcile_snapshot(snapshot());
    assert!(workspace.focus_resource_panel(&ResourcePanelId::new("images")));

    workspace.record_load_error(WorkspaceError::new("temporarily unavailable"));
    workspace.reconcile_snapshot(snapshot());

    let WorkspaceLoadState::Ready(snapshot) = workspace.load_state() else {
        panic!("the recovered workspace is ready");
    };
    let view = workspace.view(snapshot);
    assert_eq!(
        view.focused_resource_panel,
        Some(&ResourcePanelId::new("images"))
    );
    assert_eq!(
        view.selected_resource
            .map(|resource| resource.name.as_str()),
        Some("alpine")
    );
    assert_eq!(
        view.selected_detail_view.map(|view| view.title.as_str()),
        Some("Inspect")
    );
}
