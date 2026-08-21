use std::{
    future, io,
    io::Write,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyModifiers,
    },
    execute,
};
use ratatui::DefaultTerminal;
use tokio::sync::mpsc;
use tuivir::{
    application::{
        App, AppEvent, Command, CommandRegistry, ProviderRequest, ResourceShellEffect,
        ResourceShellSessionLifecycle,
    },
    infrastructure::{
        config::{Env, FileSystemReader, load},
        pane_boundary_state::{Env as StateEnv, StateStorage, save as save_pane_boundary},
        process::{CliRunner, TokioCliRunner},
        resource_shell::{ResourceShellRuntime, ResourceShellRuntimeEvent},
        runtime::{ProviderRuntime, RefreshTimer},
    },
    presentation,
};

#[cfg(test)]
mod host_tests;

#[tokio::main]
async fn main() -> io::Result<()> {
    if std::env::args().nth(1).as_deref() == Some("--version") {
        println!("{}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    // Configuration is loaded and validated before raw mode so a bad file
    // exits with a readable diagnostic instead of scrambling the terminal.
    let registry = match load(&Env::from_environment(), &FileSystemReader) {
        Ok(registry) => registry,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    };
    let state_env = StateEnv::from_environment();
    let pane_boundary =
        tuivir::infrastructure::pane_boundary_state::load(&state_env, &FileSystemReader);
    let mut terminal = ratatui::init();
    if let Err(error) = execute!(io::stdout(), EnableMouseCapture) {
        ratatui::restore();
        return Err(error);
    }
    // `ratatui::init` gives back raw mode and the alternate screen on a panic,
    // but it never enabled mouse capture and so cannot know to turn it off.
    // Without this a panic leaves the user's terminal spitting escape sequences
    // at every movement of the mouse.
    let restore_screen = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic| {
        let _ = execute!(io::stdout(), DisableMouseCapture);
        restore_screen(panic);
    }));
    let result = run(&mut terminal, registry, pane_boundary, state_env).await;
    let _ = execute!(io::stdout(), DisableMouseCapture);
    ratatui::restore();
    result
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ShellControl {
    Continue,
    Quit,
}

const DETAIL_DISPATCH_QUIET_PERIOD: Duration = Duration::from_millis(75);

/// Host-owned dispatch timing for Detail View Tab loads.
///
/// Application request identity still decides which completion is valid; this
/// queue prevents superseded Provider work from starting in the first place.
struct DetailDispatchQueue {
    quiet_period: Duration,
    pending: Option<(Instant, ProviderRequest)>,
}

/// Host boundary for copying application-owned Details text.
trait Clipboard {
    fn copy(&mut self, text: &str) -> io::Result<()>;
}

/// OSC 52 lets capable terminals (including Ghostty) own the platform
/// clipboard while keeping Tuivir independent of a desktop clipboard API.
struct Osc52Clipboard<W>(W);

impl<W: Write> Clipboard for Osc52Clipboard<W> {
    fn copy(&mut self, text: &str) -> io::Result<()> {
        const TABLE: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let bytes = text.as_bytes();
        let mut encoded = String::with_capacity(bytes.len().div_ceil(3) * 4);
        for chunk in bytes.chunks(3) {
            let bits = (u32::from(chunk[0]) << 16)
                | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
                | u32::from(*chunk.get(2).unwrap_or(&0));
            encoded.push(TABLE[(bits >> 18) as usize & 63] as char);
            encoded.push(TABLE[(bits >> 12) as usize & 63] as char);
            encoded.push(if chunk.len() > 1 {
                TABLE[(bits >> 6) as usize & 63] as char
            } else {
                '='
            });
            encoded.push(if chunk.len() > 2 {
                TABLE[bits as usize & 63] as char
            } else {
                '='
            });
        }
        write!(self.0, "\x1b]52;c;{encoded}\x07")?;
        self.0.flush()
    }
}

fn copy_pending_details(app: &mut App, clipboard: &mut dyn Clipboard) {
    let Some(text) = app.take_pending_details_copy() else {
        return;
    };
    if let Err(error) = clipboard.copy(&text) {
        app.report_details_copy_failure(error.to_string());
    }
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
#[cfg(test)]
fn handle_key(app: &mut App, event: KeyEvent) -> (ShellControl, Vec<ProviderRequest>) {
    handle_command(app, resolve_key_command(app, event))
}

fn resolve_key_command(app: &App, event: KeyEvent) -> Option<Command> {
    let key = presentation::key_from_event(event)?;
    if app.reserved(key) == Some(Command::Quit) {
        Some(Command::Quit)
    } else {
        app.resolve_command(key)
    }
}

fn handle_command(app: &mut App, command: Option<Command>) -> (ShellControl, Vec<ProviderRequest>) {
    match command {
        Some(Command::Quit) => (ShellControl::Quit, Vec::new()),
        Some(command) => (ShellControl::Continue, app.invoke(command)),
        None => (ShellControl::Continue, Vec::new()),
    }
}

/// Routes one terminal mouse event through the layout that drew the screen.
///
/// The mouse resolves to a Command and goes through `App::invoke`, exactly as a
/// Keybinding does, so it can never take a shortcut past the work a Command
/// carries with it.
#[cfg(test)]
fn handle_mouse(
    app: &mut App,
    event: crossterm::event::MouseEvent,
    layout: Option<&presentation::ScreenLayout>,
) -> Vec<ProviderRequest> {
    resolve_mouse_command(app, event, layout).map_or_else(Vec::new, |command| app.invoke(command))
}

fn resolve_mouse_command(
    app: &App,
    event: crossterm::event::MouseEvent,
    layout: Option<&presentation::ScreenLayout>,
) -> Option<Command> {
    let (layout, input) = (layout?, presentation::mouse_from_event(event)?);
    presentation::resolve_mouse(layout, input, app.state().pane_boundary.grab())
}

fn persist_pane_boundary(app: &mut App, env: &StateEnv, storage: &dyn StateStorage) {
    let Some(boundary) = app.take_pending_pane_boundary_save() else {
        return;
    };
    if let Err(error) = save_pane_boundary(env, storage, boundary) {
        app.report_pane_boundary_persistence_failure(error.to_string());
    }
}

/// Starts application-requested sessions at the host boundary. Process, PTY,
/// event-loop, and emulator ownership never enter `AppState`.
fn dispatch_resource_shell_effects(
    app: &mut App,
    runtime: &mut ResourceShellRuntime,
    events: &mpsc::UnboundedSender<ResourceShellRuntimeEvent>,
    layout: Option<&presentation::ScreenLayout>,
) -> Vec<ProviderRequest> {
    let size = layout
        .and_then(|layout| layout.panes.as_ref())
        .map(|panes| panes.detail_content)
        .unwrap_or(ratatui::layout::Rect::new(0, 0, 80, 24));
    let mut requests = Vec::new();
    for effect in app.take_resource_shell_effects() {
        match effect {
            ResourceShellEffect::Start { session, process } => {
                let event = match runtime.start(
                    session.id,
                    &process,
                    size.width.max(2),
                    size.height.max(1),
                    events.clone(),
                ) {
                    Ok(()) => AppEvent::ResourceShellStarted {
                        session_id: session.id,
                    },
                    Err(error) => AppEvent::ResourceShellStartFailed {
                        session_id: session.id,
                        reason: error.to_string(),
                    },
                };
                requests.extend(app.update(event));
            }
            ResourceShellEffect::Stop { session_id } => runtime.stop(session_id),
        }
    }
    requests
}

/// Encodes the common interactive keys without asking the application to know
/// about PTYs or terminal engines. Alacritty remains the owner of output
/// parsing and terminal state.
fn terminal_key_bytes(event: KeyEvent) -> Option<Vec<u8>> {
    let bytes = match event.code {
        KeyCode::Char(character) if event.modifiers.contains(KeyModifiers::CONTROL) => {
            vec![(character.to_ascii_uppercase() as u8) & 0x1f]
        }
        KeyCode::Char(character) => character.to_string().into_bytes(),
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Backspace => vec![0x7f],
        KeyCode::Esc => vec![0x1b],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::Up => b"\x1b[A".to_vec(),
        KeyCode::Down => b"\x1b[B".to_vec(),
        KeyCode::Right => b"\x1b[C".to_vec(),
        KeyCode::Left => b"\x1b[D".to_vec(),
        KeyCode::Home => b"\x1b[H".to_vec(),
        KeyCode::End => b"\x1b[F".to_vec(),
        KeyCode::Delete => b"\x1b[3~".to_vec(),
        _ => return None,
    };
    Some(bytes)
}

async fn run(
    terminal: &mut DefaultTerminal,
    registry: CommandRegistry,
    pane_boundary: tuivir::application::PaneBoundary,
    state_env: StateEnv,
) -> io::Result<()> {
    let cli = Arc::new(TokioCliRunner) as Arc<dyn CliRunner>;
    let runtime = ProviderRuntime::with_builtin_providers(cli);
    let (completion_tx, mut completion_rx) = mpsc::unbounded_channel();
    let mut app = App::with_registry_and_pane_boundary(registry, pane_boundary);
    let state_storage = FileSystemReader;
    let mut clipboard = Osc52Clipboard(io::stdout());
    let mut detail_dispatch = DetailDispatchQueue::new(DETAIL_DISPATCH_QUIET_PERIOD);
    let (resource_shell_events, mut resource_shell_event_rx) = mpsc::unbounded_channel();
    let mut resource_shell_runtime = ResourceShellRuntime::default();

    for discovered in runtime.discover().await {
        let requests = app.update(discovered.into_event());
        dispatch_all(&runtime, &completion_tx, &mut detail_dispatch, requests);
    }

    let (key_tx, mut key_rx) = mpsc::unbounded_channel();
    let mut input = InputThread::start(key_tx);
    let mut refresh_timer = RefreshTimer::new();

    let result = loop {
        // Nothing has been drawn yet, so there is nothing to point at.
        let mut layout: Option<presentation::ScreenLayout> = None;
        if let Err(error) = terminal.draw(|frame| {
            let measured = presentation::ScreenLayout::measure(app.state(), frame.area());
            presentation::render_with_layout(app.state(), frame, &measured);
            if let (Some(session), Some(panes)) = (
                app.state().visible_resource_shell_session(),
                measured.panes.as_ref(),
            ) && session.lifecycle == ResourceShellSessionLifecycle::Running
            {
                let _ = resource_shell_runtime.resize(
                    session.id,
                    panes.detail_content.width,
                    panes.detail_content.height,
                );
                if let Some(screen) = resource_shell_runtime.screen_text(session.id) {
                    presentation::render_resource_shell_text(&screen, frame, panes.detail_content);
                }
            }
            layout = Some(measured);
        }) {
            break Err(error);
        }

        let detail_deadline = detail_dispatch.deadline();
        tokio::select! {
            biased;
            Some(event) = key_rx.recv() => {
                let (control, requests) = match event {
                    Event::Key(key) => {
                        if let Some(session) = app.state().visible_resource_shell_session()
                            && session.lifecycle == ResourceShellSessionLifecycle::Running
                            && let Some(bytes) = terminal_key_bytes(key)
                        {
                            let _ = resource_shell_runtime.write(session.id, bytes);
                            (ShellControl::Continue, Vec::new())
                        } else {
                            let command = resolve_key_command(&app, key);
                            let (control, requests) = handle_command(&mut app, command);
                            (control, requests)
                        }
                    }
                    Event::Mouse(mouse) => {
                        let command = resolve_mouse_command(&app, mouse, layout.as_ref());
                        let requests = command.map_or_else(Vec::new, |command| app.invoke(command));
                        (ShellControl::Continue, requests)
                    }
                    _ => (ShellControl::Continue, Vec::new()),
                };
                persist_pane_boundary(&mut app, &state_env, &state_storage);
                dispatch_all(&runtime, &completion_tx, &mut detail_dispatch, requests);
                let requests = dispatch_resource_shell_effects(
                    &mut app,
                    &mut resource_shell_runtime,
                    &resource_shell_events,
                    layout.as_ref(),
                );
                dispatch_all(&runtime, &completion_tx, &mut detail_dispatch, requests);
                copy_pending_details(&mut app, &mut clipboard);
                if control == ShellControl::Quit {
                    break Ok(());
                }
            }
            Some(event) = completion_rx.recv() => {
                let requests = app.update(event);
                dispatch_all(&runtime, &completion_tx, &mut detail_dispatch, requests);
                let requests = dispatch_resource_shell_effects(
                    &mut app,
                    &mut resource_shell_runtime,
                    &resource_shell_events,
                    layout.as_ref(),
                );
                dispatch_all(&runtime, &completion_tx, &mut detail_dispatch, requests);
            }
            Some(event) = resource_shell_event_rx.recv() => {
                let requests = match event {
                    ResourceShellRuntimeEvent::OutputReady { .. } => Vec::new(),
                    ResourceShellRuntimeEvent::Exited { session_id } => {
                        app.update(AppEvent::ResourceShellExited { session_id })
                    }
                };
                dispatch_all(&runtime, &completion_tx, &mut detail_dispatch, requests);
            }
            _ = refresh_timer.tick() => {
                let requests = app.update(AppEvent::RefreshTimerElapsed);
                dispatch_all(&runtime, &completion_tx, &mut detail_dispatch, requests);
            }
            _ = wait_for_detail_dispatch(detail_deadline) => {
                if let Some(request) = detail_dispatch.take_ready(Instant::now())
                    && app.detail_request_is_current(&request)
                {
                    runtime.dispatch(request, completion_tx.clone());
                }
            }
        }
    };

    input.stop();
    result
}

/// The blocking terminal reader publishes ordinary Tuivir input events.
struct InputThread {
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl InputThread {
    fn start(keys: mpsc::UnboundedSender<Event>) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let handle = spawn_input_thread(keys.clone(), Arc::clone(&stop));
        Self {
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
    keys: mpsc::UnboundedSender<Event>,
    stop: Arc<AtomicBool>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        while !stop.load(Ordering::Relaxed) {
            match event::poll(Duration::from_millis(100)) {
                Ok(true) => match event::read() {
                    Ok(event @ (Event::Key(_) | Event::Mouse(_))) => {
                        if keys.send(event).is_err() {
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
