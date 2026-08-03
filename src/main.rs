use std::{
    future, io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use crossterm::{
    cursor::MoveTo,
    event::{self, Event, KeyEvent},
    execute,
    terminal::{Clear, ClearType, EnterAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::DefaultTerminal;
use tokio::sync::mpsc;
use virtui::{
    application::{
        App, AppEvent, Command, CommandRegistry, InteractiveShellOutcome, ProviderRequest,
    },
    infrastructure::{
        config::{Env, FileSystemReader, load},
        process::{CliRunner, InteractiveRunner, ProcessSpec, TokioCliRunner},
        runtime::{ProviderRuntime, RefreshTimer},
    },
    presentation,
};

#[cfg(test)]
mod host_tests;

#[tokio::main]
async fn main() -> io::Result<()> {
    // Configuration is loaded and validated before raw mode so a bad file
    // exits with a readable diagnostic instead of scrambling the terminal.
    let registry = match load(&Env::from_environment(), &FileSystemReader) {
        Ok(registry) => registry,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    };
    let mut terminal = ratatui::init();
    let result = run(&mut terminal, registry).await;
    ratatui::restore();
    result
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ShellControl {
    Continue,
    Quit,
}

const DETAIL_DISPATCH_QUIET_PERIOD: Duration = Duration::from_millis(75);

/// Host-owned dispatch timing for Detail View loads.
///
/// Application request identity still decides which completion is valid; this
/// queue prevents superseded Provider work from starting in the first place.
struct DetailDispatchQueue {
    quiet_period: Duration,
    pending: Option<(Instant, ProviderRequest)>,
}

impl DetailDispatchQueue {
    fn new(quiet_period: Duration) -> Self {
        Self {
            quiet_period,
            pending: None,
        }
    }

    fn accept(&mut self, now: Instant, request: ProviderRequest) -> Option<ProviderRequest> {
        if matches!(request, ProviderRequest::LoadResourceDetails { .. }) {
            self.pending = Some((now + self.quiet_period, request));
            None
        } else {
            Some(request)
        }
    }

    fn deadline(&self) -> Option<Instant> {
        self.pending.as_ref().map(|(deadline, _)| *deadline)
    }

    fn take_ready(&mut self, now: Instant) -> Option<ProviderRequest> {
        let (deadline, _) = self.pending.as_ref()?;
        if now < *deadline {
            return None;
        }
        self.pending.take().map(|(_, request)| request)
    }
}

/// Maps terminal input at the host boundary, then asks the application to
/// resolve and invoke the resulting logical command.
fn handle_key(app: &mut App, event: KeyEvent) -> (ShellControl, Vec<ProviderRequest>) {
    let Some(key) = presentation::key_from_event(event) else {
        return (ShellControl::Continue, Vec::new());
    };
    if app.reserved(key) == Some(Command::Quit) {
        return (ShellControl::Quit, Vec::new());
    }
    match app.resolve_command(key) {
        Some(Command::Quit) => (ShellControl::Quit, Vec::new()),
        Some(command) => (ShellControl::Continue, app.invoke(command)),
        None => (ShellControl::Continue, Vec::new()),
    }
}

/// The terminal and input-reader operations ordered by an interactive-shell
/// handover. This is a host seam: neither application nor infrastructure owns
/// the user's terminal lifecycle.
trait ShellTerminal {
    fn suspend(&mut self) -> io::Result<()>;
    fn resume(&mut self) -> io::Result<()>;
    fn discard_keys(&mut self);
    fn resume_reading(&mut self);
}

fn open_pending_shell(
    app: &mut App,
    terminal: &mut dyn ShellTerminal,
    runner: &dyn InteractiveRunner,
) -> io::Result<Vec<ProviderRequest>> {
    let Some(shell) = app.take_pending_shell() else {
        return Ok(Vec::new());
    };
    terminal.suspend()?;
    let result = runner.run_interactive(&ProcessSpec::from(&shell.process));
    let resumed = take_the_terminal_back(terminal);
    let outcome = result.err().and_then(|error| error.start_failure()).map_or(
        InteractiveShellOutcome::Exited,
        InteractiveShellOutcome::StartFailed,
    );
    let requests = app.update(AppEvent::ShellClosed { shell, outcome });
    resumed?;
    Ok(requests)
}

fn take_the_terminal_back(terminal: &mut dyn ShellTerminal) -> io::Result<()> {
    terminal.resume()?;
    terminal.discard_keys();
    terminal.resume_reading();
    Ok(())
}

async fn run(terminal: &mut DefaultTerminal, registry: CommandRegistry) -> io::Result<()> {
    let cli = Arc::new(TokioCliRunner) as Arc<dyn CliRunner>;
    let runtime = ProviderRuntime::with_builtin_providers(cli);
    let (completion_tx, mut completion_rx) = mpsc::unbounded_channel();
    let mut app = App::with_registry(registry);
    let mut detail_dispatch = DetailDispatchQueue::new(DETAIL_DISPATCH_QUIET_PERIOD);

    for discovered in runtime.discover().await {
        let requests = app.update(discovered.into_event());
        dispatch_all(&runtime, &completion_tx, &mut detail_dispatch, requests);
    }

    let (key_tx, mut key_rx) = mpsc::unbounded_channel();
    let mut input = InputThread::start(key_tx);
    let mut refresh_timer = RefreshTimer::new();

    let result = loop {
        if let Err(error) = terminal.draw(|frame| presentation::render(app.state(), frame)) {
            break Err(error);
        }

        let detail_deadline = detail_dispatch.deadline();
        tokio::select! {
            biased;
            Some(key) = key_rx.recv() => {
                let (control, requests) = handle_key(&mut app, key);
                dispatch_all(&runtime, &completion_tx, &mut detail_dispatch, requests);
                if control == ShellControl::Quit {
                    break Ok(());
                }
                // A key may have asked for the terminal. Handing it over blocks
                // this loop until the shell exits, which is the point: Virtui
                // has no screen to draw on until it comes back.
                //
                // Asked only when a shell is actually waiting: `block_in_place`
                // hands this worker's remaining tasks to another thread, which
                // is worth doing for a shell and worth nothing for the `j` that
                // moved the selection.
                if app.state().pending_shell.is_some() {
                    let mut host = Host {
                        terminal: &mut *terminal,
                        input: &mut input,
                        keys: &mut key_rx,
                    };
                    // Moving those tasks aside is what keeps provider work
                    // already in flight running while the shell holds the
                    // terminal. It also makes the multi-threaded runtime a
                    // stated requirement rather than a silent one: it panics on
                    // a current-thread runtime.
                    let handover = tokio::task::block_in_place(|| {
                        open_pending_shell(&mut app, &mut host, &TokioCliRunner)
                    });
                    match handover {
                        Ok(requests) => dispatch_all(
                            &runtime,
                            &completion_tx,
                            &mut detail_dispatch,
                            requests,
                        ),
                        // The screen never came back, so the modal that would
                        // have carried the shell's own failure will never be
                        // drawn. This exit line is the last place left to say
                        // it, and it is printed once the terminal is restored.
                        Err(error) => break Err(match app.state().command_error.as_deref() {
                            Some(shell) => {
                                io::Error::new(error.kind(), format!("{error}; {shell}"))
                            }
                            None => error,
                        }),
                    }
                }
            }
            Some(event) = completion_rx.recv() => {
                let requests = app.update(event);
                dispatch_all(&runtime, &completion_tx, &mut detail_dispatch, requests);
            }
            _ = refresh_timer.tick() => {
                let requests = app.update(AppEvent::RefreshTimerElapsed);
                dispatch_all(&runtime, &completion_tx, &mut detail_dispatch, requests);
            }
            _ = wait_for_detail_dispatch(detail_deadline) => {
                if let Some(request) = detail_dispatch.take_ready(Instant::now()) {
                    if app.detail_request_is_current(&request) {
                        runtime.dispatch(request, completion_tx.clone());
                    }
                }
            }
        }
    };

    input.stop();
    result
}

/// The real terminal and the thread competing with a shell for its keystrokes.
struct Host<'a> {
    terminal: &'a mut DefaultTerminal,
    input: &'a mut InputThread,
    /// The keys the reader has already published, which the discard step empties
    /// before anything is allowed to read again.
    keys: &'a mut mpsc::UnboundedReceiver<KeyEvent>,
}

impl ShellTerminal for Host<'_> {
    fn suspend(&mut self) -> io::Result<()> {
        // Reading stops before the screen is given up: a thread still polling
        // crossterm would swallow the keystrokes meant for the shell.
        self.input.stop();
        // Raw mode goes; the alternate screen stays. Leaving it would uncover
        // the terminal Virtui was launched from, and the shell would open on
        // top of whatever was already there — the user's own scrollback, with a
        // container's prompt in the middle of it. Wiping the alternate screen
        // instead opens the shell on nothing but itself, and leaves the real
        // terminal untouched for Virtui to hand back whole at the end.
        disable_raw_mode()?;
        execute!(io::stdout(), Clear(ClearType::All), MoveTo(0, 0))
    }

    fn resume(&mut self) -> io::Result<()> {
        // Re-entering raw mode and the alternate screen directly rather than
        // building a second terminal with `try_init`: that installs a panic hook
        // wrapping the previous one, so a session with several shells in it
        // would nest a fresh hook per shell.
        enable_raw_mode()?;
        // Asking for the alternate screen Virtui never gave up costs nothing,
        // and is what recovers the one case where it did lose it: a full-screen
        // program run inside the shell — an editor in the container — leaves the
        // alternate screen on its way out and drops the terminal back onto the
        // screen Virtui must not draw over.
        execute!(io::stdout(), EnterAlternateScreen)?;
        // The shell wrote all over the screen Virtui last drew, so nothing that
        // survives the handover is worth keeping — and without this the next
        // draw would diff against a buffer describing a screen that is gone.
        //
        // Clearing asks the terminal where its cursor is and waits for the
        // answer, which is a second reason this step must finish before
        // `resume_reading`: an input thread would take that answer for a
        // keystroke and leave the clear waiting for a reply already eaten.
        self.terminal.clear()?;
        Ok(())
    }

    fn discard_keys(&mut self) {
        // Draining what the reader published rather than what the terminal
        // still holds: the reader is stopped, so anything it had time to send
        // before noticing is already here, and nothing new can arrive until
        // `resume_reading`.
        while self.keys.try_recv().is_ok() {}
    }

    fn resume_reading(&mut self) {
        self.input.start_again();
    }
}

/// The blocking terminal reader, publishing keys as application input.
///
/// It is stoppable because an Interactive Shell needs the keystrokes more than
/// Virtui does, and restartable because Virtui needs them back afterwards.
struct InputThread {
    keys: mpsc::UnboundedSender<KeyEvent>,
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl InputThread {
    fn start(keys: mpsc::UnboundedSender<KeyEvent>) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let handle = spawn_input_thread(keys.clone(), Arc::clone(&stop));
        Self {
            keys,
            stop,
            handle: Some(handle),
        }
    }

    /// Stops reading and waits for the thread to notice, so no reader is left
    /// holding the terminal when the next process takes it.
    fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }

    fn start_again(&mut self) {
        self.stop.store(false, Ordering::Relaxed);
        self.handle = Some(spawn_input_thread(
            self.keys.clone(),
            Arc::clone(&self.stop),
        ));
    }
}

fn dispatch_all(
    runtime: &ProviderRuntime,
    completion_tx: &mpsc::UnboundedSender<AppEvent>,
    detail_dispatch: &mut DetailDispatchQueue,
    requests: Vec<ProviderRequest>,
) {
    let now = Instant::now();
    for request in requests {
        if let Some(request) = detail_dispatch.accept(now, request) {
            runtime.dispatch(request, completion_tx.clone());
        }
    }
}

async fn wait_for_detail_dispatch(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline.into()).await,
        None => future::pending().await,
    }
}

fn spawn_input_thread(
    keys: mpsc::UnboundedSender<KeyEvent>,
    stop: Arc<AtomicBool>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        while !stop.load(Ordering::Relaxed) {
            match event::poll(Duration::from_millis(100)) {
                Ok(true) => match event::read() {
                    Ok(Event::Key(key)) => {
                        if keys.send(key).is_err() {
                            break;
                        }
                    }
                    Ok(_) => {}
                    Err(_) => break,
                },
                Ok(false) => {}
                Err(_) => break,
            }
        }
    })
}
