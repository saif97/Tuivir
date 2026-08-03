use std::{sync::Arc, time::Duration};
use tokio::time::{Instant, Interval, MissedTickBehavior};

use crate::{
    application::{AppEvent, ProviderRequest},
    domain::ProviderId,
    infrastructure::process::CliRunner,
    infrastructure::provider::{
        DockerSandboxWorkspace, DockerWorkspace, IncusWorkspace, ProviderDiscovery,
        ProviderWorkspace,
    },
};

/// The cadence for refreshing the Active Workspace.
pub const REFRESH_INTERVAL: Duration = Duration::from_secs(2);

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
        let mut discoveries = tokio::task::JoinSet::new();
        for (index, (_, workspace)) in self.workspaces.iter().enumerate() {
            let workspace = Arc::clone(workspace);
            let cli = Arc::clone(&self.cli);
            discoveries.spawn(async move { (index, workspace.discover(cli.as_ref()).await) });
        }

        let mut discovered = Vec::with_capacity(self.workspaces.len());
        while let Some(result) = discoveries.join_next().await {
            let (index, discovery) = result.expect("Provider discovery task panicked");
            if let Some(discovery) = discovery {
                discovered.push((index, discovery));
            }
        }
        discovered.sort_unstable_by_key(|(index, _)| *index);
        discovered
            .into_iter()
            .map(|(_, discovery)| discovery)
            .collect()
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
                target,
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
                        .execute_command(cli.as_ref(), &target, command, state)
                        .await;
                    let _ = events.send(AppEvent::ResourceCommandCompleted {
                        request_id,
                        provider_id,
                        target,
                        command,
                        result,
                    });
                });
            }
            ProviderRequest::LoadResourceDetails {
                request_id,
                provider_id,
                target,
                view_id,
            } => {
                let Some(workspace) = self.workspace(&provider_id) else {
                    return;
                };
                let cli = Arc::clone(&self.cli);
                tokio::spawn(async move {
                    let result = workspace
                        .load_details(cli.as_ref(), &target, &view_id)
                        .await;
                    let _ = events.send(AppEvent::ResourceDetailsCompleted {
                        request_id,
                        provider_id,
                        target,
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
