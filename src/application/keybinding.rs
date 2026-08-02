use std::fmt;

/// One reason application keybinding policy refused a set of overrides.
///
/// Validation is atomic: every discoverable diagnostic is collected and no
/// part of an invalid override set is applied.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum KeybindingError {
    /// A keybinding names a Command Virtui does not register.
    UnknownCommand { id: String },
    /// A key string Virtui cannot represent.
    InvalidKey { id: String, key: String },
    /// One Command lists the same key twice, however it was spelled.
    DuplicateKey { id: String, key: String },
    /// Two Commands that can be invoked at the same time claim one key.
    ///
    /// `first` is the Command registered earlier; neither is given priority.
    ConflictingKey {
        key: String,
        first: String,
        second: String,
    },
    /// A Command other than Quit tried to claim the emergency `ctrl+c`.
    ReservedKey { id: String, key: String },
}

impl fmt::Display for KeybindingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownCommand { id } => {
                write!(formatter, "[keybindings] has no Command named \"{id}\"")
            }
            Self::InvalidKey { id, key } => write!(
                formatter,
                "[keybindings] \"{id}\" has an unrecognised key \"{key}\""
            ),
            Self::DuplicateKey { id, key } => write!(
                formatter,
                "[keybindings] \"{id}\" lists \"{key}\" more than once"
            ),
            Self::ConflictingKey { key, first, second } => write!(
                formatter,
                "[keybindings] \"{key}\" is bound to both \"{first}\" and \"{second}\", \
                 which can be invoked at the same time"
            ),
            Self::ReservedKey { id, key } => write!(
                formatter,
                "[keybindings] \"{id}\" cannot claim \"{key}\": it always quits Virtui"
            ),
        }
    }
}
