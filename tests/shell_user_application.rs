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
