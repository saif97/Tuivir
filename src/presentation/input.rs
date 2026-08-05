use crossterm::event::{
    KeyCode as TerminalCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent,
    MouseEventKind,
};

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MouseAction {
    Press,
    ScrollUp,
    ScrollDown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MouseInput {
    pub action: MouseAction,
    pub column: u16,
    pub row: u16,
}

/// Normalizes terminal mouse events into the small pointer vocabulary the host
/// needs for hit-testing. Motion, release, and non-primary buttons are ignored.
pub fn mouse_from_event(event: MouseEvent) -> Option<MouseInput> {
    let action = match event.kind {
        MouseEventKind::Down(MouseButton::Left) => MouseAction::Press,
        MouseEventKind::ScrollUp => MouseAction::ScrollUp,
        MouseEventKind::ScrollDown => MouseAction::ScrollDown,
        _ => return None,
    };
    Some(MouseInput {
        action,
        column: event.column,
        row: event.row,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_click_and_wheel_but_ignores_motion_and_release() {
        let click = mouse_from_event(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 4,
            row: 8,
            modifiers: KeyModifiers::NONE,
        });
        assert_eq!(
            click,
            Some(MouseInput {
                action: MouseAction::Press,
                column: 4,
                row: 8,
            })
        );
        assert_eq!(
            mouse_from_event(MouseEvent {
                kind: MouseEventKind::ScrollDown,
                column: 4,
                row: 8,
                modifiers: KeyModifiers::NONE,
            })
            .map(|input| input.action),
            Some(MouseAction::ScrollDown)
        );
        assert_eq!(
            mouse_from_event(MouseEvent {
                kind: MouseEventKind::Moved,
                column: 4,
                row: 8,
                modifiers: KeyModifiers::NONE,
            }),
            None
        );
    }
}
