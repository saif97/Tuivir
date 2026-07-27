use std::{io, sync::Arc, time::Duration};

use crossterm::event::KeyEvent;
use tokio::time::{Instant, Interval, MissedTickBehavior};

use crate::{
    app::{App, AppEvent},
    cli::{CliRunner, InteractiveRunner},
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

/// The terminal Virtui gives up while an Interactive Shell owns it.
///
/// Leaving the Ratatui screen and stopping the competition for keystrokes are
/// both the host's business, not the application's, so they live behind this
/// seam rather than inside [`App`].
///
/// Taking it back is three steps rather than one because their order is the
/// whole point: keys queued while the shell held the terminal have to be gone
/// before anything reads again, and a reader started first would race the
/// discard for them. Stating the order here, in the function every test drives,
/// is what stops a host from getting it subtly wrong on its own.
pub trait ShellTerminal {
    /// Gives the terminal back to whatever runs next, and stops reading input.
    fn suspend(&mut self) -> io::Result<()>;

    /// Takes the screen back, still not reading.
    fn resume(&mut self) -> io::Result<()>;

    /// Drops whatever the user typed while the shell held the terminal.
    ///
    /// Those keys were typed at the shell, not at Virtui, so acting on them
    /// would be acting on an instruction meant for somebody else.
    fn discard_keys(&mut self);

    /// Starts reading keys into Virtui again.
    fn resume_reading(&mut self);
}

/// Hands the terminal to the Interactive Shell the application asked for, and
/// takes it back.
///
/// Resuming does not depend on how the shell ended: a Provider CLI that never
/// started, or one that exited badly, must not be able to leave the user
/// without their terminal.
///
/// Nor does reporting depend on resuming. The application is told how the shell
/// ended even when the screen refused to come back, so a host on its way out
/// carries that outcome with it instead of losing it alongside the screen that
/// would have shown it; the screen's own failure is passed on afterwards, once
/// there is nothing left to lose by returning early.
///
/// The returned requests are ordinary background work — a refresh of the
/// Active Workspace, and nothing else.
pub fn open_pending_shell(
    app: &mut App,
    terminal: &mut dyn ShellTerminal,
    runner: &dyn InteractiveRunner,
) -> io::Result<Vec<ProviderRequest>> {
    let Some(shell) = app.take_pending_shell() else {
        return Ok(Vec::new());
    };
    terminal.suspend()?;
    let result = runner.run_interactive(&shell.process);
    let resumed = take_the_terminal_back(terminal);
    // The application is told how the shell ended whether or not the screen came
    // back. A host whose terminal is beyond saving is on its way out, and what
    // happened inside the shell is the one fact that would otherwise leave with
    // it unrecorded.
    let requests = app.update(AppEvent::ShellClosed { shell, result });
    resumed?;
    Ok(requests)
}

/// Takes the screen back and lets Virtui read keys into it again.
///
/// Discarding before reading, not after: a reader started first is already
/// competing for the keys the discard is meant to remove. A screen that never
/// came back has nothing to read into, so neither step is attempted.
fn take_the_terminal_back(terminal: &mut dyn ShellTerminal) -> io::Result<()> {
    terminal.resume()?;
    terminal.discard_keys();
    terminal.resume_reading();
    Ok(())
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
                        .execute_command(cli.as_ref(), &resource_id, command, state)
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
                resource_id,
                view_id,
            } => {
                let Some(workspace) = self.workspace(&provider_id) else {
                    return;
                };
                let cli = Arc::clone(&self.cli);
                tokio::spawn(async move {
                    let result = workspace
                        .load_details(cli.as_ref(), &resource_id, &view_id)
                        .await;
                    let _ = events.send(AppEvent::ResourceDetailsCompleted {
                        request_id,
                        provider_id,
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
