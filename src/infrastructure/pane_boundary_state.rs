//! The durable, runtime-owned Pane Boundary preference.
//!
//! It deliberately lives in the XDG state directory rather than the user's
//! configuration file: resizing is application state, not hand-authored setup.

use std::{
    io,
    path::{Path, PathBuf},
};

use serde::Deserialize;

use crate::{
    application::PaneBoundary,
    infrastructure::config::{FileSystemReader, ReadFile},
};

/// Environment values used to locate Tuivir's XDG state file.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Env {
    pub xdg_state_home: Option<PathBuf>,
    pub home: Option<PathBuf>,
}

impl Env {
    pub fn from_environment() -> Self {
        Self {
            xdg_state_home: std::env::var_os("XDG_STATE_HOME").map(PathBuf::from),
            home: std::env::var_os("HOME").map(PathBuf::from),
        }
    }
}

/// The host filesystem operations needed by state loading and saving.
pub trait StateStorage: ReadFile {
    fn write_atomically(&self, path: &Path, contents: &str) -> io::Result<()>;
}

impl StateStorage for FileSystemReader {
    fn write_atomically(&self, path: &Path, contents: &str) -> io::Result<()> {
        let parent = path.parent().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "state path has no parent directory",
            )
        })?;
        std::fs::create_dir_all(parent)?;
        let temporary = parent.join(format!(".pane-boundary-{}.tmp", std::process::id()));
        std::fs::write(&temporary, contents)?;
        std::fs::rename(temporary, path)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedPaneBoundary {
    resources_percent: u16,
}

/// Restores the last valid preference, never allowing obsolete state to block
/// startup or overwrite the compiled default.
pub fn load(env: &Env, storage: &dyn StateStorage) -> PaneBoundary {
    let Some(path) = state_path(env) else {
        return PaneBoundary::default();
    };
    let Ok(contents) = storage.read(&path) else {
        return PaneBoundary::default();
    };
    let Ok(saved) = serde_json::from_str::<SavedPaneBoundary>(&contents) else {
        return PaneBoundary::default();
    };
    if !PaneBoundary::is_valid_percent(saved.resources_percent) {
        return PaneBoundary::default();
    }
    PaneBoundary::new(saved.resources_percent)
}

/// Atomically saves only a user-selected Pane Boundary.  Launching never
/// calls this function, so it does not create a state file by itself.
pub fn save(env: &Env, storage: &dyn StateStorage, boundary: PaneBoundary) -> io::Result<()> {
    let Some(path) = state_path(env) else {
        return Ok(());
    };
    storage.write_atomically(
        &path,
        &format!(
            r#"{{"resources_percent":{}}}"#,
            boundary.resources_percent()
        ),
    )
}

fn state_path(env: &Env) -> Option<PathBuf> {
    let root = env
        .xdg_state_home
        .as_ref()
        .filter(|path| path.is_absolute())
        .cloned()
        .or_else(|| env.home.as_ref().map(|home| home.join(".local/state")))?;
    Some(root.join("tuivir").join("pane-boundary.json"))
}
