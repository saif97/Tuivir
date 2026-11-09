mod common;
use common::ready_workspace;
use tuivir::{application::*, domain::*, infrastructure::provider::ProviderDiscovery};

fn app_with_shell(selectable: bool) -> App {
    let mut app = App::new();
    let mut process = ResourceShellProcess::new("fixture", &["shell"]);
    if selectable {
        process = process.with_user_selection();
    }
    ready_workspace(
        &mut app,
        ProviderDiscovery::new(
            Provider::new(
                ProviderId::new("fixture"),
                "Fixture",
                Some(TargetEnvironment::new("local")),
                None,
            ),
            None,
        ),
        WorkspaceSnapshot {
            panels: vec![ResourcePanel {
                id: ResourcePanelId::new("containers"),
                title: "Containers".into(),
                detail_views: vec![],
                resources: vec![Resource {
                    id: ResourceId::new("web"),
                    name: "web".into(),
                    secondary_text: None,
                    status: None,
                    state: Some(ResourceState::Running),
                    fields: vec![],
                    snapshot_details: vec![],
                    available_commands: &[],
                    shell: Some(process),
                }],
            }],
        },
    );
    app
}

fn discovered(
    requests: Vec<ProviderRequest>,
    result: Result<Option<Vec<String>>, WorkspaceError>,
) -> AppEvent {
    let request = requests
        .into_iter()
        .find(|r| matches!(r, ProviderRequest::DiscoverShellAccounts { .. }))
        .expect("account discovery");
    let ProviderRequest::DiscoverShellAccounts { request_id, .. } = request else {
        unreachable!()
    };
    AppEvent::ShellAccountsDiscovered { request_id, result }
}

#[test]
fn first_shell_use_requires_explicit_selection_even_for_a_single_fuzzy_match() {
    for command in [Command::OpenShell, Command::OpenShellInDetails] {
        let mut app = app_with_shell(true);
        let requests = app.invoke(command);
        assert!(app.take_resource_shell_effects().is_empty());
        app.update(discovered(requests, Ok(Some(vec!["worker".into()]))));
        for c in "wkr".chars() {
            app.invoke(Command::ShellUserCharacter(c));
        }
        let picker = app.state().shell_user_picker.as_ref().unwrap();
        assert_eq!(picker.candidates(), vec!["worker"]);
        assert!(app.take_resource_shell_effects().is_empty());
        assert!(tuivir::presentation::render_to_text(app.state(), 100, 30).contains("worker"));
    }
}

fn verified(
    requests: Vec<ProviderRequest>,
    result: Result<ResourceShellProcess, WorkspaceError>,
) -> AppEvent {
    let request = requests
        .into_iter()
        .find(|r| matches!(r, ProviderRequest::VerifyShellUser { .. }))
        .expect("account verification");
    let ProviderRequest::VerifyShellUser {
        request_id,
        username,
        ..
    } = request
    else {
        unreachable!()
    };
    assert_eq!(username, "worker");
    AppEvent::ShellUserVerified { request_id, result }
}

fn pick_worker(app: &mut App) -> Vec<ProviderRequest> {
    let requests = app.invoke(Command::OpenShell);
    app.update(discovered(requests, Ok(Some(vec!["worker".into()]))));
    app.invoke(Command::Confirm)
}

#[test]
fn a_validated_selection_becomes_the_default_and_starts_one_session() {
    let mut app = app_with_shell(true);
    let request = pick_worker(&mut app);
    assert!(app.state().shell_user_picker.as_ref().unwrap().verifying);
    assert!(app.take_resource_shell_effects().is_empty());
    assert!(app.take_pending_shell_user_save().is_none());
    app.update(verified(
        request,
        Ok(ResourceShellProcess::new("fixture", &["as-worker"])),
    ));
    assert!(app.state().shell_user_picker.is_none());
    let saved = app.take_pending_shell_user_save().unwrap();
    assert_eq!(saved[0].username, "worker");
    assert_eq!(
        saved[0].key.target,
        common::resource_target("containers", "web")
    );
    assert_eq!(
        app.state().resource_shell_sessions[0].username.as_deref(),
        Some("worker")
    );
    assert_eq!(app.take_resource_shell_effects().len(), 1);
    app.invoke(Command::OpenShell);
    assert!(app.take_resource_shell_effects().is_empty());
    let mut restarted = app_with_shell(true).with_default_shell_users(saved);
    let requests = restarted.invoke(Command::OpenShell);
    assert!(requests.iter().any(
        |r| matches!(r, ProviderRequest::VerifyShellUser {username, ..} if username == "worker")
    ));
    assert!(restarted.state().shell_user_picker.is_none());
}
