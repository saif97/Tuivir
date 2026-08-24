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
        self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    },
    execute,
};
use ratatui::{DefaultTerminal, layout::Rect};
use tokio::sync::mpsc;

mod resource_shell_runtime;

use resource_shell_runtime::{ResourceShellRuntime, ResourceShellRuntimeEvent};
use tuivir::{
    application::{
        App, AppEvent, Command, CommandRegistry, ProviderRequest, ResourceShellEffect,
        ResourceShellSessionId,
    },
    infrastructure::{
        config::{Env, FileSystemReader, load},
        pane_boundary_state::{Env as StateEnv, StateStorage, save as save_pane_boundary},
        process::{CliRunner, TokioCliRunner},
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
    if let Err(error) = execute!(io::stdout(), EnableMouseCapture, EnableBracketedPaste) {
        ratatui::restore();
        return Err(error);
    }
    // `ratatui::init` gives back raw mode and the alternate screen on a panic,
    // but it never enabled mouse capture and so cannot know to turn it off.
    // Without this a panic leaves the user's terminal spitting escape sequences
    // at every movement of the mouse.
    let restore_screen = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic| {
        let _ = execute!(io::stdout(), DisableMouseCapture, DisableBracketedPaste);
        restore_screen(panic);
    }));
    let result = run(&mut terminal, registry, pane_boundary, state_env).await;
    let _ = execute!(io::stdout(), DisableMouseCapture, DisableBracketedPaste);
    ratatui::restore();
    result
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ShellControl {
    Continue,
    Quit,
}

/// Host-local keyboard focus for the currently visible PTY. `Ctrl-B q`
/// releases it, returning ordinary keys to Tuivir without ever sending the
/// prefix or release gesture to the provider process.
#[derive(Default)]
struct ShellInputRouter {
    active_session: Option<ResourceShellSessionId>,
    focused: bool,
    prefix_pending: bool,
    selection_gesture: ShellSelectionGesture,
}

enum ShellKeyRoute {
    ToPty(Vec<u8>),
    Released,
    ToggleSize,
    ToTuivir,
}

enum ShellPointerRoute {
    ToPty(Vec<u8>),
    Select { start: (u16, u16), end: (u16, u16) },
    Scroll { lines: i32 },
    ToTuivir,
    None,
}

#[derive(Default)]
enum ShellSelectionGesture {
    #[default]
    Idle,
    Pressed {
        session_id: ResourceShellSessionId,
        start: (u16, u16),
    },
    Dragging {
        session_id: ResourceShellSessionId,
        start: (u16, u16),
    },
}

impl ShellInputRouter {
    fn route(&mut self, session_id: ResourceShellSessionId, key: KeyEvent) -> ShellKeyRoute {
        if self.active_session != Some(session_id) {
            self.active_session = Some(session_id);
            self.focused = true;
            self.prefix_pending = false;
        }
        if self.prefix_pending {
            self.prefix_pending = false;
            if key.code == KeyCode::Char('q') && key.modifiers.is_empty() {
                self.focused = false;
                return ShellKeyRoute::Released;
            }
            if key.code == KeyCode::Char('z') && key.modifiers.is_empty() {
                return ShellKeyRoute::ToggleSize;
            }
            if key.code == KeyCode::Char('b') && key.modifiers == KeyModifiers::CONTROL {
                return ShellKeyRoute::ToPty(vec![0x02]);
            }
            if let Some(bytes) = terminal_key_bytes(key) {
                return ShellKeyRoute::ToPty([&[0x02][..], bytes.as_slice()].concat());
            }
            return ShellKeyRoute::ToTuivir;
        }
        if !self.focused {
            if key.code == KeyCode::Enter && key.modifiers.is_empty() {
                self.focused = true;
                return ShellKeyRoute::ToPty(b"\r".to_vec());
            }
            return ShellKeyRoute::ToTuivir;
        }
        if key.code == KeyCode::Char('b') && key.modifiers == KeyModifiers::CONTROL {
            self.prefix_pending = true;
            return ShellKeyRoute::ToTuivir;
        }
        terminal_key_bytes(key).map_or(ShellKeyRoute::ToTuivir, ShellKeyRoute::ToPty)
    }

    /// Reserves only Tuivir's terminal-prefix controls while no live PTY can
    /// receive input. This keeps the enlarged exit and start-failure screens
    /// escapable without pretending their sessions can accept ordinary keys.
    fn route_without_terminal(
        &mut self,
        session_id: ResourceShellSessionId,
        key: KeyEvent,
    ) -> ShellKeyRoute {
        if self.active_session != Some(session_id) {
            self.active_session = Some(session_id);
            self.focused = false;
            self.prefix_pending = false;
        }
        if self.prefix_pending {
            self.prefix_pending = false;
            return match (key.code, key.modifiers) {
                (KeyCode::Char('q'), modifiers) if modifiers.is_empty() => ShellKeyRoute::Released,
                (KeyCode::Char('z'), modifiers) if modifiers.is_empty() => {
                    ShellKeyRoute::ToggleSize
                }
                _ => ShellKeyRoute::ToTuivir,
            };
        }
        if key.code == KeyCode::Char('b') && key.modifiers == KeyModifiers::CONTROL {
            self.prefix_pending = true;
        }
        ShellKeyRoute::ToTuivir
    }

    fn route_paste(
        &mut self,
        session_id: ResourceShellSessionId,
        text: &str,
        bracketed_paste: bool,
    ) -> ShellKeyRoute {
        if self.active_session != Some(session_id) {
            self.active_session = Some(session_id);
            self.focused = true;
            self.prefix_pending = false;
        }
        if !self.focused {
            return ShellKeyRoute::ToTuivir;
        }
        let mut bytes = Vec::with_capacity(text.len() + if bracketed_paste { 12 } else { 0 });
        if bracketed_paste {
            bytes.extend_from_slice(b"\x1b[200~");
        }
        bytes.extend_from_slice(text.as_bytes());
        if bracketed_paste {
            bytes.extend_from_slice(b"\x1b[201~");
        }
        ShellKeyRoute::ToPty(bytes)
    }

    fn route_mouse(
        &mut self,
        session_id: ResourceShellSessionId,
        event: MouseEvent,
        viewport: Rect,
        mouse_reporting: bool,
        sgr_mouse: bool,
    ) -> ShellPointerRoute {
        let Some(position) = terminal_position(viewport, event) else {
            self.focused = false;
            self.prefix_pending = false;
            self.selection_gesture = ShellSelectionGesture::Idle;
            return ShellPointerRoute::ToTuivir;
        };
        if self.active_session != Some(session_id) {
            self.active_session = Some(session_id);
            self.focused = false;
            self.prefix_pending = false;
        }
        if mouse_reporting && self.focused {
            return terminal_mouse_bytes(event, position, sgr_mouse)
                .map_or(ShellPointerRoute::None, ShellPointerRoute::ToPty);
        }
        match event.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if mouse_reporting {
                    self.focused = true;
                    ShellPointerRoute::None
                } else {
                    self.selection_gesture = ShellSelectionGesture::Pressed {
                        session_id,
                        start: position,
                    };
                    ShellPointerRoute::None
                }
            }
            MouseEventKind::Drag(MouseButton::Left) if !mouse_reporting => {
                let start = match self.selection_gesture {
                    ShellSelectionGesture::Pressed {
                        session_id: selected_session,
                        start,
                    }
                    | ShellSelectionGesture::Dragging {
                        session_id: selected_session,
                        start,
                    } if selected_session == session_id => start,
                    _ => return ShellPointerRoute::None,
                };
                self.selection_gesture = ShellSelectionGesture::Dragging { session_id, start };
                self.focused = false;
                ShellPointerRoute::Select {
                    start,
                    end: position,
                }
            }
            MouseEventKind::Up(MouseButton::Left) if !mouse_reporting => {
                let gesture = std::mem::take(&mut self.selection_gesture);
                if matches!(gesture, ShellSelectionGesture::Pressed { session_id: id, .. } if id == session_id)
                {
                    self.focused = true;
                }
                ShellPointerRoute::None
            }
            MouseEventKind::ScrollUp if !mouse_reporting => ShellPointerRoute::Scroll { lines: 3 },
            MouseEventKind::ScrollDown if !mouse_reporting => {
                ShellPointerRoute::Scroll { lines: -3 }
            }
            _ => ShellPointerRoute::None,
        }
    }
}

fn terminal_position(viewport: Rect, event: MouseEvent) -> Option<(u16, u16)> {
    (event.column >= viewport.x
        && event.column < viewport.right()
        && event.row >= viewport.y
        && event.row < viewport.bottom())
    .then_some((event.column - viewport.x, event.row - viewport.y))
}

fn terminal_mouse_bytes(
    event: MouseEvent,
    position: (u16, u16),
    sgr_mouse: bool,
) -> Option<Vec<u8>> {
    let (code, suffix) = match event.kind {
        MouseEventKind::Down(MouseButton::Left) => (0, 'M'),
        MouseEventKind::Drag(MouseButton::Left) => (32, 'M'),
        MouseEventKind::Up(MouseButton::Left) => (3, 'm'),
        MouseEventKind::ScrollUp => (64, 'M'),
        MouseEventKind::ScrollDown => (65, 'M'),
        _ => return None,
    };
    if sgr_mouse {
        Some(format!("\x1b[<{code};{};{}{suffix}", position.0 + 1, position.1 + 1).into_bytes())
    } else {
        let column = u8::try_from(position.0 + 33).ok()?;
        let row = u8::try_from(position.1 + 33).ok()?;
        Some(vec![0x1b, b'[', b'M', code + 32, column, row])
    }
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
        Some(command) => {
            let requests = app.invoke(command);
            let control = if app.quit_is_ready() {
                ShellControl::Quit
            } else {
                ShellControl::Continue
            };
            (control, requests)
        }
        None => (ShellControl::Continue, Vec::new()),
    }
}

/// Releases terminal focus. From an enlarged session, the same gesture also
/// restores the normal Details presentation; from Details it changes no
/// application state.
fn release_resource_shell(app: &mut App) -> Vec<ProviderRequest> {
    if app.state().enlarged_resource_shell_session().is_some() {
        app.invoke(Command::ToggleResourceShellSize)
    } else {
        Vec::new()
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
    let effects = app.take_resource_shell_effects();
    if effects.is_empty() {
        return Vec::new();
    }
    let mut requests = Vec::new();
    for effect in effects {
        match effect {
            ResourceShellEffect::Start { session, process } => {
                let size = layout
                    .map(|layout| presentation::ScreenLayout::measure(app.state(), layout.area))
                    .and_then(|layout| layout.resource_shell)
                    .map(|shell| shell.terminal)
                    .unwrap_or(ratatui::layout::Rect::new(0, 0, 80, 24));
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

/// Whether a coalesced PTY output wakeup needs another frame. Hidden sessions
/// keep draining in their private runtime, but their terminal cells are not
/// part of the current frame and therefore must not wake the renderer.
fn resource_shell_output_requires_redraw(app: &App, session_id: ResourceShellSessionId) -> bool {
    app.state()
        .visible_running_resource_shell_session()
        .is_some_and(|session| session.id == session_id)
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
        KeyCode::F(1) => b"\x1bOP".to_vec(),
        KeyCode::F(2) => b"\x1bOQ".to_vec(),
        KeyCode::F(3) => b"\x1bOR".to_vec(),
        KeyCode::F(4) => b"\x1bOS".to_vec(),
        KeyCode::F(number @ 5..=12) => format!(
            "\x1b[{}~",
            [15, 17, 18, 19, 20, 21, 23, 24][usize::from(number - 5)]
        )
        .into_bytes(),
        KeyCode::F(number) => format!("\x1b[{}~", 12 + number).into_bytes(),
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
    let mut shell_input = ShellInputRouter::default();

    for discovered in runtime.discover().await {
        let requests = app.update(discovered.into_event());
        dispatch_all(&runtime, &completion_tx, &mut detail_dispatch, requests);
    }

    let (key_tx, mut key_rx) = mpsc::unbounded_channel();
    let mut input = InputThread::start(key_tx);
    let mut refresh_timer = RefreshTimer::new();
    // Keep the last measured geometry for mouse handling while the host is
    // intentionally idle. In particular, hidden shell output must not force a
    // new frame just to replace a layout it never used.
    let mut layout: Option<presentation::ScreenLayout> = None;
    let mut redraw_needed = true;

    let result = loop {
        if redraw_needed {
            if let Err(error) = terminal.draw(|frame| {
                let measured = presentation::ScreenLayout::measure(app.state(), frame.area());
                presentation::render_with_layout(app.state(), frame, &measured);
                if let (Some(session), Some(shell)) = (
                    app.state().visible_running_resource_shell_session(),
                    measured.resource_shell,
                ) {
                    let _ = resource_shell_runtime.resize(
                        session.id,
                        shell.terminal.width,
                        shell.terminal.height,
                    );
                    if let Some(screen) = resource_shell_runtime.screen(session.id) {
                        presentation::render_resource_shell_screen(screen, frame, shell.terminal);
                    }
                    // This acknowledgement comes only after the visible terminal
                    // has been rendered. Hidden sessions leave their coalesced
                    // wakeup pending while their PTYs continue draining output.
                    resource_shell_runtime.acknowledge_output(session.id);
                }
                layout = Some(measured);
            }) {
                break Err(error);
            }
            redraw_needed = false;
        }

        let detail_deadline = detail_dispatch.deadline();
        tokio::select! {
            biased;
            Some(event) = key_rx.recv() => {
                let (control, requests) = match event {
                    Event::Key(key) => {
                        let enlarged = app
                            .state()
                            .enlarged_resource_shell_session()
                            .is_some();
                        let route = app
                            .state()
                            .visible_running_resource_shell_session()
                            .map(|session| shell_input.route(session.id, key))
                            .or_else(|| {
                                enlarged.then(|| {
                                    app.state()
                                        .visible_resource_shell_session()
                                        .map(|session| {
                                            shell_input.route_without_terminal(session.id, key)
                                        })
                                })?
                            });
                        match route {
                            Some(ShellKeyRoute::ToPty(bytes)) => {
                                let session = app.state().visible_running_resource_shell_session()
                                    .expect("a routed Resource Shell Session stays visible");
                                let _ = resource_shell_runtime.write(session.id, bytes);
                                (ShellControl::Continue, Vec::new())
                            }
                            Some(ShellKeyRoute::Released) => {
                                (ShellControl::Continue, release_resource_shell(&mut app))
                            }
                            Some(ShellKeyRoute::ToggleSize) => (
                                ShellControl::Continue,
                                app.invoke(Command::ToggleResourceShellSize),
                            ),
                            Some(ShellKeyRoute::ToTuivir) | None => {
                                let command = resolve_key_command(&app, key);
                                if command == Some(Command::CopyDetails)
                                    && let Some(session) = app
                                        .state()
                                        .visible_running_resource_shell_session()
                                    && let Some(text) = resource_shell_runtime.selected_text(session.id)
                                {
                                    if let Err(error) = clipboard.copy(&text) {
                                        app.report_details_copy_failure(error.to_string());
                                    }
                                    (ShellControl::Continue, Vec::new())
                                } else {
                                    handle_command(&mut app, command)
                                }
                            }
                        }
                    }
                    Event::Mouse(mouse) => {
                        let route = app
                            .state()
                            .visible_running_resource_shell_session()
                            .and_then(|session| {
                                let viewport = layout.as_ref()?.resource_shell?.terminal;
                                let modes = resource_shell_runtime.input_modes(session.id)?;
                                Some((
                                    session.id,
                                    shell_input.route_mouse(
                                        session.id,
                                        mouse,
                                        viewport,
                                        modes.mouse_reporting,
                                        modes.sgr_mouse,
                                    ),
                                ))
                            });
                        let requests = match route {
                            Some((session_id, ShellPointerRoute::ToPty(bytes))) => {
                                let _ = resource_shell_runtime.write(session_id, bytes);
                                Vec::new()
                            }
                            Some((session_id, ShellPointerRoute::Scroll { lines })) => {
                                let _ = resource_shell_runtime.scroll(session_id, lines);
                                Vec::new()
                            }
                            Some((session_id, ShellPointerRoute::Select { start, end })) => {
                                let _ = resource_shell_runtime.select(session_id, start, end);
                                Vec::new()
                            }
                            Some((_, ShellPointerRoute::None)) => {
                                Vec::new()
                            }
                            Some((_, ShellPointerRoute::ToTuivir)) | None => {
                                resolve_mouse_command(&app, mouse, layout.as_ref())
                                    .map_or_else(Vec::new, |command| app.invoke(command))
                            }
                        };
                        (ShellControl::Continue, requests)
                    }
                    Event::Paste(text) => {
                        if let Some(session) = app
                            .state()
                            .visible_running_resource_shell_session()
                        {
                            let bracketed_paste = resource_shell_runtime
                                .input_modes(session.id)
                                .is_some_and(|modes| modes.bracketed_paste);
                            if let ShellKeyRoute::ToPty(bytes) =
                                shell_input.route_paste(session.id, &text, bracketed_paste)
                            {
                                let _ = resource_shell_runtime.write(session.id, bytes);
                            }
                        }
                        (ShellControl::Continue, Vec::new())
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
                redraw_needed = true;
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
                redraw_needed = true;
            }
            Some(event) = resource_shell_event_rx.recv() => {
                let (requests, requests_redraw) = match event {
                    ResourceShellRuntimeEvent::OutputReady { session_id } => {
                        (Vec::new(), resource_shell_output_requires_redraw(&app, session_id))
                    }
                    ResourceShellRuntimeEvent::Exited { session_id } => {
                        (app.update(AppEvent::ResourceShellExited { session_id }), true)
                    }
                };
                dispatch_all(&runtime, &completion_tx, &mut detail_dispatch, requests);
                redraw_needed |= requests_redraw;
            }
            _ = refresh_timer.tick() => {
                let requests = app.update(AppEvent::RefreshTimerElapsed);
                dispatch_all(&runtime, &completion_tx, &mut detail_dispatch, requests);
                redraw_needed = true;
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
                    Ok(event @ (Event::Key(_) | Event::Mouse(_) | Event::Paste(_))) => {
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
