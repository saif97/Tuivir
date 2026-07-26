use std::fmt;

use crossterm::event::{KeyCode as TerminalCode, KeyEvent, KeyEventKind, KeyModifiers};

/// One key combination, in the registry's own vocabulary.
///
/// Raw terminal input is normalized into this type at the runtime adapter, so
/// Command routing never sees a crossterm event and configuration never has to
/// describe one.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Key {
    code: KeyCode,
    ctrl: bool,
    alt: bool,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum KeyCode {
    /// Any single printable Unicode character, case-sensitively.
    Character(char),
    Named(Named),
}

/// A key with no printable character, written by a familiar name rather than
/// in Vim notation.
///
/// The spacebar is deliberately absent: it produces a character, so it is a
/// [`KeyCode::Character`] like any other and `space` is only a spelling for it.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Named {
    Backspace,
    Enter,
    Esc,
    Tab,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    Delete,
    /// What a terminal reports for `shift+tab`; it has no name of its own.
    BackTab,
    /// `f1` through `f12`.
    Function(u8),
}

const NAMED_KEYS: [(&str, Named); 14] = [
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
];

impl Named {
    /// Parses an already-lowercased key name.
    fn parse(text: &str) -> Option<Self> {
        if let Some((_, named)) = NAMED_KEYS.iter().find(|(name, _)| *name == text) {
            return Some(*named);
        }
        text.strip_prefix('f')
            .and_then(|number| number.parse::<u8>().ok())
            .filter(|number| (1..=12).contains(number))
            .map(Named::Function)
    }
}

impl Key {
    pub fn character(character: char) -> Self {
        Self::new(KeyCode::Character(character))
    }

    pub fn named(named: Named) -> Self {
        Self::new(KeyCode::Named(named))
    }

    fn new(code: KeyCode) -> Self {
        Self {
            code,
            ctrl: false,
            alt: false,
        }
    }

    pub fn with_ctrl(self) -> Self {
        Self { ctrl: true, ..self }
    }

    pub fn with_alt(self) -> Self {
        Self { alt: true, ..self }
    }

    pub fn parse(text: &str) -> Result<Self, InvalidKey> {
        // Modifiers are stripped one prefix at a time rather than by splitting
        // on `+`, so `+` and `ctrl++` still name the plus character.
        let mut remaining = text;
        let (mut ctrl, mut alt, mut shift) = (false, false, false);
        loop {
            let modifier = [
                ("ctrl+", &mut ctrl),
                ("alt+", &mut alt),
                ("shift+", &mut shift),
            ]
            .into_iter()
            .find_map(|(prefix, flag)| remaining.strip_prefix(prefix).map(|rest| (rest, flag)));
            let Some((rest, flag)) = modifier else { break };
            if *flag {
                return Err(InvalidKey::new(text));
            }
            *flag = true;
            remaining = rest;
        }

        let code = Self::parse_code(remaining).ok_or_else(|| InvalidKey::new(text))?;
        // `shift` only distinguishes keys that have no printable character of
        // their own; a shifted character is written as the character itself.
        let code = match (shift, code) {
            (false, code) => code,
            (true, KeyCode::Named(Named::Tab)) => KeyCode::Named(Named::BackTab),
            (true, _) => return Err(InvalidKey::new(text)),
        };
        Ok(Self { code, ctrl, alt })
    }

    /// Normalizes a terminal key event into the registry's vocabulary.
    ///
    /// Returns `None` for anything that is not a representable press, so an
    /// unknown event is ignored rather than guessed at.
    pub fn from_event(event: KeyEvent) -> Option<Self> {
        if event.kind != KeyEventKind::Press {
            return None;
        }
        let code = match event.code {
            // A shifted character already arrives as the character it produces,
            // so the shift modifier carries no further information.
            TerminalCode::Char(character) => KeyCode::Character(character),
            TerminalCode::Backspace => KeyCode::Named(Named::Backspace),
            TerminalCode::Enter => KeyCode::Named(Named::Enter),
            TerminalCode::Esc => KeyCode::Named(Named::Esc),
            TerminalCode::Tab => KeyCode::Named(Named::Tab),
            TerminalCode::BackTab => KeyCode::Named(Named::BackTab),
            TerminalCode::Left => KeyCode::Named(Named::Left),
            TerminalCode::Right => KeyCode::Named(Named::Right),
            TerminalCode::Up => KeyCode::Named(Named::Up),
            TerminalCode::Down => KeyCode::Named(Named::Down),
            TerminalCode::Home => KeyCode::Named(Named::Home),
            TerminalCode::End => KeyCode::Named(Named::End),
            TerminalCode::PageUp => KeyCode::Named(Named::PageUp),
            TerminalCode::PageDown => KeyCode::Named(Named::PageDown),
            TerminalCode::Insert => KeyCode::Named(Named::Insert),
            TerminalCode::Delete => KeyCode::Named(Named::Delete),
            TerminalCode::F(number @ 1..=12) => KeyCode::Named(Named::Function(number)),
            _ => return None,
        };
        Some(Self {
            code,
            ctrl: event.modifiers.contains(KeyModifiers::CONTROL),
            alt: event.modifiers.contains(KeyModifiers::ALT),
        })
    }

    fn parse_code(text: &str) -> Option<KeyCode> {
        // A single character is always itself, so a one-character name can
        // never shadow a printable key, and `S` stays distinct from `s`.
        let mut characters = text.chars();
        if let (Some(character), None) = (characters.next(), characters.next()) {
            return Some(KeyCode::Character(character));
        }
        // Key names are lowercase literals, so `Esc` is a typo rather than a
        // second spelling: a mistyped binding is reported, never silently
        // reinterpreted.
        //
        // `space` is only a spelling for the character the spacebar produces;
        // writing it as a name must yield the very key that a press produces.
        if text == "space" {
            return Some(KeyCode::Character(' '));
        }
        Named::parse(text).map(KeyCode::Named)
    }
}

impl fmt::Display for Key {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.ctrl {
            formatter.write_str("ctrl+")?;
        }
        if self.alt {
            formatter.write_str("alt+")?;
        }
        match self.code {
            // An inline hint showing a literal blank would be invisible, so the
            // spacebar is spelled with the same name that configures it.
            KeyCode::Character(' ') => formatter.write_str("space"),
            KeyCode::Character(character) => write!(formatter, "{character}"),
            KeyCode::Named(Named::BackTab) => formatter.write_str("shift+tab"),
            KeyCode::Named(Named::Function(number)) => write!(formatter, "f{number}"),
            KeyCode::Named(named) => formatter.write_str(
                NAMED_KEYS
                    .iter()
                    .find(|(_, candidate)| *candidate == named)
                    .map(|(name, _)| *name)
                    .unwrap_or_default(),
            ),
        }
    }
}

/// A configured key string Virtui cannot represent.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InvalidKey {
    pub input: String,
}

impl InvalidKey {
    fn new(input: impl Into<String>) -> Self {
        Self {
            input: input.into(),
        }
    }
}
