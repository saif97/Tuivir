use std::{
    collections::BTreeMap,
    fmt,
    io,
    path::{Path, PathBuf},
};

use serde::Deserialize;

use crate::command::CommandRegistry;

/// One reason Virtui refused a configuration file.
///
/// Validation is atomic: every discoverable diagnostic is collected and
/// reported, and no part of an invalid configuration is applied.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConfigError {
    /// A `[keybindings]` entry names a Command Virtui does not register.
    UnknownCommand { id: String },
    /// A key string Virtui cannot represent.
    InvalidKey { id: String, key: String },
    /// A Command other than Quit tried to claim the emergency `ctrl+c`.
    ReservedKey { id: String, key: String },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownCommand { id } => {
                write!(formatter, "[keybindings] has no Command named \"{id}\"")
            }
            Self::InvalidKey { id, key } => write!(
                formatter,
                "[keybindings] \"{id}\" has an unrecognised key \"{key}\""
            ),
            Self::ReservedKey { id, key } => write!(
                formatter,
                "[keybindings] \"{id}\" cannot claim \"{key}\": it always quits Virtui"
            ),
        }
    }
}

/// The environment Virtui consults to discover its configuration file.
///
/// Real startup builds this from `VIRTUI_CONFIG_FILE`, `XDG_CONFIG_HOME`, and
/// `HOME`; tests inject it so they never depend on a real home directory.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Env {
    /// `VIRTUI_CONFIG_FILE`: one absolute file, with highest precedence.
    pub config_file: Option<PathBuf>,
    /// `$XDG_CONFIG_HOME`: only consulted when it is absolute.
    pub xdg_config_home: Option<PathBuf>,
    /// `$HOME`: the base for the `~/.config` fallback.
    pub home: Option<PathBuf>,
}

/// Reads a configuration file's contents, isolated so tests can supply an
/// in-memory filesystem.
pub trait ReadFile {
    fn read(&self, path: &Path) -> io::Result<String>;
}

/// Why a configuration file could not be loaded as an effective registry.
///
/// A missing *discovered* file is not an error — it means compiled defaults —
/// so it never appears here.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LoadError {
    /// `VIRTUI_CONFIG_FILE` names a path that is not absolute.
    ExplicitNotAbsolute { path: PathBuf },
    /// The explicitly selected file does not exist.
    ExplicitMissing { path: PathBuf },
    /// The selected file exists but could not be read.
    Unreadable { path: PathBuf },
    /// The file could not be parsed as the configuration format.
    Unparsable { path: PathBuf, message: String },
    /// The file parsed, but its keybindings were rejected.
    Invalid { path: PathBuf, errors: Vec<ConfigError> },
}

/// Loads configuration once, layering any `[keybindings]` over the compiled
/// defaults.
///
/// With no explicit file and no discoverable file, Virtui uses the defaults
/// and creates nothing.
pub fn load(env: &Env, reader: &dyn ReadFile) -> Result<CommandRegistry, LoadError> {
    // An explicit file always wins and must be absolute; a discovered file is
    // optional, so its absence is not an error.
    let (path, explicit) = match env.config_file.clone() {
        Some(path) => {
            if !path.is_absolute() {
                return Err(LoadError::ExplicitNotAbsolute { path });
            }
            (path, true)
        }
        None => match discovered_path(env) {
            Some(path) => (path, false),
            None => return Ok(CommandRegistry::builtin()),
        },
    };
    let contents = match reader.read(&path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            if explicit {
                return Err(LoadError::ExplicitMissing { path });
            }
            // A missing discovered file means compiled defaults, and Virtui
            // creates nothing and searches no project directory.
            return Ok(CommandRegistry::builtin());
        }
        Err(_) => return Err(LoadError::Unreadable { path }),
    };
    if contents.trim().is_empty() {
        return Ok(CommandRegistry::builtin());
    }
    let raw: RawConfig = match toml::from_str(&contents) {
        Ok(raw) => raw,
        Err(error) => {
            return Err(LoadError::Unparsable {
                path,
                message: error.to_string(),
            });
        }
    };
    let overrides = raw.into_overrides();
    CommandRegistry::effective(&overrides).map_err(|errors| LoadError::Invalid { path, errors })
}

/// Selects exactly one discovered path, or `None` when neither `XDG_CONFIG_HOME`
/// nor `HOME` names a directory.
fn discovered_path(env: &Env) -> Option<PathBuf> {
    if let Some(xdg) = &env.xdg_config_home
        && xdg.is_absolute()
    {
        return Some(xdg.join("virtui").join("config.toml"));
    }
    env.home
        .as_ref()
        .map(|home| home.join(".config").join("virtui").join("config.toml"))
}

#[derive(Deserialize)]
struct RawConfig {
    keybindings: Option<BTreeMap<String, Vec<String>>>,
}

impl RawConfig {
    fn into_overrides(self) -> Vec<(String, Vec<String>)> {
        self.keybindings
            .map(|table| table.into_iter().collect())
            .unwrap_or_default()
    }
}
