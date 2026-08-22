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
    grid::Scroll,
    sync::FairMutex,
    term::{Config as TermConfig, Osc52, TermMode},
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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
/// Input protocol modes currently requested by a Resource Shell Session.
///
/// The host reads this neutral value to encode user input without allowing
/// Alacritty types into application state.
pub struct ResourceShellInputModes {
    pub bracketed_paste: bool,
    pub mouse_reporting: bool,
    pub sgr_mouse: bool,
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
    selection: Option<((u16, u16), (u16, u16))>,
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
                selection: None,
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

    /// Reports the input protocol modes active in a live session.
    pub fn input_modes(
        &self,
        session_id: ResourceShellSessionId,
    ) -> Option<ResourceShellInputModes> {
        let session = self.sessions.get(&session_id)?;
        let terminal = session.terminal.lock();
        Some(ResourceShellInputModes {
            bracketed_paste: terminal.mode().contains(TermMode::BRACKETED_PASTE),
            mouse_reporting: terminal.mode().intersects(TermMode::MOUSE_MODE),
            sgr_mouse: terminal.mode().contains(TermMode::SGR_MOUSE),
        })
    }

    /// Moves the visible terminal viewport through its bounded scrollback.
    pub fn scroll(&mut self, session_id: ResourceShellSessionId, lines: i32) -> io::Result<()> {
        let Some(session) = self.sessions.get_mut(&session_id) else {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "unknown Resource Shell Session",
            ));
        };
        session.terminal.lock().scroll_display(Scroll::Delta(lines));
        Ok(())
    }

    /// Records a user-owned text selection in the visible terminal viewport.
    pub fn select(
        &mut self,
        session_id: ResourceShellSessionId,
        start: (u16, u16),
        end: (u16, u16),
    ) -> io::Result<()> {
        let Some(session) = self.sessions.get_mut(&session_id) else {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "unknown Resource Shell Session",
            ));
        };
        session.selection = Some((start, end));
        Ok(())
    }

    /// Returns the text explicitly selected by the user, without accepting
    /// terminal-originated clipboard requests.
    pub fn selected_text(&self, session_id: ResourceShellSessionId) -> Option<String> {
        let screen = self.screen(session_id)?;
        let text = screen
            .lines
            .into_iter()
            .map(|line| {
                line.into_iter()
                    .filter(|cell| cell.selected)
                    .map(|cell| cell.text)
                    .collect::<String>()
                    .trim_end()
                    .to_owned()
            })
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        (!text.is_empty()).then_some(text)
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
                selected: session.selection.is_some_and(|(start, end)| {
                    let position = ((index % columns) as u16, (index / columns) as u16);
                    let (first, last) = if start <= end {
                        (start, end)
                    } else {
                        (end, start)
                    };
                    position >= first && position <= last
                }),
            });
            if (index + 1) % columns == 0 {
                lines.push(line);
                line = Vec::new();
            }
        }
        Some(ResourceShellScreen { lines })
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
