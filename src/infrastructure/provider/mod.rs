use std::{future::Future, pin::Pin};

mod docker;
mod docker_sandbox;
mod incus;

pub use docker::DockerWorkspace;
pub use docker_sandbox::DockerSandboxWorkspace;
pub use incus::IncusWorkspace;

use crate::application::{
    DetailView, Resource, ResourceCommand, ResourceDetails, ResourcePanel, WorkspaceError,
    WorkspaceSnapshot,
};
use crate::domain::{
    DetailViewId, Provider, ProviderId, ProviderVersion, ResourceId, ResourcePanelId,
    ResourceState, ResourceTarget, TargetEnvironment,
};
use crate::infrastructure::process::{CliRunner, ProcessError};

/// What a Provider CLI left behind, with no suggested next step attached.
///
/// `fallback` carries the caller's meaning for a process that failed without
/// writing anything at all.
pub fn provider_cli_error(provider_name: &str, error: &ProcessError, fallback: &str) -> String {
    match error {
        ProcessError::ExecutableNotFound => {
            format!("{provider_name} CLI is no longer available")
        }
        ProcessError::SpawnFailed(reason) => {
            format!("{provider_name} CLI could not be started: {reason}")
        }
        ProcessError::Exited(failure) => failure.message_or(fallback),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// The presence and initial state of an installed provider.
///
/// `error` distinguishes an installed-but-unreachable provider from an absent
/// CLI, which is represented by `None` from [`ProviderWorkspace::discover`].
pub struct ProviderDiscovery {
    provider: Provider,
    error: Option<WorkspaceError>,
}

impl ProviderDiscovery {
    pub fn new(provider: Provider, error: Option<WorkspaceError>) -> Self {
        Self { provider, error }
    }

    pub fn provider(&self) -> &Provider {
        &self.provider
    }

    pub fn error(&self) -> Option<&WorkspaceError> {
        self.error.as_ref()
    }

    pub fn into_parts(self) -> (Provider, Option<WorkspaceError>) {
        (self.provider, self.error)
    }

    pub fn into_event(self) -> crate::application::AppEvent {
        let (provider, error) = self.into_parts();
        crate::application::AppEvent::ProviderDiscovered { provider, error }
    }
}

/// A provider-specific source of presentation-neutral workspace data.
///
/// Implementations own CLI commands and parsing. They never render Ratatui
/// widgets, keeping provider knowledge out of the shared shell.
pub trait ProviderWorkspace: Send + Sync {
    fn id(&self) -> ProviderId;

    /// Returns `None` when the provider CLI is absent; otherwise returns an
    /// available provider or one with an actionable connection error.
    fn discover<'a>(
        &'a self,
        cli: &'a dyn CliRunner,
    ) -> Pin<Box<dyn Future<Output = Option<ProviderDiscovery>> + Send + 'a>>;

    /// Refreshes the provider's native resource panels through the CLI seam.
    fn refresh<'a>(
        &'a self,
        cli: &'a dyn CliRunner,
    ) -> Pin<Box<dyn Future<Output = Result<WorkspaceSnapshot, WorkspaceError>> + Send + 'a>>;

    /// Runs one lifecycle Command against a Resource.
    ///
    /// `state` is what the last refresh reported for that Resource, so a
    /// Command that must behave differently for a running Resource can do so
    /// without a second Provider CLI query.
    fn execute_command<'a>(
        &'a self,
        cli: &'a dyn CliRunner,
        target: &'a ResourceTarget,
        command: ResourceCommand,
        state: ResourceState,
    ) -> Pin<Box<dyn Future<Output = Result<(), WorkspaceError>> + Send + 'a>>;

    /// Loads one of the detail views this workspace declared for a Resource.
    ///
    /// Only the view the user is looking at is ever asked for, so this runs
    /// exactly one Provider CLI request per visible view.
    fn load_details<'a>(
        &'a self,
        cli: &'a dyn CliRunner,
        target: &'a ResourceTarget,
        view_id: &'a DetailViewId,
    ) -> Pin<Box<dyn Future<Output = Result<ResourceDetails, WorkspaceError>> + Send + 'a>>;
}
