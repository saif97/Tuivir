use std::fmt;

/// One reason Virtui refused a configuration file.
///
/// Validation is atomic: every discoverable diagnostic is collected and
/// reported, and no part of an invalid configuration is applied.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConfigError {
    /// A `[keybindings]` entry names a Command Virtui does not register.
    UnknownCommand { id: String },
    /// A key string Virtui cannot represent.
    InvalidKey { id: String, key: String },
    /// A Command other than Quit tried to claim the emergency `ctrl+c`.
    ReservedKey { id: String, key: String },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownCommand { id } => {
                write!(formatter, "[keybindings] has no Command named \"{id}\"")
            }
            Self::InvalidKey { id, key } => write!(
                formatter,
                "[keybindings] \"{id}\" has an unrecognised key \"{key}\""
            ),
            Self::ReservedKey { id, key } => write!(
                formatter,
                "[keybindings] \"{id}\" cannot claim \"{key}\": it always quits Virtui"
            ),
        }
    }
}
