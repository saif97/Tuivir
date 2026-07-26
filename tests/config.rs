use std::{
    collections::HashMap,
    io::{Error, ErrorKind},
    path::{Path, PathBuf},
};

use virtui::{
    command::{Command, CommandScope},
    config::{ConfigError, Env, FileSystemReader, LoadError, ReadFile, load},
    keys::Key,
    provider::ResourceCommand,
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
    let path = PathBuf::from("/cfg/virtui.toml");
    let env = Env {
        config_file: Some(path.clone()),
        ..Default::default()
    };
    let fs = MemoryFs::with(path, "[keybindings]\n\"resource.delete\" = [\"x\"]\n");

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
fn an_empty_explicit_config_file_means_compiled_defaults() {
    let path = PathBuf::from("/cfg/virtui.toml");
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
    let path = PathBuf::from("/xdg/virtui/config.toml");
    let env = Env {
        xdg_config_home: Some(PathBuf::from("/xdg")),
        ..Default::default()
    };
    let fs = MemoryFs::with(path, "[keybindings]\n\"resource.restart\" = [\"x\"]\n");

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
    let path = PathBuf::from("/home/me/.config/virtui/config.toml");
    let env = Env {
        home: Some(PathBuf::from("/home/me")),
        ..Default::default()
    };
    let fs = MemoryFs::with(path, "[keybindings]\n\"resource.restart\" = [\"x\"]\n");

    let registry = load(&env, &fs).expect("a valid configuration");

    assert_eq!(
        registry.resolve(CommandScope::ResourceView, key("x")),
        Some(Command::Resource(ResourceCommand::Restart))
    );
}

/// A relative `XDG_CONFIG_HOME` is ignored, falling back to `~/.config` rather
/// than producing an unsafe path.
#[test]
fn a_relative_xdg_config_home_falls_back_to_home() {
    let path = PathBuf::from("/home/me/.config/virtui/config.toml");
    let env = Env {
        xdg_config_home: Some(PathBuf::from("relative-xdg")),
        home: Some(PathBuf::from("/home/me")),
        ..Default::default()
    };
    let fs = MemoryFs::with(path, "[keybindings]\n\"resource.restart\" = [\"x\"]\n");

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
    let path = PathBuf::from("/cfg/virtui.toml");
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
    assert_eq!(error_path, PathBuf::from("/cfg/virtui.toml"));
}

#[test]
fn an_unknown_command_id_is_rejected() {
    let path = PathBuf::from("/cfg/virtui.toml");
    let env = Env {
        config_file: Some(path.clone()),
        ..Default::default()
    };
    let fs = MemoryFs::with(path, "[keybindings]\n\"no.such.command\" = [\"x\"]\n");

    assert_eq!(
        load(&env, &fs).unwrap_err(),
        LoadError::Invalid {
            path: PathBuf::from("/cfg/virtui.toml"),
            errors: vec![ConfigError::UnknownCommand {
                id: "no.such.command".to_owned()
            }]
        }
    );
}

#[test]
fn an_unrecognised_key_is_rejected() {
    let path = PathBuf::from("/cfg/virtui.toml");
    let env = Env {
        config_file: Some(path.clone()),
        ..Default::default()
    };
    let fs = MemoryFs::with(path, "[keybindings]\n\"resource.restart\" = [\"f13\"]\n");

    assert_eq!(
        load(&env, &fs).unwrap_err(),
        LoadError::Invalid {
            path: PathBuf::from("/cfg/virtui.toml"),
            errors: vec![ConfigError::InvalidKey {
                id: "resource.restart".to_owned(),
                key: "f13".to_owned()
            }]
        }
    );
}

#[test]
fn claiming_ctrl_c_for_another_command_is_rejected() {
    let path = PathBuf::from("/cfg/virtui.toml");
    let env = Env {
        config_file: Some(path.clone()),
        ..Default::default()
    };
    let fs = MemoryFs::with(path, "[keybindings]\n\"resource.delete\" = [\"ctrl+c\"]\n");

    assert_eq!(
        load(&env, &fs).unwrap_err(),
        LoadError::Invalid {
            path: PathBuf::from("/cfg/virtui.toml"),
            errors: vec![ConfigError::ReservedKey {
                id: "resource.delete".to_owned(),
                key: "ctrl+c".to_owned()
            }]
        }
    );
}

/// The production filesystem adapter reads an actual file, so startup does not
/// need a second reader implementation.
#[test]
fn the_real_filesystem_reader_reads_a_file_from_disk() {
    let path = std::env::temp_dir().join("virtui-fsreader-test.toml");
    std::fs::write(&path, "[keybindings]\n\"resource.restart\" = [\"x\"]\n").unwrap();
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
