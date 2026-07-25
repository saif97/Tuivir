use std::{sync::Arc, time::Duration};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind};
use tokio::time::{Instant, Interval, MissedTickBehavior};

use crate::{
    app::{App, AppEvent, FocusedPanel},
    cli::CliRunner,
    docker::DockerWorkspace,
    incus::IncusWorkspace,
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
    if key.kind != KeyEventKind::Press {
        return (ShellControl::Continue, Vec::new());
    }
    if app.state().confirmation.is_some() {
        return match key.code {
            KeyCode::Char('y') | KeyCode::Enter => (
                ShellControl::Continue,
                app.update(AppEvent::ConfirmResourceCommand),
            ),
            KeyCode::Char('n') | KeyCode::Esc => (
                ShellControl::Continue,
                app.update(AppEvent::CancelConfirmation),
            ),
            _ => (ShellControl::Continue, Vec::new()),
        };
    }
    if app.state().command_error.is_some() {
        return match key.code {
            KeyCode::Esc | KeyCode::Enter => (
                ShellControl::Continue,
                app.update(AppEvent::DismissCommandError),
            ),
            _ => (ShellControl::Continue, Vec::new()),
        };
    }
    if app.state().help_overlay.is_some() {
        return match key.code {
            KeyCode::Char('?') | KeyCode::Char('q') | KeyCode::Esc => {
                (ShellControl::Continue, app.update(AppEvent::ToggleHelp))
            }
            _ => (ShellControl::Continue, Vec::new()),
        };
    }
    if key.code == KeyCode::Char('?') {
        return (ShellControl::Continue, app.update(AppEvent::ToggleHelp));
    }
    if let Some(command) = app.resource_command_for_key(&key) {
        return (
            ShellControl::Continue,
            app.update(AppEvent::ResourceCommandInvoked(command)),
        );
    }
    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => (ShellControl::Quit, Vec::new()),
        KeyCode::Char('1') => (ShellControl::Continue, app.update(AppEvent::FocusProviders)),
        KeyCode::Char('2') => (ShellControl::Continue, app.update(AppEvent::FocusResources)),
        KeyCode::Char('j') | KeyCode::Down => {
            let event = match app.state().focused_panel {
                FocusedPanel::Providers => AppEvent::SelectNextProvider,
                FocusedPanel::Resources => AppEvent::SelectNextResource,
            };
            (ShellControl::Continue, app.update(event))
        }
        KeyCode::Char('k') | KeyCode::Up => {
            let event = match app.state().focused_panel {
                FocusedPanel::Providers => AppEvent::SelectPreviousProvider,
                FocusedPanel::Resources => AppEvent::SelectPreviousResource,
            };
            (ShellControl::Continue, app.update(event))
        }
        KeyCode::Char('r')
            if key
                .modifiers
                .contains(crossterm::event::KeyModifiers::CONTROL) =>
        {
            (ShellControl::Continue, app.update(AppEvent::ManualRefresh))
        }
        KeyCode::Char(']') => (
            ShellControl::Continue,
            app.update(AppEvent::SelectNextProvider),
        ),
        KeyCode::Char('[') => (
            ShellControl::Continue,
            app.update(AppEvent::SelectPreviousProvider),
        ),
        _ => (ShellControl::Continue, Vec::new()),
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
                resource_id,
                resource_name,
                command,
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
                        .execute_command(cli.as_ref(), &resource_id, command)
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
        }
    }
}
