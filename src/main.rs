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
use vertui::{
    app::{App, AppEvent},
    cli::{CliRunner, TokioCliRunner},
    docker::DockerWorkspace,
    provider::{ProviderAction, ProviderWorkspace},
    runtime::{ProviderRuntime, RefreshTimer, ShellControl, handle_key},
    ui,
};

#[tokio::main]
async fn main() -> io::Result<()> {
    let mut terminal = ratatui::init();
    let result = run(&mut terminal).await;
    ratatui::restore();
    result
}

async fn run(terminal: &mut DefaultTerminal) -> io::Result<()> {
    let workspaces = [Arc::new(DockerWorkspace::new()) as Arc<dyn ProviderWorkspace>];
    let cli = Arc::new(TokioCliRunner) as Arc<dyn CliRunner>;
    let runtime = ProviderRuntime::new(workspaces, cli);
    let (completion_tx, mut completion_rx) = mpsc::unbounded_channel();
    let mut app = App::new();

    for discovered in runtime.discover().await {
        let actions = app.update(AppEvent::ProviderDiscovered(discovered));
        dispatch_all(&runtime, &completion_tx, actions);
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
                let (control, actions) = handle_key(&mut app, key);
                dispatch_all(&runtime, &completion_tx, actions);
                if control == ShellControl::Quit {
                    break Ok(());
                }
            }
            Some(event) = completion_rx.recv() => {
                let actions = app.update(event);
                dispatch_all(&runtime, &completion_tx, actions);
            }
            _ = refresh_timer.tick() => {
                let actions = app.update(AppEvent::RefreshTimerElapsed);
                dispatch_all(&runtime, &completion_tx, actions);
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
    actions: Vec<ProviderAction>,
) {
    for action in actions {
        runtime.dispatch(action, completion_tx.clone());
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
