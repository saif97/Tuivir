//! PROTOTYPE ONLY — issue #99's Alacritty terminal-engine acceptance harness.
//!
//! This deliberately lives outside Tuivir's application modules. It runs a
//! private PTY through Alacritty's terminal engine and projects that grid into
//! Ratatui. Delete it after the engine-selection spike.

use std::{
    borrow::Cow,
    env, io,
    sync::{
        Arc,
        mpsc::{self, Receiver, Sender},
    },
    time::Duration,
};

use alacritty_terminal::{
    event::{Event as AlacrittyEvent, EventListener, WindowSize},
    event_loop::{EventLoop, Msg},
    grid::Dimensions,
    sync::FairMutex,
    term::{Config, Term, TermMode, cell::Flags},
    tty::{self, Options, Shell},
    vte::ansi::{Color as AlacrittyColor, NamedColor, Rgb},
};
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
        KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    },
    execute,
};
use ratatui::{
    DefaultTerminal,
    style::{Color, Modifier, Style},
    widgets::Clear,
};

const CELL_WIDTH: u16 = 8;
const CELL_HEIGHT: u16 = 16;

#[derive(Clone, Copy)]
struct GridSize {
    lines: usize,
    columns: usize,
}

impl Dimensions for GridSize {
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

#[derive(Clone)]
struct EventProxy(Sender<AlacrittyEvent>);

impl EventListener for EventProxy {
    fn send_event(&self, event: AlacrittyEvent) {
        let _ = self.0.send(event);
    }
}

fn main() -> io::Result<()> {
    let shell = shell_from_args()?;
    let mut terminal = ratatui::init();
    execute!(io::stdout(), EnableMouseCapture)?;
    let result = run(&mut terminal, shell);
    let _ = execute!(io::stdout(), DisableMouseCapture);
    ratatui::restore();
    result
}

fn shell_from_args() -> io::Result<Shell> {
    let args: Vec<_> = env::args().skip(1).collect();
    match args.as_slice() {
        [flag] if flag == "--btop" => Ok(Shell::new("btop".into(), vec![])),
        [] => Ok(Shell::new(
            env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into()),
            vec![],
        )),
        [flag, container, rest @ ..] if flag == "--docker" => Ok(Shell::new(
            "docker".into(),
            [
                vec!["exec".into(), "-it".into(), container.clone()],
                rest.to_vec(),
            ]
            .concat(),
        )),
        [flag, program, rest @ ..] if flag == "--command" => {
            Ok(Shell::new(program.clone(), rest.to_vec()))
        }
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "usage: cargo run --bin alacritty_terminal_spike [-- --btop] | [-- --docker CONTAINER [SHELL...]] | [-- --command PROGRAM [ARG...]]",
        )),
    }
}

fn run(outer: &mut DefaultTerminal, shell: Shell) -> io::Result<()> {
    let (event_tx, event_rx) = mpsc::channel();
    let size = grid_size(outer.size()?.into());
    let proxy = EventProxy(event_tx);
    let term = Arc::new(FairMutex::new(Term::new(
        Config::default(),
        &size,
        proxy.clone(),
    )));
    tty::setup_env();
    let pty = tty::new(
        &Options {
            shell: Some(shell),
            drain_on_exit: true,
            ..Options::default()
        },
        window_size(size),
        0,
    )?;
    let event_loop = EventLoop::new(term.clone(), proxy, pty, true, false)?;
    let sender = event_loop.channel();
    let thread = event_loop.spawn();
    let result = interaction_loop(outer, term, sender.clone(), event_rx);
    let _ = sender.send(Msg::Shutdown);
    let _ = thread.join();
    result
}

fn interaction_loop(
    outer: &mut DefaultTerminal,
    term: Arc<FairMutex<Term<EventProxy>>>,
    sender: alacritty_terminal::event_loop::EventLoopSender,
    events: Receiver<AlacrittyEvent>,
) -> io::Result<()> {
    let mut previous_size = grid_size(outer.size()?.into());
    let mut terminal_hidden = false;
    loop {
        drain_engine_events(&events, &sender, previous_size);
        let size = grid_size(outer.size()?.into());
        if size.columns != previous_size.columns || size.lines != previous_size.lines {
            term.lock().resize(size);
            send_resize(&sender, size);
            previous_size = size;
        }
        outer.draw(|frame| {
            if terminal_hidden {
                frame.render_widget(Clear, frame.area());
            } else {
                render_terminal(frame, &term.lock());
            }
        })?;
        if !event::poll(Duration::from_millis(16))? {
            continue;
        }
        match event::read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                if key.code == KeyCode::Char('g') && key.modifiers == KeyModifiers::CONTROL {
                    return Ok(());
                }
                if key.code == KeyCode::Char('h') && key.modifiers == KeyModifiers::CONTROL {
                    terminal_hidden = !terminal_hidden;
                    continue;
                }
                send_input(&sender, key_bytes(key, *term.lock().mode()));
            }
            Event::Paste(text) => {
                let bytes = if term.lock().mode().contains(TermMode::BRACKETED_PASTE) {
                    format!("\x1b[200~{text}\x1b[201~").into_bytes()
                } else {
                    text.into_bytes()
                };
                send_input(&sender, bytes);
            }
            Event::Mouse(mouse) => send_input(&sender, mouse_bytes(mouse, *term.lock().mode())),
            Event::FocusGained if term.lock().mode().contains(TermMode::FOCUS_IN_OUT) => {
                send_input(&sender, b"\x1b[I".to_vec())
            }
            Event::FocusLost if term.lock().mode().contains(TermMode::FOCUS_IN_OUT) => {
                send_input(&sender, b"\x1b[O".to_vec())
            }
            _ => {}
        }
    }
}

fn drain_engine_events(
    events: &Receiver<AlacrittyEvent>,
    sender: &alacritty_terminal::event_loop::EventLoopSender,
    size: GridSize,
) {
    while let Ok(event) = events.try_recv() {
        match event {
            AlacrittyEvent::PtyWrite(text) => send_input(sender, text.into_bytes()),
            AlacrittyEvent::TextAreaSizeRequest(format) => {
                send_input(sender, format(window_size(size)).into_bytes())
            }
            AlacrittyEvent::ClipboardLoad(_, format) => send_input(sender, format("").into_bytes()),
            AlacrittyEvent::ColorRequest(_, format) => {
                send_input(sender, format(Rgb::default()).into_bytes())
            }
            _ => {}
        }
    }
}

fn render_terminal(frame: &mut ratatui::Frame, term: &Term<EventProxy>) {
    if frame.area().width == 0 || frame.area().height == 0 {
        return;
    }
    let content = term.renderable_content();
    let cursor = content.cursor;
    let colors = content.colors;
    let buffer = frame.buffer_mut();
    for indexed in content.display_iter {
        let cell = indexed.cell;
        let x = indexed.point.column.0 as u16;
        let y = indexed.point.line.0 as u16;
        let mut symbol = cell.c.to_string();
        if let Some(combining) = cell.zerowidth() {
            symbol.extend(combining);
        }
        let (mut fg, mut bg) = (
            resolve_color(cell.fg, colors),
            resolve_color(cell.bg, colors),
        );
        if cell.flags.contains(Flags::INVERSE) {
            std::mem::swap(&mut fg, &mut bg);
        }
        let mut modifiers = Modifier::empty();
        if cell.flags.contains(Flags::BOLD) {
            modifiers |= Modifier::BOLD;
        }
        if cell.flags.contains(Flags::DIM) {
            modifiers |= Modifier::DIM;
        }
        if cell.flags.contains(Flags::ITALIC) {
            modifiers |= Modifier::ITALIC;
        }
        if cell.flags.intersects(Flags::ALL_UNDERLINES) {
            modifiers |= Modifier::UNDERLINED;
        }
        if cell.flags.contains(Flags::STRIKEOUT) {
            modifiers |= Modifier::CROSSED_OUT;
        }
        if cell.flags.contains(Flags::HIDDEN) {
            symbol = " ".into();
        }
        buffer[(x, y)]
            .set_symbol(&symbol)
            .set_style(Style::new().fg(fg).bg(bg).add_modifier(modifiers));
    }
    if cursor.shape != alacritty_terminal::vte::ansi::CursorShape::Hidden {
        frame.set_cursor_position((cursor.point.column.0 as u16, cursor.point.line.0 as u16));
    }
}

fn resolve_color(color: AlacrittyColor, colors: &alacritty_terminal::term::color::Colors) -> Color {
    let rgb = match color {
        AlacrittyColor::Spec(rgb) => rgb,
        AlacrittyColor::Indexed(index) => {
            colors[index as usize].unwrap_or_else(|| indexed_rgb(index))
        }
        AlacrittyColor::Named(name) => colors[name].unwrap_or_else(|| named_rgb(name)),
    };
    Color::Rgb(rgb.r, rgb.g, rgb.b)
}

fn indexed_rgb(index: u8) -> Rgb {
    const ANSI: [Rgb; 16] = [
        Rgb { r: 0, g: 0, b: 0 },
        Rgb {
            r: 205,
            g: 49,
            b: 49,
        },
        Rgb {
            r: 13,
            g: 188,
            b: 121,
        },
        Rgb {
            r: 229,
            g: 229,
            b: 16,
        },
        Rgb {
            r: 36,
            g: 114,
            b: 200,
        },
        Rgb {
            r: 188,
            g: 63,
            b: 188,
        },
        Rgb {
            r: 17,
            g: 168,
            b: 205,
        },
        Rgb {
            r: 229,
            g: 229,
            b: 229,
        },
        Rgb {
            r: 102,
            g: 102,
            b: 102,
        },
        Rgb {
            r: 241,
            g: 76,
            b: 76,
        },
        Rgb {
            r: 35,
            g: 209,
            b: 139,
        },
        Rgb {
            r: 245,
            g: 245,
            b: 67,
        },
        Rgb {
            r: 59,
            g: 142,
            b: 234,
        },
        Rgb {
            r: 214,
            g: 112,
            b: 214,
        },
        Rgb {
            r: 41,
            g: 184,
            b: 219,
        },
        Rgb {
            r: 255,
            g: 255,
            b: 255,
        },
    ];
    match index {
        0..=15 => ANSI[index as usize],
        16..=231 => {
            let value = index - 16;
            let component = |part| if part == 0 { 0 } else { 55 + part * 40 };
            Rgb {
                r: component(value / 36),
                g: component(value / 6 % 6),
                b: component(value % 6),
            }
        }
        232..=255 => {
            let shade = 8 + (index - 232) * 10;
            Rgb {
                r: shade,
                g: shade,
                b: shade,
            }
        }
    }
}

fn named_rgb(color: NamedColor) -> Rgb {
    match color {
        NamedColor::Foreground | NamedColor::BrightForeground => indexed_rgb(7),
        NamedColor::Background => indexed_rgb(0),
        NamedColor::Cursor => indexed_rgb(7),
        NamedColor::DimForeground => indexed_rgb(8),
        NamedColor::Black | NamedColor::DimBlack => indexed_rgb(0),
        NamedColor::Red | NamedColor::DimRed => indexed_rgb(1),
        NamedColor::Green | NamedColor::DimGreen => indexed_rgb(2),
        NamedColor::Yellow | NamedColor::DimYellow => indexed_rgb(3),
        NamedColor::Blue | NamedColor::DimBlue => indexed_rgb(4),
        NamedColor::Magenta | NamedColor::DimMagenta => indexed_rgb(5),
        NamedColor::Cyan | NamedColor::DimCyan => indexed_rgb(6),
        NamedColor::White | NamedColor::DimWhite => indexed_rgb(7),
        NamedColor::BrightBlack => indexed_rgb(8),
        NamedColor::BrightRed => indexed_rgb(9),
        NamedColor::BrightGreen => indexed_rgb(10),
        NamedColor::BrightYellow => indexed_rgb(11),
        NamedColor::BrightBlue => indexed_rgb(12),
        NamedColor::BrightMagenta => indexed_rgb(13),
        NamedColor::BrightCyan => indexed_rgb(14),
        NamedColor::BrightWhite => indexed_rgb(15),
    }
}

fn key_bytes(key: KeyEvent, mode: TermMode) -> Vec<u8> {
    let modifiers = key.modifiers;
    let mut bytes = match key.code {
        KeyCode::Char(character) if modifiers.contains(KeyModifiers::CONTROL) => {
            vec![control_byte(character)]
        }
        KeyCode::Char(character) => character.to_string().into_bytes(),
        KeyCode::Enter => b"\r".to_vec(),
        KeyCode::Backspace => b"\x7f".to_vec(),
        KeyCode::Esc => b"\x1b".to_vec(),
        KeyCode::Tab if modifiers.contains(KeyModifiers::SHIFT) => b"\x1b[Z".to_vec(),
        KeyCode::Tab => b"\t".to_vec(),
        KeyCode::Up => cursor_key('A', mode.contains(TermMode::APP_CURSOR), modifiers),
        KeyCode::Down => cursor_key('B', mode.contains(TermMode::APP_CURSOR), modifiers),
        KeyCode::Right => cursor_key('C', mode.contains(TermMode::APP_CURSOR), modifiers),
        KeyCode::Left => cursor_key('D', mode.contains(TermMode::APP_CURSOR), modifiers),
        KeyCode::Home => cursor_key('H', mode.contains(TermMode::APP_CURSOR), modifiers),
        KeyCode::End => cursor_key('F', mode.contains(TermMode::APP_CURSOR), modifiers),
        KeyCode::PageUp => modified_key("5", '~', modifiers),
        KeyCode::PageDown => modified_key("6", '~', modifiers),
        KeyCode::Insert => modified_key("2", '~', modifiers),
        KeyCode::Delete => modified_key("3", '~', modifiers),
        KeyCode::F(number) => function_key(number, modifiers),
        _ => Vec::new(),
    };
    if !bytes.is_empty()
        && modifiers.contains(KeyModifiers::ALT)
        && !matches!(key.code, KeyCode::Esc)
    {
        bytes.insert(0, 0x1b);
    }
    bytes
}

fn control_byte(character: char) -> u8 {
    match character {
        ' ' | '@' => 0,
        '?' => 0x7f,
        character => character.to_ascii_uppercase() as u8 & 0x1f,
    }
}

fn cursor_key(final_byte: char, application: bool, modifiers: KeyModifiers) -> Vec<u8> {
    if modifiers.is_empty() {
        return format!("\x1b{}{}", if application { 'O' } else { '[' }, final_byte).into_bytes();
    }
    format!("\x1b[1;{}{}", modifier_code(modifiers), final_byte).into_bytes()
}

fn modified_key(prefix: &str, final_byte: char, modifiers: KeyModifiers) -> Vec<u8> {
    if modifiers.is_empty() {
        format!("\x1b[{prefix}{final_byte}").into_bytes()
    } else {
        format!("\x1b[{prefix};{}{final_byte}", modifier_code(modifiers)).into_bytes()
    }
}

fn function_key(number: u8, modifiers: KeyModifiers) -> Vec<u8> {
    if (1..=4).contains(&number) && modifiers.is_empty() {
        return format!("\x1bO{}", ['P', 'Q', 'R', 'S'][(number - 1) as usize]).into_bytes();
    }
    let sequence = match number {
        1 => "1",
        2 => "12",
        3 => "13",
        4 => "14",
        5 => "15",
        6 => "17",
        7 => "18",
        8 => "19",
        9 => "20",
        10 => "21",
        11 => "23",
        12 => "24",
        _ => return Vec::new(),
    };
    modified_key(sequence, '~', modifiers)
}

fn modifier_code(modifiers: KeyModifiers) -> u8 {
    1 + u8::from(modifiers.contains(KeyModifiers::SHIFT))
        + 2 * u8::from(modifiers.contains(KeyModifiers::ALT))
        + 4 * u8::from(modifiers.contains(KeyModifiers::CONTROL))
}

fn mouse_bytes(mouse: MouseEvent, mode: TermMode) -> Vec<u8> {
    if !mode.intersects(TermMode::MOUSE_MODE) {
        return Vec::new();
    }
    let (button, release) = match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => (0, false),
        MouseEventKind::Down(MouseButton::Middle) => (1, false),
        MouseEventKind::Down(MouseButton::Right) => (2, false),
        MouseEventKind::Up(_) => (3, true),
        MouseEventKind::Drag(MouseButton::Left)
            if mode.intersects(TermMode::MOUSE_DRAG | TermMode::MOUSE_MOTION) =>
        {
            (32, false)
        }
        MouseEventKind::Moved if mode.contains(TermMode::MOUSE_MOTION) => (35, false),
        MouseEventKind::ScrollUp => (64, false),
        MouseEventKind::ScrollDown => (65, false),
        _ => return Vec::new(),
    };
    let modifiers = 4 * u8::from(mouse.modifiers.contains(KeyModifiers::SHIFT))
        + 8 * u8::from(mouse.modifiers.contains(KeyModifiers::ALT))
        + 16 * u8::from(mouse.modifiers.contains(KeyModifiers::CONTROL));
    let (button, x, y) = (button + modifiers, mouse.column + 1, mouse.row + 1);
    if mode.contains(TermMode::SGR_MOUSE) {
        format!("\x1b[<{button};{x};{y}{}", if release { 'm' } else { 'M' }).into_bytes()
    } else if x <= 223 && y <= 223 {
        vec![0x1b, b'[', b'M', 32 + button, 32 + x as u8, 32 + y as u8]
    } else {
        Vec::new()
    }
}

fn send_input(sender: &alacritty_terminal::event_loop::EventLoopSender, input: Vec<u8>) {
    if !input.is_empty() {
        let _ = sender.send(Msg::Input(Cow::Owned(input)));
    }
}
fn send_resize(sender: &alacritty_terminal::event_loop::EventLoopSender, size: GridSize) {
    let _ = sender.send(Msg::Resize(window_size(size)));
}
fn grid_size(area: ratatui::layout::Rect) -> GridSize {
    GridSize {
        lines: usize::from(area.height.max(1)),
        columns: usize::from(area.width.max(2)),
    }
}
fn window_size(size: GridSize) -> WindowSize {
    WindowSize {
        num_lines: size.lines as u16,
        num_cols: size.columns as u16,
        cell_width: CELL_WIDTH,
        cell_height: CELL_HEIGHT,
    }
}
