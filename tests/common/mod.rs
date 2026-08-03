//! The CLI double every Provider Workspace test drives its provider through.
//!
//! Each integration test is its own crate, so items unused by one of them are
//! not dead — they are simply used by another.
#![allow(dead_code)]

use std::{
    collections::VecDeque,
    future::Future,
    pin::Pin,
    sync::{Mutex, MutexGuard},
};

use virtui::infrastructure::process::{
    CliRunner, ProcessError, ProcessFailure, ProcessOutput, ProcessSpec,
};
use virtui::{
    application::{
        App, AppEvent, ProviderRequest, ResourceDetails, WorkspaceError, WorkspaceSnapshot,
    },
    infrastructure::provider::ProviderDiscovery,
};

/// A [`CliRunner`] that answers a fixed script of commands in order.
///
/// It asserts each command matches what the test said would come next, and
/// panics on any command the script did not expect. That is what lets a test
/// prove a Provider ran *only* the work it should have.
pub struct FixtureCli {
    responses: Mutex<VecDeque<(ProcessSpec, Result<ProcessOutput, ProcessError>)>>,
}

impl FixtureCli {
    pub fn new(
        responses: impl IntoIterator<Item = (ProcessSpec, Result<ProcessOutput, ProcessError>)>,
    ) -> Self {
        Self {
            responses: Mutex::new(responses.into_iter().collect()),
        }
    }

    fn queue(
        &self,
    ) -> MutexGuard<'_, VecDeque<(ProcessSpec, Result<ProcessOutput, ProcessError>)>> {
        self.responses.lock().expect("fixture queue lock")
    }
}

impl CliRunner for FixtureCli {
    fn run<'a>(
        &'a self,
        command: ProcessSpec,
    ) -> Pin<Box<dyn Future<Output = Result<ProcessOutput, ProcessError>> + Send + 'a>> {
        Box::pin(async move {
            let (expected, response) = self.queue().pop_front().expect("unexpected CLI command");
            assert_eq!(command, expected);
            response
        })
    }
}

/// A process that exited 0, writing `stdout` and nothing to stderr.
pub fn success(stdout: &str) -> Result<ProcessOutput, ProcessError> {
    Ok(ProcessOutput {
        stdout: stdout.to_owned(),
        stderr: String::new(),
    })
}

/// A process that failed, explaining itself on stderr as a CLI usually does.
pub fn failure(stderr: &str) -> Result<ProcessOutput, ProcessError> {
    Err(ProcessError::Exited(ProcessFailure {
        exit_code: Some(1),
        stdout: String::new(),
        stderr: stderr.to_owned(),
    }))
}

/// A process that failed but wrote its complaint to stdout instead.
pub fn failure_on_stdout(stdout: &str) -> Result<ProcessOutput, ProcessError> {
    Err(ProcessError::Exited(ProcessFailure {
        exit_code: Some(1),
        stdout: stdout.to_owned(),
        stderr: String::new(),
    }))
}

/// A process that failed without writing a word, leaving the caller's own
/// prose as the only thing to show the user.
pub fn silent_failure(exit_code: i32) -> Result<ProcessOutput, ProcessError> {
    Err(ProcessError::Exited(ProcessFailure {
        exit_code: Some(exit_code),
        stdout: String::new(),
        stderr: String::new(),
    }))
}

/// Constructs the completion matching a refresh request emitted by the App.
pub fn refresh_completed(
    request: ProviderRequest,
    result: Result<WorkspaceSnapshot, WorkspaceError>,
) -> AppEvent {
    match request {
        ProviderRequest::RefreshWorkspace {
            request_id,
            provider_id,
        } => AppEvent::RefreshCompleted {
            request_id,
            provider_id,
            result,
        },
        other => panic!("expected refresh request, got {other:?}"),
    }
}

/// Selects the refresh from requests emitted by one App Event or Command.
pub fn refresh_request(requests: Vec<ProviderRequest>) -> ProviderRequest {
    requests
        .into_iter()
        .find(|request| matches!(request, ProviderRequest::RefreshWorkspace { .. }))
        .expect("refresh request")
}

/// Discovers a Provider and applies its initial snapshot, returning any detail
/// loads the ready Provider Workspace emits.
pub fn ready_workspace(
    app: &mut App,
    discovery: ProviderDiscovery,
    snapshot: WorkspaceSnapshot,
) -> Vec<ProviderRequest> {
    let refresh = refresh_request(app.update(discovery.into_event()));
    app.update(refresh_completed(refresh, Ok(snapshot)))
}

/// Selects the Resource Command from requests emitted by one App Command.
pub fn command_request(requests: Vec<ProviderRequest>) -> ProviderRequest {
    requests
        .into_iter()
        .find(|request| matches!(request, ProviderRequest::ExecuteResourceCommand { .. }))
        .expect("Resource Command request")
}

/// Constructs the completion matching a Resource Command request.
pub fn command_completed(request: ProviderRequest, result: Result<(), WorkspaceError>) -> AppEvent {
    match request {
        ProviderRequest::ExecuteResourceCommand {
            request_id,
            provider_id,
            target,
            command,
            ..
        } => AppEvent::ResourceCommandCompleted {
            request_id,
            provider_id,
            target,
            command,
            result,
        },
        other => panic!("expected Resource Command request, got {other:?}"),
    }
}

/// Selects the detail load from requests emitted by one App Event or Command.
pub fn detail_request(requests: Vec<ProviderRequest>) -> ProviderRequest {
    requests
        .into_iter()
        .find(|request| matches!(request, ProviderRequest::LoadResourceDetails { .. }))
        .expect("detail load request")
}

/// Constructs the completion matching a detail-load request.
pub fn details_completed(
    request: ProviderRequest,
    result: Result<ResourceDetails, WorkspaceError>,
) -> AppEvent {
    match request {
        ProviderRequest::LoadResourceDetails {
            request_id,
            provider_id,
            target,
            view_id,
        } => AppEvent::ResourceDetailsCompleted {
            request_id,
            provider_id,
            target,
            view_id,
            result,
        },
        other => panic!("expected detail load request, got {other:?}"),
    }
}
