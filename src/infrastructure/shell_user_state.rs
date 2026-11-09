//! User-selected accounts belong in XDG state, alongside Pane Boundary state.
use crate::{
    application::{DefaultShellUser, ShellUserKey},
    domain::{ProviderId, ResourceId, ResourcePanelId, ResourceTarget, TargetEnvironment},
    infrastructure::pane_boundary_state::{Env, StateStorage, state_path},
};
use serde::{Deserialize, Serialize};
use std::io;

#[derive(Deserialize, Serialize)]
struct SavedUser {
    provider: String,
    environment: Option<String>,
    panel: String,
    resource: String,
    username: String,
}

pub fn load(env: &Env, storage: &dyn StateStorage) -> Vec<DefaultShellUser> {
    let Some(path) = state_path(env, "shell-users.json") else {
        return Vec::new();
    };
    let Ok(contents) = storage.read(&path) else {
        return Vec::new();
    };
    let Ok(saved) = serde_json::from_str::<Vec<SavedUser>>(&contents) else {
        return Vec::new();
    };
    saved
        .into_iter()
        .map(|entry| DefaultShellUser {
            key: ShellUserKey {
                provider_id: ProviderId::new(entry.provider),
                target_environment: entry.environment.map(TargetEnvironment::new),
                target: ResourceTarget::new(
                    ResourcePanelId::new(entry.panel),
                    ResourceId::new(entry.resource),
                ),
            },
            username: entry.username,
        })
        .collect()
}

pub fn save(env: &Env, storage: &dyn StateStorage, users: &[DefaultShellUser]) -> io::Result<()> {
    let path = state_path(env, "shell-users.json").ok_or_else(|| {
        io::Error::other("No home or XDG state directory for Default Shell Users")
    })?;
    let saved: Vec<_> = users
        .iter()
        .map(|user| SavedUser {
            provider: user.key.provider_id.to_string(),
            environment: user
                .key
                .target_environment
                .as_ref()
                .map(ToString::to_string),
            panel: user.key.target.panel_id().to_string(),
            resource: user.key.target.resource_id().to_string(),
            username: user.username.clone(),
        })
        .collect();
    storage.write_atomically(&path, &serde_json::to_string(&saved)?)
}
