//! The binary host's Alacritty PTY and terminal-emulator runtime.
//!
//! Application state sees stable [`ResourceShellSessionId`] values and lifecycle
//! facts only. This registry owns the child process, PTY, event-loop thread,
//! and emulator for each live session.

use std::{
    borrow::Cow,
    collections::HashMap,
    io,
    sync::{Arc, Mutex},
    thread::JoinHandle,
};

use alacritty_terminal::{
    Term,
    event::{Event, EventListener, WindowSize},
    event_loop::{EventLoop, EventLoopSender, Msg, State},
    grid::Dimensions,
    sync::FairMutex,
    term::{Config as TermConfig, Osc52},
    tty::{self, Options, Shell},
    vte::ansi::{Color as AnsiColor, NamedColor},
};
use ratatui::style::Color;
use tokio::sync::mpsc::UnboundedSender;

use tuivir::application::{ResourceShellProcess, ResourceShellSessionId};
use tuivir::presentation::{ResourceShellCell, ResourceShellScreen};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// A fact the private PTY runtime publishes to its host.
pub enum ResourceShellRuntimeEvent {
    OutputReady { session_id: ResourceShellSessionId },
    Exited { session_id: ResourceShellSessionId },
}

#[derive(Default)]
/// Owns every live process and terminal engine without exposing either to
/// application state.
pub struct ResourceShellRuntime {
    sessions: HashMap<ResourceShellSessionId, LiveResourceShell>,
}

struct LiveResourceShell {
    terminal: Arc<FairMutex<Term<SessionListener>>>,
    input: EventLoopSender,
    event_loop: JoinHandle<(EventLoop<tty::Pty, SessionListener>, State)>,
}

#[derive(Clone)]
struct SessionListener {
    session_id: ResourceShellSessionId,
    events: UnboundedSender<ResourceShellRuntimeEvent>,
    input: Arc<Mutex<Option<EventLoopSender>>>,
}

impl EventListener for SessionListener {
    fn send_event(&self, event: Event) {
        match event {
            Event::Wakeup => {
                let _ = self.events.send(ResourceShellRuntimeEvent::OutputReady {
                    session_id: self.session_id,
                });
            }
            Event::ChildExit(_) => {
                let _ = self.events.send(ResourceShellRuntimeEvent::Exited {
                    session_id: self.session_id,
                });
            }
            // Terminal queries are part of the bidirectional terminal
            // protocol. Clipboard requests are intentionally ignored.
            Event::PtyWrite(reply) => {
                if let Some(input) = self.input.lock().expect("terminal input lock").as_ref() {
                    let _ = input.send(Msg::Input(Cow::Owned(reply.into_bytes())));
                }
            }
            _ => {}
        }
    }
}

struct TerminalSize {
    columns: usize,
    lines: usize,
}

impl Dimensions for TerminalSize {
    fn total_lines(&self) -> usize {
        self.lines
    }

    fn screen_lines(&self) -> usize {
        self.lines
    }

    fn columns(&self) -> usize {
        self.columns
    }
}

impl ResourceShellRuntime {
    /// Starts exactly the Provider-declared executable and argument vector in
    /// a private PTY, then drives Alacritty's terminal emulator on its own
    /// thread.
    pub fn start(
        &mut self,
        session_id: ResourceShellSessionId,
        process: &ResourceShellProcess,
        columns: u16,
        lines: u16,
        events: UnboundedSender<ResourceShellRuntimeEvent>,
    ) -> io::Result<()> {
        if self.sessions.contains_key(&session_id) {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "Resource Shell Session already has a live runtime",
            ));
        }
        let size = TerminalSize {
            columns: usize::from(columns.max(2)),
            lines: usize::from(lines.max(1)),
        };
        let input = Arc::new(Mutex::new(None));
        let listener = SessionListener {
            session_id,
            events,
            input: Arc::clone(&input),
        };
        let config = TermConfig {
            scrolling_history: 10_000,
            osc52: Osc52::Disabled,
            ..TermConfig::default()
        };
        let terminal = Arc::new(FairMutex::new(Term::new(config, &size, listener.clone())));
        let pty_options = Options {
            shell: Some(Shell::new(
                process.program().to_owned(),
                process.args().to_vec(),
            )),
            drain_on_exit: true,
            ..Options::default()
        };
        let window_size = WindowSize {
            num_lines: lines.max(1),
            num_cols: columns.max(2),
            cell_width: 1,
            cell_height: 1,
        };
        let pty = tty::new(&pty_options, window_size, session_id.value())?;
        let event_loop = EventLoop::new(Arc::clone(&terminal), listener, pty, true, false)?;
        let event_loop_sender = event_loop.channel();
        *input.lock().expect("terminal input lock") = Some(event_loop_sender.clone());
        let event_loop = event_loop.spawn();
        self.sessions.insert(
            session_id,
            LiveResourceShell {
                terminal,
                input: event_loop_sender,
                event_loop,
            },
        );
        Ok(())
    }

    /// Encodes input bytes supplied by the host into the session's PTY.
    pub fn write(&self, session_id: ResourceShellSessionId, bytes: Vec<u8>) -> io::Result<()> {
        let Some(session) = self.sessions.get(&session_id) else {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "unknown Resource Shell Session",
            ));
        };
        session
            .input
            .send(Msg::Input(Cow::Owned(bytes)))
            .map_err(|error| io::Error::other(error.to_string()))
    }

    /// Keeps the emulator and its private PTY aligned with the Details
    /// rectangle currently visible to the user.
    pub fn resize(
        &mut self,
        session_id: ResourceShellSessionId,
        columns: u16,
        lines: u16,
    ) -> io::Result<()> {
        let Some(session) = self.sessions.get_mut(&session_id) else {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "unknown Resource Shell Session",
            ));
        };
        let columns = columns.max(2);
        let lines = lines.max(1);
        session.terminal.lock().resize(TerminalSize {
            columns: usize::from(columns),
            lines: usize::from(lines),
        });
        session
            .input
            .send(Msg::Resize(WindowSize {
                num_lines: lines,
                num_cols: columns,
                cell_width: 1,
                cell_height: 1,
            }))
            .map_err(|error| io::Error::other(error.to_string()))
    }

    /// Terminates and reaps the private PTY event loop for one session.
    /// Dropping Alacritty's Unix PTY sends SIGHUP to the child and waits for it,
    /// so no provider process outlives the Resource that owned it.
    pub fn stop(&mut self, session_id: ResourceShellSessionId) {
        let Some(session) = self.sessions.remove(&session_id) else {
            return;
        };
        let _ = session.input.send(Msg::Shutdown);
        let _ = session.event_loop.join();
    }

    /// Adapts the emulator's visible grid into presentation-neutral cells.
    pub fn screen(&self, session_id: ResourceShellSessionId) -> Option<ResourceShellScreen> {
        let session = self.sessions.get(&session_id)?;
        let terminal = session.terminal.lock();
        let content = terminal.renderable_content();
        let columns = terminal.grid().columns();
        let cursor = content.cursor.point;
        let cursor_index = usize::try_from(cursor.line.0)
            .ok()
            .and_then(|line| line.checked_mul(columns))
            .and_then(|offset| offset.checked_add(cursor.column.0));
        let mut lines = Vec::new();
        let mut line = Vec::new();
        for (index, cell) in content.display_iter.enumerate() {
            let mut text = cell.c.to_string();
            if let Some(zerowidth) = cell.zerowidth() {
                text.extend(zerowidth);
            }
            line.push(ResourceShellCell {
                text,
                foreground: terminal_color(cell.fg),
                background: terminal_color(cell.bg),
                cursor: cursor_index == Some(index),
            });
            if (index + 1) % columns == 0 {
                lines.push(line);
                line = Vec::new();
            }
        }
        Some(ResourceShellScreen { lines })
    }

    /// Flattens the emulator's visible grid for focused acceptance tests.
    pub fn screen_text(&self, session_id: ResourceShellSessionId) -> Option<String> {
        let session = self.sessions.get(&session_id)?;
        let terminal = session.terminal.lock();
        let columns = terminal.grid().columns();
        let mut lines = Vec::new();
        let mut line = String::new();
        for (index, cell) in terminal.renderable_content().display_iter.enumerate() {
            line.push(cell.c);
            if let Some(zerowidth) = cell.zerowidth() {
                line.extend(zerowidth);
            }
            if (index + 1) % columns == 0 {
                lines.push(line.trim_end().to_owned());
                line = String::new();
            }
        }
        Some(lines.join("\n"))
    }
}

fn terminal_color(color: AnsiColor) -> Option<Color> {
    match color {
        AnsiColor::Spec(rgb) => Some(Color::Rgb(rgb.r, rgb.g, rgb.b)),
        AnsiColor::Indexed(index) => Some(Color::Indexed(index)),
        AnsiColor::Named(NamedColor::Black) => Some(Color::Black),
        AnsiColor::Named(NamedColor::Red) => Some(Color::Red),
        AnsiColor::Named(NamedColor::Green) => Some(Color::Green),
        AnsiColor::Named(NamedColor::Yellow) => Some(Color::Yellow),
        AnsiColor::Named(NamedColor::Blue) => Some(Color::Blue),
        AnsiColor::Named(NamedColor::Magenta) => Some(Color::Magenta),
        AnsiColor::Named(NamedColor::Cyan) => Some(Color::Cyan),
        AnsiColor::Named(NamedColor::White) => Some(Color::Gray),
        AnsiColor::Named(NamedColor::BrightBlack) => Some(Color::DarkGray),
        AnsiColor::Named(NamedColor::BrightRed) => Some(Color::LightRed),
        AnsiColor::Named(NamedColor::BrightGreen) => Some(Color::LightGreen),
        AnsiColor::Named(NamedColor::BrightYellow) => Some(Color::LightYellow),
        AnsiColor::Named(NamedColor::BrightBlue) => Some(Color::LightBlue),
        AnsiColor::Named(NamedColor::BrightMagenta) => Some(Color::LightMagenta),
        AnsiColor::Named(NamedColor::BrightCyan) => Some(Color::LightCyan),
        AnsiColor::Named(NamedColor::BrightWhite) => Some(Color::White),
        AnsiColor::Named(_) => None,
    }
}
