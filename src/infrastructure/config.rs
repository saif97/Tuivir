use std::{
    collections::BTreeMap,
    fmt, io,
    path::{Path, PathBuf},
};

use serde::Deserialize;

use crate::application::{CommandRegistry, KeybindingError};

/// The environment Tuivir consults to discover its configuration file.
///
/// Real startup builds this from `TUIVIR_CONFIG_FILE`, `XDG_CONFIG_HOME`, and
/// `HOME`; tests inject it so they never depend on a real home directory.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Env {
    /// `TUIVIR_CONFIG_FILE`: one absolute file, with highest precedence.
    pub config_file: Option<PathBuf>,
    /// `$XDG_CONFIG_HOME`: only consulted when it is absolute.
    pub xdg_config_home: Option<PathBuf>,
    /// `$HOME`: the base for the `~/.config` fallback.
    pub home: Option<PathBuf>,
}

impl Env {
    /// Builds the environment from the real process environment.
    pub fn from_environment() -> Self {
        Self {
            config_file: std::env::var_os("TUIVIR_CONFIG_FILE").map(PathBuf::from),
            xdg_config_home: std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from),
            home: std::env::var_os("HOME").map(PathBuf::from),
        }
    }
}

/// Reads a configuration file's contents, isolated so tests can supply an
/// in-memory filesystem.
pub trait ReadFile {
    fn read(&self, path: &Path) -> io::Result<String>;
}

/// Reads configuration from the real filesystem.
pub struct FileSystemReader;

impl ReadFile for FileSystemReader {
    fn read(&self, path: &Path) -> io::Result<String> {
        std::fs::read_to_string(path)
    }
}

/// Why a configuration file could not be loaded as an effective registry.
///
/// A missing *discovered* file is not an error — it means compiled defaults —
/// so it never appears here.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LoadError {
    /// `TUIVIR_CONFIG_FILE` names a path that is not absolute.
    ExplicitNotAbsolute { path: PathBuf },
    /// `XDG_CONFIG_HOME` is set to a path that is not absolute.
    XdgNotAbsolute { path: PathBuf },
    /// The explicitly selected file does not exist.
    ExplicitMissing { path: PathBuf },
    /// The selected file exists but could not be read.
    Unreadable { path: PathBuf },
    /// The file could not be parsed as the configuration format.
    Unparsable { path: PathBuf, message: String },
    /// The file parsed, but its keybindings were rejected.
    Invalid {
        path: PathBuf,
        errors: Vec<KeybindingError>,
    },
}

impl fmt::Display for LoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ExplicitNotAbsolute { path } => write!(
                formatter,
                "TUIVIR_CONFIG_FILE must be an absolute path, not {}",
                path.display()
            ),
            Self::XdgNotAbsolute { path } => write!(
                formatter,
                "XDG_CONFIG_HOME must be an absolute path, not {}",
                path.display()
            ),
            Self::ExplicitMissing { path } => {
                write!(
                    formatter,
                    "Configuration file does not exist: {}",
                    path.display()
                )
            }
            Self::Unreadable { path } => write!(
                formatter,
                "Configuration file could not be read: {}",
                path.display()
            ),
            Self::Unparsable { path, message } => {
                write!(formatter, "Could not parse {}: {}", path.display(), message)
            }
            Self::Invalid { path, errors } => {
                write!(formatter, "Invalid configuration in {}:", path.display())?;
                for error in errors {
                    write!(formatter, "\n  {error}")?;
                }
                Ok(())
            }
        }
    }
}

/// Loads configuration once, layering any `[keybindings]` over the compiled
/// defaults.
///
/// With no explicit file and no discoverable file, Tuivir uses the defaults
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
        None => match discovered_path(env)? {
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
            // A missing discovered file means compiled defaults, and Tuivir
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
///
/// A set-but-relative `XDG_CONFIG_HOME` is fatal rather than ignored: it names
/// no single file, and silently reading a different one would apply bindings
/// the user never selected.
fn discovered_path(env: &Env) -> Result<Option<PathBuf>, LoadError> {
    // An exported-but-empty variable is a shell's way of saying "unset", which
    // the process environment cannot distinguish, so it selects nothing rather
    // than naming a relative path.
    if let Some(xdg) = env
        .xdg_config_home
        .as_ref()
        .filter(|xdg| !xdg.as_os_str().is_empty())
    {
        if !xdg.is_absolute() {
            return Err(LoadError::XdgNotAbsolute { path: xdg.clone() });
        }
        return Ok(Some(xdg.join("tuivir").join("config.toml")));
    }
    Ok(env
        .home
        .as_ref()
        .map(|home| home.join(".config").join("tuivir").join("config.toml")))
}

/// The file's shape. Unknown fields are refused so a typo or a setting meant
/// for another tool is reported rather than silently ignored.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
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
