use std::fmt;

use super::Key;

/// One reason application keybinding policy refused a set of overrides.
///
/// Validation is atomic: every discoverable diagnostic is collected and no
/// part of an invalid override set is applied.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum KeybindingError {
    /// The Shell Prefix is not a key Tuivir recognizes.
    InvalidShellPrefix { key: String },
    /// A Resource Shell Session control names an unrecognized key.
    InvalidShellKey { id: String, key: String },
    /// A Resource Shell Session control is not one Tuivir recognizes.
    UnknownShellKeybinding { id: String },
    /// The focus control must always let the user return to Tuivir.
    EmptyShellKeybinding { id: String },
    /// A Resource Shell Session control lists one key more than once.
    DuplicateShellKey { id: String, key: String },
    /// Two Resource Shell Session controls claim the same key.
    ConflictingShellKey {
        key: String,
        first: String,
        second: String,
    },
    /// A Shell Prefix would make a Resource Shell Session control unreachable.
    ShellPrefixCollision { id: String, key: String },
    /// A keybinding names a Command Tuivir does not register.
    UnknownCommand { id: String },
    /// A key string Tuivir cannot represent.
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

pub(crate) fn duplicate_keys(keys: &[Key]) -> Vec<Key> {
    let mut seen = Vec::new();
    let mut duplicates = Vec::new();
    for key in keys {
        if seen.contains(key) {
            if !duplicates.contains(key) {
                duplicates.push(*key);
            }
        } else {
            seen.push(*key);
        }
    }
    duplicates
}

impl fmt::Display for KeybindingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidShellPrefix { key } => {
                write!(
                    formatter,
                    "[resource_shell] has an unrecognised Shell Prefix \"{key}\""
                )
            }
            Self::InvalidShellKey { id, key } => write!(
                formatter,
                "[resource_shell.keybindings] \"{id}\" has an unrecognised key \"{key}\""
            ),
            Self::UnknownShellKeybinding { id } => write!(
                formatter,
                "[resource_shell.keybindings] has no Resource Shell control named \"{id}\""
            ),
            Self::EmptyShellKeybinding { id } => write!(
                formatter,
                "[resource_shell.keybindings] \"{id}\" must bind at least one key"
            ),
            Self::DuplicateShellKey { id, key } => write!(
                formatter,
                "[resource_shell.keybindings] \"{id}\" lists \"{key}\" more than once"
            ),
            Self::ConflictingShellKey { key, first, second } => write!(
                formatter,
                "[resource_shell.keybindings] \"{key}\" is bound to both \"{first}\" and \"{second}\""
            ),
            Self::ShellPrefixCollision { id, key } => write!(
                formatter,
                "[resource_shell] Shell Prefix \"{key}\" collides with \"{id}\""
            ),
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
                "[keybindings] \"{id}\" cannot claim \"{key}\": it always quits Tuivir"
            ),
        }
    }
}
