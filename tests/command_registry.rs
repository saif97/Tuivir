use virtui::application::{
    Command, CommandRegistry, CommandScope, Key, KeybindingError, ResourceCommand,
};

fn key(text: &str) -> Key {
    Key::parse(text).unwrap_or_else(|_| panic!("{text} is a key"))
}

/// Layers a `[keybindings]` table over the compiled defaults, expecting it to
/// be valid.
fn effective(overrides: &[(&str, &[&str])]) -> CommandRegistry {
    let overrides = overrides
        .iter()
        .map(|(id, keys)| {
            (
                (*id).to_owned(),
                keys.iter().map(|key| (*key).to_owned()).collect(),
            )
        })
        .collect::<Vec<_>>();
    CommandRegistry::effective(&overrides).unwrap_or_else(|errors| {
        panic!("configuration should be valid, but: {errors:?}");
    })
}

/// Layers a `[keybindings]` table over the compiled defaults, expecting it to
/// be refused.
fn rejected(overrides: &[(&str, &[&str])]) -> Vec<KeybindingError> {
    let overrides = overrides
        .iter()
        .map(|(id, keys)| {
            (
                (*id).to_owned(),
                keys.iter().map(|key| (*key).to_owned()).collect(),
            )
        })
        .collect::<Vec<_>>();
    CommandRegistry::effective(&overrides)
        .err()
        .unwrap_or_else(|| panic!("configuration should be refused: {overrides:?}"))
}

/// `ctrl+c` is always an emergency Quit, so the terminal can always be
/// restored no matter what the file says.
#[test]
fn ctrl_c_always_quits() {
    for registry in [
        CommandRegistry::builtin(),
        effective(&[("app.quit", &["ctrl+q"])]),
        effective(&[("app.quit", &[])]),
    ] {
        assert_eq!(registry.reserved(key("ctrl+c")), Some(Command::Quit));
    }
}

#[test]
fn configuring_ctrl_c_for_quit_preserves_the_position_the_user_gave_it() {
    let registry = effective(&[("app.quit", &["ctrl+c", "ctrl+q"])]);

    assert_eq!(
        registry.first_key(Command::Quit),
        Some(key("ctrl+c")),
        "an explicitly configured ctrl+c keeps its place in the list"
    );
}

#[test]
fn ctrl_c_is_appended_to_quit_when_it_is_not_configured() {
    let registry = effective(&[("app.quit", &["ctrl+q"])]);
    let quit = registry
        .in_scope(CommandScope::ResourceView)
        .find(|entry| entry.id == "app.quit")
        .expect("Quit stays bound");

    assert_eq!(quit.keys, vec![key("ctrl+q"), key("ctrl+c")]);
}

#[test]
fn no_other_command_may_claim_ctrl_c() {
    let errors =
        CommandRegistry::effective(&[("resource.delete".to_owned(), vec!["ctrl+c".to_owned()])])
            .expect_err("ctrl+c is reserved");

    assert_eq!(
        errors,
        vec![KeybindingError::ReservedKey {
            id: "resource.delete".to_owned(),
            key: "ctrl+c".to_owned(),
        }]
    );
}

/// One key listed twice for one Command is a mistake with no meaning: the
/// second mention can only be a typo or a misunderstanding.
#[test]
fn a_key_repeated_within_one_command_is_rejected() {
    assert_eq!(
        rejected(&[("resource.stop", &["x", "x"])]),
        vec![KeybindingError::DuplicateKey {
            id: "resource.stop".to_owned(),
            key: "x".to_owned(),
        }]
    );
}

/// Duplicates are compared as parsed keys rather than as the text the user
/// wrote, so the two spellings of the spacebar cannot slip through as two
/// separate bindings.
#[test]
fn two_spellings_of_one_key_are_still_a_duplicate() {
    assert_eq!(
        rejected(&[("app.help", &["space", " "])]),
        vec![KeybindingError::DuplicateKey {
            id: "app.help".to_owned(),
            key: "space".to_owned(),
        }]
    );
}

/// Two Commands reachable at once cannot share a key. Resolving by
/// registration order would let one silently win while Help and the inline hint
/// still advertised the loser, so the file is refused instead.
#[test]
fn one_key_bound_to_two_commands_in_one_scope_is_rejected() {
    assert_eq!(
        rejected(&[("resource.stop", &["j"])]),
        vec![KeybindingError::ConflictingKey {
            key: "j".to_owned(),
            first: "selection.next".to_owned(),
            second: "resource.stop".to_owned(),
        }],
        "j already moves the selection in the resource view"
    );
}

/// A modal replaces the workspace scope, so a modal key and a resource key are
/// never offered at the same time and can share a spelling.
#[test]
fn one_key_may_serve_commands_whose_scopes_cannot_overlap() {
    let registry = effective(&[("resource.stop", &["y"])]);

    assert_eq!(
        registry.resolve(CommandScope::ResourceView, key("y")),
        Some(Command::Resource(ResourceCommand::Stop))
    );
    assert_eq!(
        registry.resolve(CommandScope::Confirmation, key("y")),
        Some(Command::Confirm),
        "the same key still confirms the modal that replaced the workspace"
    );
}

/// The compiled defaults are a configuration like any other, so they must pass
/// the validation a user's file has to pass.
#[test]
fn the_compiled_defaults_contain_no_conflict() {
    effective(&[]);
}

#[test]
fn a_configured_command_replaces_its_complete_default_list() {
    let registry = effective(&[("selection.next", &["ctrl+n"])]);

    assert_eq!(
        registry.resolve(CommandScope::ResourceView, key("ctrl+n")),
        Some(Command::SelectNext)
    );
    for removed in ["j", "down"] {
        assert_eq!(
            registry.resolve(CommandScope::ResourceView, key(removed)),
            None,
            "a replaced default must not stay active"
        );
    }
}

#[test]
fn an_unmentioned_command_keeps_its_defaults() {
    let registry = effective(&[("selection.next", &["ctrl+n"])]);

    assert_eq!(
        registry.resolve(CommandScope::ResourceView, key("k")),
        Some(Command::SelectPrevious)
    );
    assert_eq!(
        registry.resolve(CommandScope::ResourceView, key("d")),
        Some(Command::Resource(ResourceCommand::Delete))
    );
}

#[test]
fn an_empty_key_list_leaves_a_command_unbound() {
    let registry = effective(&[("resource.delete", &[])]);

    assert_eq!(registry.resolve(CommandScope::ResourceView, key("d")), None);
    assert_eq!(
        registry.first_key(Command::Resource(ResourceCommand::Delete)),
        None,
        "an unbound Command has no inline hint"
    );
    assert!(
        !registry
            .in_scope(CommandScope::ResourceView)
            .any(|entry| entry.id == "resource.delete"),
        "an unbound Command is omitted from its scope"
    );
}

#[test]
fn several_keys_can_invoke_one_command() {
    let registry = effective(&[("resource.restart", &["r", "f5"])]);

    for text in ["r", "f5"] {
        assert_eq!(
            registry.resolve(CommandScope::ResourceView, key(text)),
            Some(Command::Resource(ResourceCommand::Restart))
        );
    }
    assert_eq!(
        registry.first_key(Command::Resource(ResourceCommand::Restart)),
        Some(key("r")),
        "the first configured key is the preferred inline hint"
    );
}

#[test]
fn shell_focus_and_navigation_defaults_resolve_in_their_own_scopes() {
    let registry = CommandRegistry::builtin();

    for (scope, text, command) in [
        (CommandScope::ResourceView, "q", Command::Quit),
        (CommandScope::ProviderSelector, "q", Command::Quit),
        (CommandScope::ResourceView, "?", Command::ToggleHelp),
        (CommandScope::ResourceView, "ctrl+r", Command::Refresh),
        (CommandScope::ResourceView, "1", Command::FocusProviders),
        (
            CommandScope::ResourceView,
            "2",
            Command::FocusResourcePanel(0),
        ),
        (
            CommandScope::ResourceView,
            "3",
            Command::FocusResourcePanel(1),
        ),
        (CommandScope::ResourceView, "enter", Command::FocusDetails),
        (CommandScope::ResourceView, "tab", Command::FocusNextPane),
        (
            CommandScope::ResourceView,
            "shift+tab",
            Command::FocusPreviousPane,
        ),
        (CommandScope::ResourceView, "j", Command::SelectNext),
        (CommandScope::ResourceView, "down", Command::SelectNext),
        (CommandScope::ResourceView, "k", Command::SelectPrevious),
        (CommandScope::ResourceView, "up", Command::SelectPrevious),
        (CommandScope::ProviderSelector, "j", Command::SelectNext),
        (CommandScope::ProviderSelector, "k", Command::SelectPrevious),
        (CommandScope::ResourceView, "]", Command::NextWorkspace),
        (CommandScope::ResourceView, "[", Command::PreviousWorkspace),
    ] {
        assert_eq!(
            registry.resolve(scope, key(text)),
            Some(command),
            "{text} in {scope:?} should invoke {command:?}"
        );
    }
}

#[test]
fn every_direct_focus_command_exposes_its_effective_hint() {
    let registry = effective(&[("focus.resources.2", &["f7"]), ("focus.details", &["f6"])]);

    assert_eq!(
        registry.first_key(Command::FocusResourcePanel(1)),
        Some(key("f7"))
    );
    assert_eq!(registry.first_key(Command::FocusDetails), Some(key("f6")));
    assert_eq!(
        registry.resolve(CommandScope::ProviderSelector, key("f7")),
        Some(Command::FocusResourcePanel(1))
    );
    assert_eq!(
        registry.resolve(CommandScope::ResourcePanel(1), key("j")),
        Some(Command::SelectNext),
        "generic Resource Panel Commands resolve in each distinct panel scope"
    );
    assert_eq!(
        registry.resolve(CommandScope::ProviderSelector, key("f6")),
        Some(Command::FocusDetails)
    );
}

/// Fast navigation moves a resource list; the Provider selector is short enough
/// that a five-item jump would only overshoot it.
#[test]
fn fast_navigation_is_registered_for_resources_but_not_the_provider_selector() {
    let registry = CommandRegistry::builtin();

    assert_eq!(
        registry.resolve(CommandScope::ResourceView, key("J")),
        Some(Command::SelectNextFast)
    );
    assert_eq!(
        registry.resolve(CommandScope::ResourceView, key("K")),
        Some(Command::SelectPreviousFast)
    );
    assert_eq!(
        registry.resolve(CommandScope::ProviderSelector, key("J")),
        None
    );
}

/// Manual refresh moves off `r` because `r` is Restart in a resource view.
#[test]
fn plain_r_restarts_a_resource_rather_than_refreshing() {
    let registry = CommandRegistry::builtin();

    assert_eq!(
        registry.resolve(CommandScope::ResourceView, key("r")),
        Some(Command::Resource(ResourceCommand::Restart))
    );
    assert_eq!(
        registry.resolve(CommandScope::ResourceView, key("ctrl+r")),
        Some(Command::Refresh)
    );
}

/// A modal scope replaces the ordinary workspace scope, so isolation comes from
/// scoped resolution rather than the order of hard-coded branches.
#[test]
fn a_modal_scope_replaces_the_workspace_scope() {
    let registry = CommandRegistry::builtin();

    for modal in [
        CommandScope::Confirmation,
        CommandScope::CommandFailure,
        CommandScope::HelpOverlay,
    ] {
        for text in ["s", "d", "j", "]", "1", "ctrl+r"] {
            assert_eq!(
                registry.resolve(modal, key(text)),
                None,
                "{text} must not reach a workspace Command from {modal:?}"
            );
        }
    }

    assert_eq!(
        registry.resolve(CommandScope::Confirmation, key("y")),
        Some(Command::Confirm)
    );
    assert_eq!(
        registry.resolve(CommandScope::Confirmation, key("enter")),
        Some(Command::Confirm)
    );
    assert_eq!(
        registry.resolve(CommandScope::Confirmation, key("n")),
        Some(Command::Cancel)
    );
    assert_eq!(
        registry.resolve(CommandScope::Confirmation, key("esc")),
        Some(Command::Cancel)
    );
    assert_eq!(
        registry.resolve(CommandScope::CommandFailure, key("esc")),
        Some(Command::Cancel)
    );
    assert_eq!(
        registry.resolve(CommandScope::HelpOverlay, key("esc")),
        Some(Command::Cancel)
    );
    assert_eq!(
        registry.resolve(CommandScope::HelpOverlay, key("?")),
        Some(Command::ToggleHelp)
    );
}

/// `Esc` cancels or returns from modal interaction; it is not a global quit.
#[test]
fn escape_is_not_registered_as_quit() {
    let registry = CommandRegistry::builtin();

    for scope in [CommandScope::ResourceView, CommandScope::ProviderSelector] {
        assert_eq!(registry.resolve(scope, key("esc")), None);
    }
}

#[test]
fn resource_lifecycle_defaults_keep_their_lazydocker_meanings() {
    let registry = CommandRegistry::builtin();

    for (text, command) in [
        ("S", ResourceCommand::Start),
        ("s", ResourceCommand::Stop),
        ("r", ResourceCommand::Restart),
        ("d", ResourceCommand::Delete),
    ] {
        assert_eq!(
            registry.resolve(CommandScope::ResourceView, key(text)),
            Some(Command::Resource(command)),
            "{text} should invoke {command}"
        );
    }
}
