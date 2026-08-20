//! Durable Pane Boundary preferences, exercised without a real home directory.

use std::{
    cell::RefCell,
    collections::HashMap,
    io::{Error, ErrorKind},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use tuivir::{
    application::PaneBoundary,
    infrastructure::{
        config::{FileSystemReader, ReadFile},
        pane_boundary_state::{Env, StateStorage, load, save},
    },
};

struct MemoryState {
    files: RefCell<HashMap<PathBuf, String>>,
}

struct FailingState {
    files: RefCell<HashMap<PathBuf, String>>,
}

impl ReadFile for FailingState {
    fn read(&self, path: &Path) -> Result<String, Error> {
        self.files
            .borrow()
            .get(path)
            .cloned()
            .ok_or_else(|| Error::new(ErrorKind::NotFound, "missing state"))
    }
}

impl StateStorage for FailingState {
    fn write_atomically(&self, _: &Path, _: &str) -> Result<(), Error> {
        Err(Error::other("disk full"))
    }
}

impl MemoryState {
    fn empty() -> Self {
        Self {
            files: RefCell::new(HashMap::new()),
        }
    }

    fn with(path: impl Into<PathBuf>, contents: impl Into<String>) -> Self {
        let state = Self::empty();
        state
            .files
            .borrow_mut()
            .insert(path.into(), contents.into());
        state
    }
}

impl ReadFile for MemoryState {
    fn read(&self, path: &Path) -> Result<String, Error> {
        self.files
            .borrow()
            .get(path)
            .cloned()
            .ok_or_else(|| Error::new(ErrorKind::NotFound, "missing state"))
    }
}

impl StateStorage for MemoryState {
    fn write_atomically(&self, path: &Path, contents: &str) -> Result<(), Error> {
        self.files
            .borrow_mut()
            .insert(path.to_path_buf(), contents.to_owned());
        Ok(())
    }
}

#[test]
fn a_saved_pane_boundary_is_restored_from_xdg_state() {
    let path = PathBuf::from("/state/tuivir/pane-boundary.json");
    let state = MemoryState::with(&path, r#"{"resources_percent": 60}"#);
    let env = Env {
        xdg_state_home: Some(PathBuf::from("/state")),
        ..Env::default()
    };

    assert_eq!(load(&env, &state), PaneBoundary::new(60));
}

#[test]
fn absent_state_keeps_the_compiled_pane_boundary_default() {
    let env = Env {
        xdg_state_home: Some(PathBuf::from("/state")),
        ..Env::default()
    };

    assert_eq!(load(&env, &MemoryState::empty()), PaneBoundary::default());
}

#[test]
fn an_invalid_saved_dimension_falls_back_safely() {
    let state = MemoryState::with(
        "/state/tuivir/pane-boundary.json",
        r#"{"resources_percent": 101}"#,
    );
    let env = Env {
        xdg_state_home: Some(PathBuf::from("/state")),
        ..Env::default()
    };

    assert_eq!(load(&env, &state), PaneBoundary::default());
}

#[test]
fn saving_a_resized_pane_boundary_uses_the_state_path() {
    let state = MemoryState::empty();
    let env = Env {
        home: Some(PathBuf::from("/home/me")),
        ..Env::default()
    };

    save(&env, &state, PaneBoundary::new(60)).expect("the preference saves");

    assert_eq!(
        state.files.borrow().get(&PathBuf::from(
            "/home/me/.local/state/tuivir/pane-boundary.json"
        )),
        Some(&r#"{"resources_percent":60}"#.to_owned())
    );
}

#[test]
fn a_failed_atomic_save_leaves_the_last_complete_preference_intact() {
    let path = PathBuf::from("/state/tuivir/pane-boundary.json");
    let state = FailingState {
        files: RefCell::new(HashMap::from([(
            path.clone(),
            r#"{"resources_percent":48}"#.into(),
        )])),
    };
    let env = Env {
        xdg_state_home: Some(PathBuf::from("/state")),
        ..Env::default()
    };

    assert!(save(&env, &state, PaneBoundary::new(60)).is_err());
    assert_eq!(
        state.files.borrow().get(&path),
        Some(&r#"{"resources_percent":48}"#.to_owned())
    );
}

#[test]
fn the_filesystem_adapter_replaces_state_without_leaving_a_temporary_file() {
    let directory = std::env::temp_dir().join(format!(
        "tuivir-state-test-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("the clock has an epoch")
            .as_nanos()
    ));
    let env = Env {
        xdg_state_home: Some(directory.clone()),
        ..Env::default()
    };

    save(&env, &FileSystemReader, PaneBoundary::new(60)).expect("an atomic state write");

    let state_directory = directory.join("tuivir");
    assert_eq!(
        std::fs::read_to_string(state_directory.join("pane-boundary.json")).unwrap(),
        r#"{"resources_percent":60}"#
    );
    assert!(
        std::fs::read_dir(&state_directory)
            .unwrap()
            .all(|entry| !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".tmp")),
        "the rename left no partially-written temporary state behind"
    );

    std::fs::remove_dir_all(directory).expect("remove the test state directory");
}
