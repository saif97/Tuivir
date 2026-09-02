use std::{
    collections::HashMap,
    io::{Error, ErrorKind},
    path::{Path, PathBuf},
};

use tuivir::{
    application::{
        Command, CommandScope, Key, KeybindingError as ConfigError, ResourceCommand,
        ResourceShellControls,
    },
    infrastructure::config::{Env, FileSystemReader, LoadError, ReadFile, load},
};

fn key(text: &str) -> Key {
    Key::parse(text).expect("a valid key")
}

/// An in-memory filesystem for configuration tests, so they never touch a real
/// home directory.
struct MemoryFs {
    files: HashMap<PathBuf, String>,
}

impl MemoryFs {
    fn empty() -> Self {
        Self {
            files: HashMap::new(),
        }
    }

    fn with(path: impl Into<PathBuf>, contents: impl Into<String>) -> Self {
        let mut files = HashMap::new();
        files.insert(path.into(), contents.into());
        Self { files }
    }
}

impl ReadFile for MemoryFs {
    fn read(&self, path: &Path) -> Result<String, Error> {
        match self.files.get(path) {
            Some(contents) => Ok(contents.clone()),
            None => Err(Error::new(ErrorKind::NotFound, "file not found")),
        }
    }
}

#[test]
fn an_explicit_config_file_overrides_one_binding_and_leaves_the_rest() {
    let path = PathBuf::from("/cfg/tuivir.toml");
    let env = Env {
        config_file: Some(path.clone()),
        ..Default::default()
    };
    let fs = MemoryFs::with(path, "[keybindings]\n\"resource_delete\" = [\"x\"]\n");

    let registry = load(&env, &fs).expect("a valid configuration");

    assert_eq!(
        registry.resolve(CommandScope::ResourceView, key("x")),
        Some(Command::Resource(ResourceCommand::Delete)),
        "the configured key now invokes Delete"
    );
    assert_eq!(
        registry.resolve(CommandScope::ResourceView, key("d")),
        None,
        "the replaced default key no longer invokes Delete"
    );
    assert_eq!(
        registry.resolve(CommandScope::ResourceView, key("r")),
        Some(Command::Resource(ResourceCommand::Restart)),
        "an unmentioned Command keeps its defaults"
    );
}

#[test]
fn resource_shell_controls_layer_over_their_defaults() {
    let path = PathBuf::from("/cfg/tuivir.toml");
    let env = Env {
        config_file: Some(path.clone()),
        ..Default::default()
    };
    let fs = MemoryFs::with(
        path,
        "[resource_shell]\nprefix = \"f5\"\n\
         [resource_shell.keybindings]\n\
         focus_tuivir = [\"x\", \"q\"]\n\
         toggle_zoom = []\n",
    );

    let registry = load(&env, &fs).expect("a valid configuration");

    assert_eq!(
        registry.resource_shell_controls(),
        &ResourceShellControls::new(key("f5"), vec![key("x"), key("q")], vec![])
    );
}

#[test]
fn invalid_resource_shell_controls_reject_the_entire_configuration() {
    let path = PathBuf::from("/cfg/tuivir.toml");
    let env = Env {
        config_file: Some(path.clone()),
        ..Default::default()
    };
    let fs = MemoryFs::with(
        path.clone(),
        "[resource_shell]\nprefix = \"f13\"\n\
         [resource_shell.keybindings]\n\
         focus_tuivir = []\n\
         toggle_zoom = [\"z\", \"z\"]\n",
    );

    assert_eq!(
        load(&env, &fs).unwrap_err(),
        LoadError::Invalid {
            path,
            errors: vec![
                ConfigError::InvalidShellPrefix {
                    key: "f13".to_owned(),
                },
                ConfigError::EmptyShellKeybinding {
                    id: "focus_tuivir".to_owned(),
                },
                ConfigError::DuplicateShellKey {
                    id: "toggle_zoom".to_owned(),
                    key: "z".to_owned(),
                },
            ],
        }
    );
}

#[test]
fn shell_control_conflicts_and_prefix_collisions_are_reported_together() {
    let path = PathBuf::from("/cfg/tuivir.toml");
    let env = Env {
        config_file: Some(path.clone()),
        ..Default::default()
    };
    let fs = MemoryFs::with(
        path.clone(),
        "[resource_shell]\nprefix = \"x\"\n\
         [resource_shell.keybindings]\n\
         focus_tuivir = [\"x\", \"q\"]\n\
         toggle_zoom = [\"q\"]\n",
    );

    assert_eq!(
        load(&env, &fs).unwrap_err(),
        LoadError::Invalid {
            path,
            errors: vec![
                ConfigError::ShellPrefixCollision {
                    id: "focus_tuivir".to_owned(),
                    key: "x".to_owned(),
                },
                ConfigError::ConflictingShellKey {
                    key: "q".to_owned(),
                    first: "focus_tuivir".to_owned(),
                    second: "toggle_zoom".to_owned(),
                },
            ],
        }
    );
}

#[test]
fn an_empty_explicit_config_file_means_compiled_defaults() {
    let path = PathBuf::from("/cfg/tuivir.toml");
    let env = Env {
        config_file: Some(path.clone()),
        ..Default::default()
    };
    let fs = MemoryFs::with(path, "");

    let registry = load(&env, &fs).expect("an empty file is valid");

    assert_eq!(
        registry.resolve(CommandScope::ResourceView, key("r")),
        Some(Command::Resource(ResourceCommand::Restart))
    );
}

#[test]
fn a_missing_explicit_file_is_fatal() {
    let path = PathBuf::from("/cfg/missing.toml");
    let env = Env {
        config_file: Some(path.clone()),
        ..Default::default()
    };

    assert_eq!(
        load(&env, &MemoryFs::empty()).unwrap_err(),
        LoadError::ExplicitMissing { path }
    );
}

#[test]
fn a_relative_explicit_file_is_fatal() {
    let path = PathBuf::from("cfg/missing.toml");
    let env = Env {
        config_file: Some(path.clone()),
        ..Default::default()
    };

    assert_eq!(
        load(&env, &MemoryFs::empty()).unwrap_err(),
        LoadError::ExplicitNotAbsolute { path }
    );
}

#[test]
fn xdg_config_home_is_used_when_no_explicit_file_is_set() {
    let path = PathBuf::from("/xdg/tuivir/config.toml");
    let env = Env {
        xdg_config_home: Some(PathBuf::from("/xdg")),
        ..Default::default()
    };
    let fs = MemoryFs::with(path, "[keybindings]\n\"resource_restart\" = [\"x\"]\n");

    let registry = load(&env, &fs).expect("a valid configuration");

    assert_eq!(
        registry.resolve(CommandScope::ResourceView, key("x")),
        Some(Command::Resource(ResourceCommand::Restart))
    );
    assert_eq!(
        registry.resolve(CommandScope::ResourceView, key("r")),
        None,
        "the configured key list replaces the default"
    );
}

#[test]
fn home_config_is_used_when_xdg_is_unset() {
    let path = PathBuf::from("/home/me/.config/tuivir/config.toml");
    let env = Env {
        home: Some(PathBuf::from("/home/me")),
        ..Default::default()
    };
    let fs = MemoryFs::with(path, "[keybindings]\n\"resource_restart\" = [\"x\"]\n");

    let registry = load(&env, &fs).expect("a valid configuration");

    assert_eq!(
        registry.resolve(CommandScope::ResourceView, key("x")),
        Some(Command::Resource(ResourceCommand::Restart))
    );
}

/// A relative `XDG_CONFIG_HOME` names no single file, so Tuivir refuses to
/// guess which one the user meant rather than quietly reading a different one.
#[test]
fn a_relative_xdg_config_home_is_fatal() {
    let env = Env {
        xdg_config_home: Some(PathBuf::from("relative-xdg")),
        home: Some(PathBuf::from("/home/me")),
        ..Default::default()
    };
    let fs = MemoryFs::with(
        "/home/me/.config/tuivir/config.toml",
        "[keybindings]\n\"resource_restart\" = [\"x\"]\n",
    );

    assert_eq!(
        load(&env, &fs).unwrap_err(),
        LoadError::XdgNotAbsolute {
            path: PathBuf::from("relative-xdg")
        },
        "a relative XDG_CONFIG_HOME is reported rather than falling back to home"
    );
}

/// An exported-but-empty variable is how a shell spells "unset", and the
/// process environment cannot tell Tuivir the difference. Treating it as a
/// relative path would refuse to start over a variable that selects nothing.
#[test]
fn an_empty_xdg_config_home_means_unset_rather_than_relative() {
    let path = PathBuf::from("/home/me/.config/tuivir/config.toml");
    let env = Env {
        xdg_config_home: Some(PathBuf::new()),
        home: Some(PathBuf::from("/home/me")),
        ..Default::default()
    };
    let fs = MemoryFs::with(path, "[keybindings]\n\"resource_restart\" = [\"x\"]\n");

    let registry = load(&env, &fs).expect("an empty XDG_CONFIG_HOME falls back to home");

    assert_eq!(
        registry.resolve(CommandScope::ResourceView, key("x")),
        Some(Command::Resource(ResourceCommand::Restart))
    );
}

/// `TUIVIR_CONFIG_FILE` selects one exact file, so discovery never runs and a
/// broken `XDG_CONFIG_HOME` cannot stop a run that does not consult it.
#[test]
fn an_explicit_file_is_unaffected_by_a_relative_xdg_config_home() {
    let path = PathBuf::from("/cfg/tuivir.toml");
    let env = Env {
        config_file: Some(path.clone()),
        xdg_config_home: Some(PathBuf::from("relative-xdg")),
        home: Some(PathBuf::from("/home/me")),
    };
    let fs = MemoryFs::with(path, "[keybindings]\n\"resource_restart\" = [\"x\"]\n");

    let registry = load(&env, &fs).expect("a valid configuration");

    assert_eq!(
        registry.resolve(CommandScope::ResourceView, key("x")),
        Some(Command::Resource(ResourceCommand::Restart))
    );
}

#[test]
fn a_missing_discovered_file_means_compiled_defaults_and_creates_nothing() {
    let env = Env {
        xdg_config_home: Some(PathBuf::from("/xdg")),
        ..Default::default()
    };
    let fs = MemoryFs::empty();

    let registry = load(&env, &fs).expect("a missing discovered file is not an error");

    assert_eq!(
        registry.resolve(CommandScope::ResourceView, key("r")),
        Some(Command::Resource(ResourceCommand::Restart)),
        "compiled defaults apply, and no file is created"
    );
}

#[test]
fn nothing_is_configured_and_nothing_exists_so_compiled_defaults_apply() {
    let env = Env::default();
    let registry = load(&env, &MemoryFs::empty()).expect("no configuration is valid");

    assert_eq!(
        registry.resolve(CommandScope::ResourceView, key("r")),
        Some(Command::Resource(ResourceCommand::Restart))
    );
}

#[test]
fn a_file_that_is_not_valid_toml_is_reported_with_its_path() {
    let path = PathBuf::from("/cfg/tuivir.toml");
    let env = Env {
        config_file: Some(path.clone()),
        ..Default::default()
    };
    let fs = MemoryFs::with(path, "[keybindings\n");

    let error = load(&env, &fs).unwrap_err();
    let LoadError::Unparsable {
        path: error_path, ..
    } = error
    else {
        panic!("expected an unparsable file, got {error:?}");
    };
    assert_eq!(error_path, PathBuf::from("/cfg/tuivir.toml"));
}

/// A field Tuivir does not understand is a typo or a setting from a different
/// tool. Ignoring it would leave the user believing it took effect.
#[test]
fn an_unknown_field_is_rejected_and_names_itself() {
    let path = PathBuf::from("/cfg/tuivir.toml");
    let env = Env {
        config_file: Some(path.clone()),
        ..Default::default()
    };
    let fs = MemoryFs::with(
        path,
        "theme = \"dark\"\n\n[keybindings]\n\"app_quit\" = [\"q\"]\n",
    );

    let error = load(&env, &fs).unwrap_err();
    let LoadError::Unparsable { path, message } = error else {
        panic!("expected an unknown field to be rejected, got {error:?}");
    };
    assert_eq!(path, PathBuf::from("/cfg/tuivir.toml"));
    assert!(
        message.contains("theme"),
        "the diagnostic must name the field the user wrote, got {message:?}"
    );
}

#[test]
fn an_unknown_command_id_is_rejected() {
    let path = PathBuf::from("/cfg/tuivir.toml");
    let env = Env {
        config_file: Some(path.clone()),
        ..Default::default()
    };
    let fs = MemoryFs::with(path, "[keybindings]\n\"no_such_command\" = [\"x\"]\n");

    assert_eq!(
        load(&env, &fs).unwrap_err(),
        LoadError::Invalid {
            path: PathBuf::from("/cfg/tuivir.toml"),
            errors: vec![ConfigError::UnknownCommand {
                id: "no_such_command".to_owned()
            }]
        }
    );
}

#[test]
fn an_unrecognised_key_is_rejected() {
    let path = PathBuf::from("/cfg/tuivir.toml");
    let env = Env {
        config_file: Some(path.clone()),
        ..Default::default()
    };
    let fs = MemoryFs::with(path, "[keybindings]\n\"resource_restart\" = [\"f13\"]\n");

    assert_eq!(
        load(&env, &fs).unwrap_err(),
        LoadError::Invalid {
            path: PathBuf::from("/cfg/tuivir.toml"),
            errors: vec![ConfigError::InvalidKey {
                id: "resource_restart".to_owned(),
                key: "f13".to_owned()
            }]
        }
    );
}

#[test]
fn claiming_ctrl_c_for_another_command_is_rejected() {
    let path = PathBuf::from("/cfg/tuivir.toml");
    let env = Env {
        config_file: Some(path.clone()),
        ..Default::default()
    };
    let fs = MemoryFs::with(path, "[keybindings]\n\"resource_delete\" = [\"ctrl+c\"]\n");

    assert_eq!(
        load(&env, &fs).unwrap_err(),
        LoadError::Invalid {
            path: PathBuf::from("/cfg/tuivir.toml"),
            errors: vec![ConfigError::ReservedKey {
                id: "resource_delete".to_owned(),
                key: "ctrl+c".to_owned()
            }]
        }
    );
}

/// Validation is atomic: one run reports every problem it can find, so fixing
/// the file is not a game of one diagnostic per restart.
///
/// Because the whole load fails, no registry ever exists to dispatch a key or
/// to generate an inline hint from — a rejected file cannot half-apply.
#[test]
fn every_discoverable_failure_is_reported_together() {
    let path = PathBuf::from("/cfg/tuivir.toml");
    let env = Env {
        config_file: Some(path.clone()),
        ..Default::default()
    };
    let fs = MemoryFs::with(
        path,
        "[keybindings]\n\
         \"no_such_command\" = [\"x\"]\n\
         \"resource_restart\" = [\"f13\"]\n\
         \"resource_stop\" = [\"j\"]\n",
    );

    let error = load(&env, &fs).unwrap_err();
    let LoadError::Invalid { errors, .. } = &error else {
        panic!("expected an invalid configuration, got {error:?}");
    };

    for expected in [
        ConfigError::UnknownCommand {
            id: "no_such_command".to_owned(),
        },
        ConfigError::InvalidKey {
            id: "resource_restart".to_owned(),
            key: "f13".to_owned(),
        },
        ConfigError::ConflictingKey {
            key: "j".to_owned(),
            first: "selection_next".to_owned(),
            second: "resource_stop".to_owned(),
        },
    ] {
        assert!(
            errors.contains(&expected),
            "expected {expected:?} among {errors:?}"
        );
    }
    assert_eq!(errors.len(), 3, "and nothing beyond them: {errors:?}");
}

/// An uppercase Command ID is a typo rather than a second spelling: IDs are
/// lowercase and case-sensitive, so it names no Command Tuivir registers.
#[test]
fn an_uppercase_command_id_is_rejected() {
    let path = PathBuf::from("/cfg/tuivir.toml");
    let env = Env {
        config_file: Some(path.clone()),
        ..Default::default()
    };
    let fs = MemoryFs::with(path, "[keybindings]\n\"Resource_Delete\" = [\"x\"]\n");

    assert_eq!(
        load(&env, &fs).unwrap_err(),
        LoadError::Invalid {
            path: PathBuf::from("/cfg/tuivir.toml"),
            errors: vec![ConfigError::UnknownCommand {
                id: "Resource_Delete".to_owned()
            }]
        }
    );
}

/// An explicit override must name a readable regular file. A directory is
/// absolute and exists, so only actually reading it tells the user why their
/// selected configuration produced nothing.
#[test]
fn an_explicit_file_that_is_a_directory_is_fatal() {
    let path = std::env::temp_dir().join("tuivir-config-is-a-directory");
    std::fs::create_dir_all(&path).expect("a directory to point the override at");
    let env = Env {
        config_file: Some(path.clone()),
        ..Default::default()
    };

    let error = load(&env, &FileSystemReader);
    let _ = std::fs::remove_dir(&path);

    assert_eq!(error.unwrap_err(), LoadError::Unreadable { path });
}

/// The production filesystem adapter reads an actual file, so startup does not
/// need a second reader implementation.
#[test]
fn the_real_filesystem_reader_reads_a_file_from_disk() {
    let path = std::env::temp_dir().join("tuivir-fsreader-test.toml");
    std::fs::write(&path, "[keybindings]\n\"resource_restart\" = [\"x\"]\n").unwrap();
    let env = Env {
        config_file: Some(path.clone()),
        ..Default::default()
    };

    let registry = load(&env, &FileSystemReader);
    let _ = std::fs::remove_file(&path);
    let registry = registry.expect("reads the real file from disk");

    assert_eq!(
        registry.resolve(CommandScope::ResourceView, key("x")),
        Some(Command::Resource(ResourceCommand::Restart))
    );
}
