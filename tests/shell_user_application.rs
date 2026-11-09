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

#[test]
fn switching_a_live_shell_validates_then_confirms_before_replacing_it() {
    let mut app = app_with_shell(true);
    let requests = pick_worker(&mut app);
    app.update(verified(
        requests,
        Ok(ResourceShellProcess::new("fixture", &["worker"])),
    ));
    let original = app.state().resource_shell_sessions[0].id;
    app.update(AppEvent::ResourceShellStarted {
        session_id: original,
    });
    let saved = app.take_pending_shell_user_save().unwrap();
    app.take_resource_shell_effects();
    let requests = app.invoke(Command::OpenShellAs);
    app.update(discovered(requests, Ok(None)));
    for c in "operator".chars() {
        app.invoke(Command::ShellUserCharacter(c));
    }
    let requests = app.invoke(Command::Confirm);
    let ProviderRequest::VerifyShellUser { request_id, .. } = requests[0] else {
        panic!("verify")
    };
    assert!(app.state().confirmation.is_none());
    app.update(AppEvent::ShellUserVerified {
        request_id,
        result: Ok(ResourceShellProcess::new("fixture", &["operator"])),
    });
    assert!(app.state().confirmation.is_some());
    assert!(app.take_resource_shell_effects().is_empty());
    assert_eq!(app.state().default_shell_users, saved);
    app.invoke(Command::Cancel);
    assert_eq!(app.state().resource_shell_sessions[0].id, original);
    assert_eq!(app.state().default_shell_users, saved);
    let requests = app.invoke(Command::OpenShellAs);
    app.update(discovered(requests, Ok(Some(vec!["operator".into()]))));
    let requests = app.invoke(Command::Confirm);
    let ProviderRequest::VerifyShellUser { request_id, .. } = requests[0] else {
        panic!("verify")
    };
    app.update(AppEvent::ShellUserVerified {
        request_id,
        result: Ok(ResourceShellProcess::new("fixture", &["operator"])),
    });
    app.invoke(Command::Confirm);
    assert_eq!(app.state().resource_shell_sessions.len(), 1);
    assert_eq!(app.state().default_shell_users[0].username, "operator");
    assert!(
        matches!(app.take_resource_shell_effects().as_slice(), [ResourceShellEffect::Stop { session_id }, ResourceShellEffect::Start { .. }] if *session_id == original)
    );
}

#[test]
fn a_separate_launch_failure_reopens_the_picker_without_replacing_the_default() {
    let mut app = app_with_shell(true);
    let requests = pick_worker(&mut app);
    app.update(verified(
        requests,
        Ok(ResourceShellProcess::new("fixture", &["worker"])),
    ));
    let saved = app.take_pending_shell_user_save().unwrap();
    let session_id = app.state().resource_shell_sessions[0].id;
    let requests = app.update(AppEvent::ResourceShellStartFailed {
        session_id,
        reason: "account no longer exists".into(),
    });
    let picker = app
        .state()
        .shell_user_picker
        .as_ref()
        .expect("recovery picker");
    assert_eq!(picker.input, "worker");
    assert_eq!(picker.error.as_deref(), Some("account no longer exists"));
    assert!(
        requests
            .iter()
            .any(|r| matches!(r, ProviderRequest::DiscoverShellAccounts { .. }))
    );
    app.invoke(Command::Cancel);
    assert_eq!(app.state().default_shell_users, saved);
    assert!(app.take_pending_shell_user_save().is_none());
}

#[test]
fn failed_checks_preserve_input_default_and_live_session_without_fallback() {
    let mut app = app_with_shell(true);
    let request = pick_worker(&mut app);
    app.update(verified(
        request,
        Ok(ResourceShellProcess::new("fixture", &["worker"])),
    ));
    let original = app.state().resource_shell_sessions[0].id;
    app.update(AppEvent::ResourceShellStarted {
        session_id: original,
    });
    let saved = app.take_pending_shell_user_save().unwrap();
    app.take_resource_shell_effects();
    for reason in ["user not found", "permission denied", "shell unavailable"] {
        let requests = app.invoke(Command::OpenShellAs);
        app.update(discovered(requests, Ok(None)));
        for c in "worker".chars() {
            app.invoke(Command::ShellUserCharacter(c));
        }
        let request = app.invoke(Command::Confirm);
        app.update(verified(request, Err(WorkspaceError::new(reason))));
        let picker = app.state().shell_user_picker.as_ref().unwrap();
        assert_eq!(picker.input, "worker");
        assert_eq!(picker.error.as_deref(), Some(reason));
        assert!(!picker.verifying);
        assert_eq!(app.state().default_shell_users, saved);
        assert_eq!(app.state().resource_shell_sessions[0].id, original);
        assert!(app.take_resource_shell_effects().is_empty());
        app.invoke(Command::Cancel);
    }
}

#[test]
fn unsupported_targets_keep_native_shell_without_shell_as_or_saved_override() {
    let mut app = app_with_shell(false).with_default_shell_users(vec![DefaultShellUser {
        key: ShellUserKey {
            provider_id: ProviderId::new("fixture"),
            target_environment: Some(TargetEnvironment::new("local")),
            target: common::resource_target("containers", "web"),
        },
        username: "worker".into(),
    }]);
    app.invoke(Command::ToggleHelp);
    assert!(
        !app.state()
            .help_overlay
            .as_ref()
            .unwrap()
            .entries
            .iter()
            .any(|e| e.description == "Shell as…")
    );
    app.invoke(Command::Cancel);
    assert!(app.invoke(Command::OpenShellAs).is_empty());
    assert!(app.state().shell_user_picker.is_none());
    app.invoke(Command::OpenShell);
    assert!(app.state().shell_user_picker.is_none());
    assert!(
        matches!(app.take_resource_shell_effects().as_slice(), [ResourceShellEffect::Start { process, .. }] if process.args() == ["shell"])
    );
}

#[test]
fn selecting_the_live_account_resumes_it_without_replacement() {
    let mut app = app_with_shell(true);
    let request = pick_worker(&mut app);
    app.update(verified(
        request,
        Ok(ResourceShellProcess::new("fixture", &["worker"])),
    ));
    let original = app.state().resource_shell_sessions[0].id;
    app.update(AppEvent::ResourceShellStarted {
        session_id: original,
    });
    app.take_pending_shell_user_save();
    app.take_resource_shell_effects();
    let requests = app.invoke(Command::OpenShellAs);
    app.update(discovered(requests, Ok(Some(vec!["worker".into()]))));
    let request = app.invoke(Command::Confirm);
    app.update(verified(
        request,
        Ok(ResourceShellProcess::new("fixture", &["worker"])),
    ));
    assert_eq!(app.state().resource_shell_sessions[0].id, original);
    assert!(app.state().confirmation.is_none());
    assert!(app.take_resource_shell_effects().is_empty());
    assert!(app.take_pending_shell_user_save().is_none());
}

#[test]
fn cancelling_verification_refuses_its_late_success() {
    let mut app = app_with_shell(true);
    let requests = pick_worker(&mut app);
    app.invoke(Command::Cancel);
    app.update(verified(
        requests,
        Ok(ResourceShellProcess::new("fixture", &["worker"])),
    ));
    assert!(app.state().default_shell_users.is_empty());
    assert!(app.take_resource_shell_effects().is_empty());
}
