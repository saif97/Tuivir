use std::{sync::Arc, time::Duration};

use crossterm::event::KeyEvent;
use tokio::time::{Instant, Interval, MissedTickBehavior};

use crate::{
    app::{App, AppEvent},
    cli::CliRunner,
    command::Command,
    docker::DockerWorkspace,
    docker_sandbox::DockerSandboxWorkspace,
    incus::IncusWorkspace,
    keys::Key,
    provider::{ProviderDiscovery, ProviderId, ProviderRequest, ProviderWorkspace},
};

/// The cadence for refreshing the Active Workspace.
pub const REFRESH_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShellControl {
    Continue,
    Quit,
}

pub fn handle_key(app: &mut App, key: KeyEvent) -> (ShellControl, Vec<ProviderRequest>) {
    // Normalize the terminal event into the registry's vocabulary before any
    // routing: the application never sees a crossterm event for a Command.
    let Some(key) = Key::from_event(key) else {
        return (ShellControl::Continue, Vec::new());
    };
    // The emergency Quit is reserved by the registry and stays active in every
    // scope, so a stuck modal can always restore the terminal.
    if app.reserved(key) == Some(Command::Quit) {
        return (ShellControl::Quit, Vec::new());
    }
    match app.resolve_command(key) {
        Some(Command::Quit) => (ShellControl::Quit, Vec::new()),
        Some(command) => (ShellControl::Continue, app.invoke(command)),
        None => (ShellControl::Continue, Vec::new()),
    }
}

/// A refresh clock that skips missed ticks instead of queuing a backlog.
pub struct RefreshTimer {
    interval: Interval,
}

impl Default for RefreshTimer {
    fn default() -> Self {
        Self::new()
    }
}

impl RefreshTimer {
    pub fn new() -> Self {
        // Skip the immediate first tick: provider discovery already triggers an
        // initial refresh (see `App::handle_provider_discovered`), so ticking
        // right away here would just duplicate it.
        let mut interval =
            tokio::time::interval_at(Instant::now() + REFRESH_INTERVAL, REFRESH_INTERVAL);
        interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
        Self { interval }
    }

    pub async fn tick(&mut self) {
        self.interval.tick().await;
    }
}

/// Executes provider requests without giving background work access to `App`.
pub struct ProviderRuntime {
    /// Compiled-in workspaces in their stable provider-selector order.
    workspaces: Vec<(ProviderId, Arc<dyn ProviderWorkspace>)>,
    cli: Arc<dyn CliRunner>,
}

impl ProviderRuntime {
    pub fn with_builtin_providers(cli: Arc<dyn CliRunner>) -> Self {
        Self::new(
            vec![
                Arc::new(DockerWorkspace) as Arc<dyn ProviderWorkspace>,
                Arc::new(IncusWorkspace) as Arc<dyn ProviderWorkspace>,
                Arc::new(DockerSandboxWorkspace) as Arc<dyn ProviderWorkspace>,
            ],
            cli,
        )
    }

    pub fn new(workspaces: Vec<Arc<dyn ProviderWorkspace>>, cli: Arc<dyn CliRunner>) -> Self {
        Self {
            workspaces: workspaces
                .into_iter()
                .map(|workspace| (workspace.id(), workspace))
                .collect(),
            cli,
        }
    }

    /// Discovers installed providers in registration order through their
    /// provider-specific CLI logic.
    pub async fn discover(&self) -> Vec<ProviderDiscovery> {
        let mut discovered = Vec::new();
        for (_, workspace) in &self.workspaces {
            if let Some(provider) = workspace.discover(self.cli.as_ref()).await {
                discovered.push(provider);
            }
        }
        discovered
    }

    /// Starts a request in the background and sends its result back as an event.
    pub fn dispatch(
        &self,
        request: ProviderRequest,
        events: tokio::sync::mpsc::UnboundedSender<AppEvent>,
    ) {
        match request {
            ProviderRequest::RefreshWorkspace {
                request_id,
                provider_id,
            } => {
                let Some(workspace) = self
                    .workspaces
                    .iter()
                    .find(|(id, _)| id == &provider_id)
                    .map(|(_, workspace)| Arc::clone(workspace))
                else {
                    return;
                };
                let cli = Arc::clone(&self.cli);
                tokio::spawn(async move {
                    let result = workspace.refresh(cli.as_ref()).await;
                    let _ = events.send(AppEvent::RefreshCompleted {
                        request_id,
                        provider_id,
                        result,
                    });
                });
            }
            ProviderRequest::ExecuteResourceCommand {
                request_id,
                provider_id,
                panel_id,
                resource_id,
                resource_name,
                command,
                state,
            } => {
                let Some(workspace) = self
                    .workspaces
                    .iter()
                    .find(|(id, _)| id == &provider_id)
                    .map(|(_, workspace)| Arc::clone(workspace))
                else {
                    return;
                };
                let cli = Arc::clone(&self.cli);
                tokio::spawn(async move {
                    let result = workspace
                        .execute_command(cli.as_ref(), &panel_id, &resource_id, command, state)
                        .await;
                    let _ = events.send(AppEvent::ResourceCommandCompleted {
                        request_id,
                        provider_id,
                        resource_id,
                        resource_name,
                        command,
                        result,
                    });
                });
            }
            ProviderRequest::LoadResourceDetails {
                request_id,
                provider_id,
                panel_id,
                resource_id,
                view_id,
            } => {
                let Some(workspace) = self.workspace(&provider_id) else {
                    return;
                };
                let cli = Arc::clone(&self.cli);
                tokio::spawn(async move {
                    let result = workspace
                        .load_details(cli.as_ref(), &panel_id, &resource_id, &view_id)
                        .await;
                    let _ = events.send(AppEvent::ResourceDetailsCompleted {
                        request_id,
                        provider_id,
                        panel_id,
                        resource_id,
                        view_id,
                        result,
                    });
                });
            }
        }
    }

    fn workspace(&self, provider_id: &ProviderId) -> Option<Arc<dyn ProviderWorkspace>> {
        self.workspaces
            .iter()
            .find(|(id, _)| id == provider_id)
            .map(|(_, workspace)| Arc::clone(workspace))
    }
}
