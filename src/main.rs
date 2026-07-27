use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
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
    app::{App, AppEvent},
    cli::{CliRunner, TokioCliRunner},
    command::CommandRegistry,
    config::{Env, FileSystemReader, load},
    provider::ProviderRequest,
    runtime::{
        ProviderRuntime, RefreshTimer, ShellControl, ShellTerminal, handle_key, open_pending_shell,
    },
    ui,
};

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

async fn run(terminal: &mut DefaultTerminal, registry: CommandRegistry) -> io::Result<()> {
    let cli = Arc::new(TokioCliRunner) as Arc<dyn CliRunner>;
    let runtime = ProviderRuntime::with_builtin_providers(cli);
    let (completion_tx, mut completion_rx) = mpsc::unbounded_channel();
    let mut app = App::with_registry(registry);

    for discovered in runtime.discover().await {
        let requests = app.update(AppEvent::ProviderDiscovered(discovered));
        dispatch_all(&runtime, &completion_tx, requests);
    }

    let (key_tx, mut key_rx) = mpsc::unbounded_channel();
    let mut input = InputThread::start(key_tx);
    let mut refresh_timer = RefreshTimer::new();

    let result = loop {
        if let Err(error) = terminal.draw(|frame| ui::render(app.state(), frame)) {
            break Err(error);
        }

        tokio::select! {
            Some(key) = key_rx.recv() => {
                let (control, requests) = handle_key(&mut app, key);
                dispatch_all(&runtime, &completion_tx, requests);
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
                        Ok(requests) => dispatch_all(&runtime, &completion_tx, requests),
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
                dispatch_all(&runtime, &completion_tx, requests);
            }
            _ = refresh_timer.tick() => {
                let requests = app.update(AppEvent::RefreshTimerElapsed);
                dispatch_all(&runtime, &completion_tx, requests);
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
    requests: Vec<ProviderRequest>,
) {
    for request in requests {
        runtime.dispatch(request, completion_tx.clone());
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
