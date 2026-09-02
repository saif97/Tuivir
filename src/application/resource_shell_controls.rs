use super::{Key, KeybindingError, keybinding::duplicate_keys};

/// Effective terminal controls owned by Tuivir while a Resource Shell Session
/// has keyboard focus.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResourceShellControls {
    prefix: Key,
    focus_tuivir: Vec<Key>,
    toggle_zoom: Vec<Key>,
}

impl Default for ResourceShellControls {
    fn default() -> Self {
        Self::new(
            Key::character(']').with_ctrl(),
            vec![Key::character('q')],
            vec![Key::character('z')],
        )
    }
}

impl ResourceShellControls {
    pub fn new(prefix: Key, focus_tuivir: Vec<Key>, toggle_zoom: Vec<Key>) -> Self {
        Self {
            prefix,
            focus_tuivir,
            toggle_zoom,
        }
    }

    pub fn prefix(&self) -> Key {
        self.prefix
    }

    pub fn focus_tuivir(&self) -> &[Key] {
        &self.focus_tuivir
    }

    pub fn toggle_zoom(&self) -> &[Key] {
        &self.toggle_zoom
    }

    /// Applies optional configuration over the Shell Prefix defaults and
    /// reports every validation failure without partially changing controls.
    pub fn effective(
        prefix: Option<String>,
        keybindings: Vec<(String, Vec<String>)>,
    ) -> Result<Self, Vec<KeybindingError>> {
        let mut controls = Self::default();
        let mut errors = Vec::new();

        if let Some(prefix) = prefix {
            match Key::parse(&prefix) {
                Ok(key) => controls.prefix = key,
                Err(_) => errors.push(KeybindingError::InvalidShellPrefix { key: prefix }),
            }
        }

        for (id, keys) in keybindings {
            let parsed = keys
                .iter()
                .filter_map(|text| match Key::parse(text) {
                    Ok(key) => Some(key),
                    Err(_) => {
                        errors.push(KeybindingError::InvalidShellKey {
                            id: id.clone(),
                            key: text.clone(),
                        });
                        None
                    }
                })
                .collect::<Vec<_>>();
            errors.extend(duplicate_keys(&parsed).into_iter().map(|key| {
                KeybindingError::DuplicateShellKey {
                    id: id.clone(),
                    key: key.to_string(),
                }
            }));
            match id.as_str() {
                "focus_tuivir" => {
                    if parsed.is_empty() {
                        errors.push(KeybindingError::EmptyShellKeybinding { id });
                    } else {
                        controls.focus_tuivir = parsed;
                    }
                }
                "toggle_zoom" => controls.toggle_zoom = parsed,
                _ => errors.push(KeybindingError::UnknownShellKeybinding { id }),
            }
        }

        for (id, keys) in [
            ("focus_tuivir", controls.focus_tuivir()),
            ("toggle_zoom", controls.toggle_zoom()),
        ] {
            if keys.contains(&controls.prefix) {
                errors.push(KeybindingError::ShellPrefixCollision {
                    id: id.to_owned(),
                    key: controls.prefix.to_string(),
                });
            }
        }
        for key in controls.focus_tuivir() {
            if controls.toggle_zoom().contains(key) {
                errors.push(KeybindingError::ConflictingShellKey {
                    key: key.to_string(),
                    first: "focus_tuivir".to_owned(),
                    second: "toggle_zoom".to_owned(),
                });
            }
        }

        if errors.is_empty() {
            Ok(controls)
        } else {
            Err(errors)
        }
    }
}
