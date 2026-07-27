//! The terminal handover, at the seam that can show its ordering without a real
//! terminal, container, or instance.
//!
//! Suspending a screen and running a process that owns the user's streams are
//! both things only the host can do, so both are traits here. A fake of each,
//! writing to one shared log, is what makes "suspend, then run, then resume"
//! an assertion rather than a hope.

use std::{
    io,
    sync::{Arc, Mutex},
};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use virtui::{
    app::{App, AppEvent},
    cli::{InteractiveRunner, ProcessError, ProcessFailure, ProcessSpec},
    provider::{
        DetailView, ProviderDiscovery, ProviderId, ProviderRequest, Resource, ResourceCommand,
        ResourceId, ResourcePanel, ResourceState, WorkspaceSnapshot,
    },
    runtime::{ShellTerminal, handle_key, open_pending_shell},
    ui::render_to_text,
};

/// Everything the host was asked to do, in the order it was asked.
#[derive(Clone, Default)]
struct Handover(Arc<Mutex<Vec<String>>>);

impl Handover {
    fn record(&self, step: impl Into<String>) {
        self.0.lock().expect("handover log lock").push(step.into());
    }

    fn steps(&self) -> Vec<String> {
        self.0.lock().expect("handover log lock").clone()
    }
}

struct FakeTerminal(Handover);

impl ShellTerminal for FakeTerminal {
    fn suspend(&mut self) -> io::Result<()> {
        self.0.record("suspend");
        Ok(())
    }

    fn resume(&mut self) -> io::Result<()> {
        self.0.record("resume");
        Ok(())
    }

    fn discard_keys(&mut self) {
        self.0.record("discard keys");
    }

    fn resume_reading(&mut self) {
        self.0.record("resume reading");
    }
}

struct FakeShell {
    handover: Handover,
    outcome: Result<(), ProcessError>,
}

impl InteractiveRunner for FakeShell {
    fn run_interactive(&self, process: &ProcessSpec) -> Result<(), ProcessError> {
        self.handover.record(format!(
            "run {} {}",
            process.program,
            process.args.join(" ")
        ));
        self.outcome.clone()
    }
}

fn docker_discovery() -> ProviderDiscovery {
    ProviderDiscovery {
        id: ProviderId::new("docker"),
        name: "Docker".to_owned(),
        target_environment: "desktop-linux".to_owned(),
        error: None,
    }
}

/// One running container, carrying the Interactive Shell Docker offers inside
/// it.
fn running_container() -> WorkspaceSnapshot {
    WorkspaceSnapshot {
        panels: vec![ResourcePanel {
            title: "Containers".to_owned(),
            detail_views: vec![DetailView::new("logs", "Logs")],
            resources: vec![Resource {
                id: ResourceId::new("container-a"),
                name: "api".to_owned(),
                status: Some("running".to_owned()),
                state: ResourceState::Running,
                fields: vec![("Image".to_owned(), "nginx:1.27".to_owned())],
                available_commands: vec![ResourceCommand::Stop],
                shell: Some(ProcessSpec::new(
                    "docker",
                    &["exec", "-it", "container-a", "/bin/sh"],
                )),
            }],
        }],
    }
}

/// An application sitting on a running container, with `E` already pressed.
fn app_awaiting_the_terminal() -> App {
    let mut app = App::new();
    let requests = app.update(AppEvent::ProviderDiscovered(docker_discovery()));
    let ProviderRequest::RefreshWorkspace {
        request_id,
        provider_id,
    } = requests.into_iter().next().expect("an initial refresh")
    else {
        panic!("discovery refreshes the Active Workspace");
    };
    app.update(AppEvent::RefreshCompleted {
        request_id,
        provider_id,
        result: Ok(running_container()),
    });
    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('E'), KeyModifiers::NONE),
    );
    assert!(
        app.state().pending_shell.is_some(),
        "a running container asks for the terminal"
    );
    app
}

/// The whole point of the handover: Virtui is off the screen before the
/// Provider CLI touches it, and back on afterwards. The refresh can only be
/// returned once all three have happened.
#[test]
fn the_terminal_is_suspended_for_the_provider_cli_and_taken_back_after() {
    let mut app = app_awaiting_the_terminal();
    let handover = Handover::default();
    let mut terminal = FakeTerminal(handover.clone());
    let shell = FakeShell {
        handover: handover.clone(),
        outcome: Ok(()),
    };

    let requests =
        open_pending_shell(&mut app, &mut terminal, &shell).expect("the terminal to come back");

    assert_eq!(
        handover.steps(),
        [
            "suspend",
            "run docker exec -it container-a /bin/sh",
            "resume",
            "discard keys",
            "resume reading",
        ]
    );
    assert!(
        matches!(
            requests.as_slice(),
            [ProviderRequest::RefreshWorkspace { provider_id, .. }]
                if provider_id == &ProviderId::new("docker")
        ),
        "returning from a shell refreshes only the Active Workspace, got {requests:?}"
    );
    assert!(app.state().pending_shell.is_none());
    assert!(app.state().command_error.is_none());
}

/// A Provider CLI that was there at discovery and is gone by the time the user
/// asks for a shell must not take the terminal with it. The complaint waits
/// until Virtui is back on screen, which is the only place the user can read
/// it.
#[test]
fn a_shell_that_never_starts_still_gives_the_terminal_back_and_names_what_failed() {
    let mut app = app_awaiting_the_terminal();
    let handover = Handover::default();
    let mut terminal = FakeTerminal(handover.clone());
    let shell = FakeShell {
        handover: handover.clone(),
        outcome: Err(ProcessError::ExecutableNotFound),
    };

    open_pending_shell(&mut app, &mut terminal, &shell).expect("the terminal to come back");

    assert_eq!(
        handover.steps().last().map(String::as_str),
        Some("resume reading"),
        "the handover finishes even when the shell never started"
    );
    assert_eq!(
        app.state().command_error.as_deref(),
        Some("Docker shell failed for api (container-a): the CLI is no longer available")
    );
    let screen = render_to_text(app.state(), 100, 24);
    assert!(
        screen.contains("Docker shell failed for api"),
        "rendered screen:\n{screen}"
    );
}

/// An interactive process writes to the user's own terminal rather than to
/// captured streams, so nothing it printed survives the handover. The exit
/// status is the whole of what is left to report.
#[test]
fn a_shell_that_exits_unsuccessfully_reports_the_status_it_left() {
    let mut app = app_awaiting_the_terminal();
    let handover = Handover::default();
    let mut terminal = FakeTerminal(handover.clone());
    let shell = FakeShell {
        handover: handover.clone(),
        outcome: Err(ProcessError::Exited(ProcessFailure {
            exit_code: Some(126),
            stdout: String::new(),
            stderr: String::new(),
        })),
    };

    let requests =
        open_pending_shell(&mut app, &mut terminal, &shell).expect("the terminal to come back");

    assert_eq!(
        handover.steps().last().map(String::as_str),
        Some("resume reading")
    );
    assert_eq!(
        app.state().command_error.as_deref(),
        Some("Docker shell failed for api (container-a): exit status 126")
    );
    assert!(
        matches!(
            requests.as_slice(),
            [ProviderRequest::RefreshWorkspace { .. }]
        ),
        "a shell that ran still leaves the workspace worth refreshing, got {requests:?}"
    );
}

/// Keys typed while the shell held the terminal were typed at the shell, and
/// are gone before Virtui reads anything.
///
/// The order is what makes that true rather than merely likely: a reader
/// started first is already pulling those keys out of the queue, so a discard
/// that follows it empties a queue the reader has partly drained and the
/// remainder lands on Virtui as commands the user never aimed at it.
#[test]
fn keys_typed_at_the_shell_are_discarded_before_virtui_reads_again() {
    let mut app = app_awaiting_the_terminal();
    let handover = Handover::default();
    let mut terminal = FakeTerminal(handover.clone());
    let shell = FakeShell {
        handover: handover.clone(),
        outcome: Ok(()),
    };

    open_pending_shell(&mut app, &mut terminal, &shell).expect("the terminal to come back");

    let steps = handover.steps();
    let discarded = steps
        .iter()
        .position(|step| step == "discard keys")
        .expect("the keys typed at the shell to be discarded");
    let reading = steps
        .iter()
        .position(|step| step == "resume reading")
        .expect("Virtui to read keys again");
    assert!(
        discarded < reading,
        "discarding must precede reading, got {steps:?}"
    );
}

/// Nothing to hand over means nothing is disturbed: the host calls this every
/// time round the loop, so an untouched terminal must stay untouched.
#[test]
fn a_loop_with_no_shell_waiting_leaves_the_terminal_alone() {
    let mut app = App::new();
    let handover = Handover::default();
    let mut terminal = FakeTerminal(handover.clone());
    let shell = FakeShell {
        handover: handover.clone(),
        outcome: Ok(()),
    };

    let requests = open_pending_shell(&mut app, &mut terminal, &shell).expect("nothing to do");

    assert!(handover.steps().is_empty());
    assert!(requests.is_empty());
}
