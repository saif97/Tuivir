//! Durable Pane Boundary preferences, exercised without a real home directory.

use std::{
    cell::RefCell,
    collections::HashMap,
    io::{Error, ErrorKind},
    path::{Path, PathBuf},
};

use tuivir::{
    application::PaneBoundary,
    infrastructure::pane_boundary_state::{Env, StateStorage, load, save},
};

struct MemoryState {
    files: RefCell<HashMap<PathBuf, String>>,
}

impl MemoryState {
    fn empty() -> Self {
        Self {
            files: RefCell::new(HashMap::new()),
        }
    }

    fn with(path: impl Into<PathBuf>, contents: impl Into<String>) -> Self {
        let state = Self::empty();
        state.files.borrow_mut().insert(path.into(), contents.into());
        state
    }
}

impl StateStorage for MemoryState {
    fn read(&self, path: &Path) -> Result<String, Error> {
        self.files
            .borrow()
            .get(path)
            .cloned()
            .ok_or_else(|| Error::new(ErrorKind::NotFound, "missing state"))
    }

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
        state
            .files
            .borrow()
            .get(&PathBuf::from("/home/me/.local/state/tuivir/pane-boundary.json")),
        Some(&r#"{"resources_percent":60}"#.to_owned())
    );
}
