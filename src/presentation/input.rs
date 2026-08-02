use crossterm::event::{KeyCode as TerminalCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::application::{Key, Named};

/// Normalizes one terminal key press into the application's logical vocabulary.
///
/// Anything that is not a representable press is ignored rather than guessed.
pub fn key_from_event(event: KeyEvent) -> Option<Key> {
    if event.kind != KeyEventKind::Press {
        return None;
    }
    let key = match event.code {
        // A shifted character already arrives as the character it produces,
        // so the shift modifier carries no further information.
        TerminalCode::Char(character) => Key::character(character),
        TerminalCode::Backspace => Key::named(Named::Backspace),
        TerminalCode::Enter => Key::named(Named::Enter),
        TerminalCode::Esc => Key::named(Named::Esc),
        TerminalCode::Tab => Key::named(Named::Tab),
        TerminalCode::BackTab => Key::named(Named::BackTab),
        TerminalCode::Left => Key::named(Named::Left),
        TerminalCode::Right => Key::named(Named::Right),
        TerminalCode::Up => Key::named(Named::Up),
        TerminalCode::Down => Key::named(Named::Down),
        TerminalCode::Home => Key::named(Named::Home),
        TerminalCode::End => Key::named(Named::End),
        TerminalCode::PageUp => Key::named(Named::PageUp),
        TerminalCode::PageDown => Key::named(Named::PageDown),
        TerminalCode::Insert => Key::named(Named::Insert),
        TerminalCode::Delete => Key::named(Named::Delete),
        TerminalCode::F(number @ 1..=12) => Key::named(Named::Function(number)),
        _ => return None,
    };
    let key = if event.modifiers.contains(KeyModifiers::CONTROL) {
        key.with_ctrl()
    } else {
        key
    };
    Some(if event.modifiers.contains(KeyModifiers::ALT) {
        key.with_alt()
    } else {
        key
    })
}
