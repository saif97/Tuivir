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
    runtime::{ProviderRuntime, RefreshTimer, ShellControl, handle_key},
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
    let stop_input = Arc::new(AtomicBool::new(false));
    let input_thread = spawn_input_thread(key_tx, Arc::clone(&stop_input));
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

    stop_input.store(true, Ordering::Relaxed);
    let _ = input_thread.join();
    result
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
