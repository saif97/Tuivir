use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

use crossterm::event::{self, Event, KeyEvent};
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
                let mut host = Host { terminal: &mut *terminal, input: &mut input };
                match open_pending_shell(&mut app, &mut host, &TokioCliRunner) {
                    Ok(requests) => dispatch_all(&runtime, &completion_tx, requests),
                    Err(error) => break Err(error),
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
}

impl ShellTerminal for Host<'_> {
    fn suspend(&mut self) -> io::Result<()> {
        // Reading stops before the screen is given up: a thread still polling
        // crossterm would swallow the keystrokes meant for the shell.
        self.input.stop();
        ratatui::try_restore()
    }

    fn resume(&mut self) -> io::Result<()> {
        *self.terminal = ratatui::try_init()?;
        // The shell wrote all over the screen Virtui last drew, so nothing that
        // survives the handover is worth keeping.
        self.terminal.clear()?;
        self.input.start_again();
        Ok(())
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
