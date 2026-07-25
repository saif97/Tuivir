use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use virtui::keys::{Key, Named};

/// A key press is shown to the user exactly as it is written in configuration,
/// so an inline hint can never drift from the file that produced it.
#[test]
fn a_key_is_displayed_as_the_text_that_configures_it() {
    for text in [
        "S",
        "?",
        "[",
        "+",
        "esc",
        "enter",
        "f12",
        "ctrl+r",
        "alt+enter",
        "shift+tab",
        "ctrl+alt+delete",
    ] {
        let key = Key::parse(text).unwrap_or_else(|_| panic!("{text} is a key"));
        assert_eq!(key.to_string(), text);
    }
}

#[test]
fn terminal_input_is_normalised_to_the_configured_key() {
    for (event, expected) in [
        (
            KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE),
            Key::character('r'),
        ),
        // A shifted character arrives as the character it produces, with the
        // shift modifier still set; the modifier is not part of the key.
        (
            KeyEvent::new(KeyCode::Char('J'), KeyModifiers::SHIFT),
            Key::character('J'),
        ),
        (
            KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL),
            Key::character('r').with_ctrl(),
        ),
        (
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
            Key::named(Named::Esc),
        ),
        (
            KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT),
            Key::named(Named::BackTab),
        ),
        (
            KeyEvent::new(KeyCode::F(5), KeyModifiers::NONE),
            Key::named(Named::Function(5)),
        ),
    ] {
        assert_eq!(
            Key::from_event(event),
            Some(expected),
            "{event:?} should normalise to {expected}"
        );
    }
}

/// Only a press is input; a repeat or release is not a second Command.
#[test]
fn a_key_release_is_not_input() {
    let release = KeyEvent::new_with_kind(KeyCode::Char('q'), KeyModifiers::NONE, KeyEventKind::Release);

    assert_eq!(Key::from_event(release), None);
}

#[test]
fn an_unrepresentable_key_press_is_ignored_rather_than_guessed() {
    let media = KeyEvent::new(KeyCode::CapsLock, KeyModifiers::NONE);

    assert_eq!(Key::from_event(media), None);
}

#[test]
fn a_printable_character_is_written_as_itself() {
    assert_eq!(Key::parse("S").expect("S is a key"), Key::character('S'));
}

#[test]
fn non_printable_keys_are_written_with_familiar_names() {
    for (text, named) in [
        ("space", Named::Space),
        ("backspace", Named::Backspace),
        ("enter", Named::Enter),
        ("esc", Named::Esc),
        ("tab", Named::Tab),
        ("left", Named::Left),
        ("right", Named::Right),
        ("up", Named::Up),
        ("down", Named::Down),
        ("home", Named::Home),
        ("end", Named::End),
        ("pageup", Named::PageUp),
        ("pagedown", Named::PageDown),
        ("insert", Named::Insert),
        ("delete", Named::Delete),
        ("f1", Named::Function(1)),
        ("f12", Named::Function(12)),
    ] {
        assert_eq!(
            Key::parse(text).unwrap_or_else(|_| panic!("{text} is a key")),
            Key::named(named),
            "{text} should parse to {named:?}"
        );
    }
}

#[test]
fn ctrl_and_alt_combine_with_a_key() {
    assert_eq!(
        Key::parse("ctrl+r").expect("ctrl+r is a key"),
        Key::character('r').with_ctrl()
    );
    assert_eq!(
        Key::parse("alt+enter").expect("alt+enter is a key"),
        Key::named(Named::Enter).with_alt()
    );
    assert_eq!(
        Key::parse("ctrl+alt+delete").expect("ctrl+alt+delete is a key"),
        Key::named(Named::Delete).with_ctrl().with_alt()
    );
}

/// Shifted printable input is the character the terminal produces, so the
/// modifier form must not offer a second way to write the same key.
#[test]
fn shift_applies_to_named_keys_but_never_to_a_printable_character() {
    assert_eq!(
        Key::parse("shift+tab").expect("shift+tab is a key"),
        Key::named(Named::BackTab)
    );
    assert!(Key::parse("shift+j").is_err(), "shift+j should be J");
    assert!(Key::parse("shift+?").is_err());
}

#[test]
fn unsupported_modifiers_are_rejected() {
    for text in ["cmd+q", "super+q", "meta+q", "hyper+q"] {
        assert!(
            Key::parse(text).is_err(),
            "{text:?} should not be a valid key"
        );
    }
}

#[test]
fn an_unrecognised_key_name_is_rejected() {
    for text in ["", "escape", "pgup", "f0", "f13", "super", "ab"] {
        assert!(
            Key::parse(text).is_err(),
            "{text:?} should not be a valid key"
        );
    }
}
