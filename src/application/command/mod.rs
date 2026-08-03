//! Command registration, configuration policy, dispatch, and display metadata.

use std::{collections::HashMap, fmt};

mod defaults;

pub use defaults::NUMBERED_RESOURCE_PANEL_CAPACITY;
use defaults::{BUILTIN_COMMANDS, CommandDefinition, RESOURCE_PANEL_FOCUS_COMMANDS, WORKSPACE};

use super::{Key, KeybindingError};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResourceCommand {
    Start,
    Stop,
    Restart,
    /// Returns a suspended Resource to running — Docker `unpause`, Incus
    /// `unfreeze`.
    Resume,
    Delete,
}

impl fmt::Display for ResourceCommand {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Start => "start",
            Self::Stop => "stop",
            Self::Restart => "restart",
            Self::Resume => "resume",
            Self::Delete => "delete",
        };
        formatter.write_str(name)
    }
}

/// One registered user intention.
///
/// A Command is what the user meant, not what happened: facts and asynchronous
/// completions stay in [`crate::application::AppEvent`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Command {
    Quit,
    ToggleHelp,
    /// Refreshes the Active Workspace now rather than waiting for the clock.
    Refresh,
    FocusProviders,
    /// Focuses a Resource Panel by its zero-based provider-defined position.
    FocusResourcePanel(usize),
    FocusDetails,
    FocusNextPane,
    FocusPreviousPane,
    /// Moves the selection in whichever panel has focus.
    SelectNext,
    SelectPrevious,
    /// Moves the resource selection by a five-item delta, clamped at the ends.
    SelectNextFast,
    SelectPreviousFast,
    NextWorkspace,
    PreviousWorkspace,
    /// Moves through the provider-native detail views of the selected Resource.
    NextDetailView,
    PreviousDetailView,
    /// Moves through the output of the visible detail view.
    ScrollDetailsDown,
    ScrollDetailsUp,
    /// Hands the terminal to the Provider CLI for an Interactive Shell inside
    /// the selected Resource.
    OpenShell,
    /// Accepts the open modal.
    Confirm,
    /// Cancels or returns from the open modal.
    Cancel,
    Resource(ResourceCommand),
}

/// The structural part of the interface in which a Command may be invoked.
///
/// Scope is structural only. Mutable Resource State never changes it; an
/// unavailable Command is rejected when it is invoked, not hidden by scope.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum CommandScope {
    /// The provider selector has focus.
    ProviderSelector,
    /// The Provider Workspace's resource view has focus.
    ResourceView,
    /// One provider-ordered Resource Panel has focus.
    ResourcePanel(usize),
    /// The Details pane has focus.
    Details,
    /// A Resource Command is waiting to be confirmed.
    Confirmation,
    /// A failed Resource Command is being reported.
    CommandFailure,
    /// The contextual help overlay is open.
    HelpOverlay,
}

/// One Command with the keys that are actually bound to it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EffectiveCommand {
    /// The stable, lowercase, dotted configuration identifier.
    pub id: &'static str,
    pub description: &'static str,
    pub command: Command,
    pub scopes: &'static [CommandScope],
    /// The effective keys, in order. The first is the preferred inline hint;
    /// an empty list means the Command is unbound.
    pub keys: Vec<Key>,
}

/// Every registered Command with its effective Keybindings.
///
/// One registry drives dispatch, contextual help, and inline hints, so none of
/// the three can drift from the others.
#[derive(Clone, Debug)]
pub struct CommandRegistry {
    commands: Vec<EffectiveCommand>,
}

impl Default for CommandRegistry {
    fn default() -> Self {
        Self::builtin()
    }
}

impl CommandRegistry {
    /// The compiled defaults, before any configuration is layered over them.
    pub fn builtin() -> Self {
        let mut commands = BUILTIN_COMMANDS
            .iter()
            .map(effective_command)
            .collect::<Vec<_>>();
        let focus_position = commands
            .iter()
            .position(|registered| registered.command == Command::FocusProviders)
            .expect("the Provider selector focus Command is registered")
            + 1;
        commands.splice(
            focus_position..focus_position,
            RESOURCE_PANEL_FOCUS_COMMANDS
                .iter()
                .enumerate()
                .map(|(index, definition)| EffectiveCommand {
                    id: definition.id,
                    description: definition.description,
                    command: Command::FocusResourcePanel(index),
                    scopes: WORKSPACE,
                    keys: vec![
                        Key::parse(definition.default_key)
                            .expect("compiled Resource Panel focus keys are representable"),
                    ],
                }),
        );
        Self { commands }
    }

    /// Layers a `[keybindings]` table over the compiled defaults.
    ///
    /// The table is a partial override: mentioning a Command replaces its
    /// complete key list, omitting it preserves the defaults, and an empty list
    /// leaves it unbound. Nothing is applied unless everything validates.
    pub fn effective(overrides: &[(String, Vec<String>)]) -> Result<Self, Vec<KeybindingError>> {
        let mut registry = Self::builtin();
        let mut errors = Vec::new();

        for (id, keys) in overrides {
            let parsed = keys
                .iter()
                .filter_map(|text| match Key::parse(text) {
                    Ok(key) => Some(key),
                    Err(invalid) => {
                        errors.push(KeybindingError::InvalidKey {
                            id: id.clone(),
                            key: invalid.input,
                        });
                        None
                    }
                })
                .collect::<Vec<_>>();
            errors.extend(duplicate_keys(id, &parsed));
            match registry
                .commands
                .iter_mut()
                .find(|command| command.id == id)
            {
                Some(command) => command.keys = parsed,
                None => errors.push(KeybindingError::UnknownCommand { id: id.clone() }),
            }
        }

        registry.enforce_emergency_quit(&mut errors);
        errors.extend(registry.conflicting_keys());

        if errors.is_empty() {
            Ok(registry)
        } else {
            Err(errors)
        }
    }

    /// Keeps `ctrl+c` bound to Quit and to nothing else.
    ///
    /// A user who lists it explicitly keeps the position they chose; otherwise
    /// it is appended, so it is an invariant rather than a preferred hint.
    fn enforce_emergency_quit(&mut self, errors: &mut Vec<KeybindingError>) {
        let emergency = Self::emergency_quit_key();
        for command in &mut self.commands {
            if command.command == Command::Quit {
                if !command.keys.contains(&emergency) {
                    command.keys.push(emergency);
                }
            } else if command.keys.contains(&emergency) {
                errors.push(KeybindingError::ReservedKey {
                    id: command.id.to_owned(),
                    key: emergency.to_string(),
                });
            }
        }
    }

    /// Reports every key claimed by two Commands that share a scope.
    ///
    /// Commands whose scopes cannot overlap are never reachable together, so
    /// they may share a key freely; those that can overlap would otherwise be
    /// resolved by registration order, which is a priority the user cannot see.
    fn conflicting_keys(&self) -> Vec<KeybindingError> {
        // Every (scope, key) a Command has claimed, against the Command that
        // claimed it first. Scopes come from the Commands themselves, so a new
        // scope cannot be left out of the check by forgetting to list it.
        let mut claimed: HashMap<(CommandScope, Key), &'static str> = HashMap::new();
        let mut conflicts: Vec<KeybindingError> = Vec::new();

        for command in &self.commands {
            for scope in command.scopes {
                for key in &command.keys {
                    // Taking the emergency Quit is already reported against the
                    // Command that took it, which says more than a conflict
                    // with the Quit it can never win against.
                    if *key == Self::emergency_quit_key() {
                        continue;
                    }
                    // The first claimant keeps the slot: it is the Command a
                    // diagnostic names, and later claimants are the conflict.
                    let first = *claimed.entry((*scope, *key)).or_insert(command.id);
                    // Claiming a free key, and one Command listing a key twice,
                    // both land here. The repeat is a duplicate rather than a
                    // conflict, and is reported as one.
                    if first == command.id {
                        continue;
                    }
                    let conflict = KeybindingError::ConflictingKey {
                        key: key.to_string(),
                        first: first.to_owned(),
                        second: command.id.to_owned(),
                    };
                    // A pair sharing several scopes conflicts once.
                    if !conflicts.contains(&conflict) {
                        conflicts.push(conflict);
                    }
                }
            }
        }
        conflicts
    }

    fn emergency_quit_key() -> Key {
        Key::character('c').with_ctrl()
    }

    /// Resolves a key that is bound no matter the scope or the configuration.
    ///
    /// Only the emergency Quit is reserved, so the user can always restore
    /// their terminal.
    pub fn reserved(&self, key: Key) -> Option<Command> {
        (key == Self::emergency_quit_key()).then_some(Command::Quit)
    }

    /// Resolves a pressed key to the Command registered for `scope`.
    pub fn resolve(&self, scope: CommandScope, key: Key) -> Option<Command> {
        self.in_scope(scope)
            .find(|command| command.keys.contains(&key))
            .map(|command| command.command)
    }

    /// The bound Commands registered for `scope`, in registration order.
    ///
    /// Unbound Commands are omitted: they are not controls the user has.
    pub fn in_scope(&self, scope: CommandScope) -> impl Iterator<Item = &EffectiveCommand> {
        self.commands.iter().filter(move |command| {
            command.scopes.iter().any(|registered| {
                registered == &scope
                    || (*registered == CommandScope::ResourceView
                        && matches!(scope, CommandScope::ResourcePanel(_)))
            }) && !command.keys.is_empty()
        })
    }

    /// The preferred inline hint for `command`, or `None` when it is unbound.
    pub fn first_key(&self, command: Command) -> Option<Key> {
        self.commands
            .iter()
            .find(|registered| registered.command == command)
            .and_then(|registered| registered.keys.first().copied())
    }
}

fn effective_command(definition: &CommandDefinition) -> EffectiveCommand {
    EffectiveCommand {
        id: definition.id,
        description: definition.description,
        command: definition.command,
        scopes: definition.scopes,
        keys: definition
            .default_keys
            .iter()
            .map(|text| Key::parse(text).expect("compiled default keys are representable"))
            .collect(),
    }
}

/// Reports each key one Command lists more than once, naming it once however
/// many times it was repeated.
///
/// Keys are compared after parsing, so two spellings of one key — `space` and a
/// literal blank — are the duplicate they actually are.
fn duplicate_keys(id: &str, keys: &[Key]) -> Vec<KeybindingError> {
    let mut seen = Vec::new();
    let mut duplicated = Vec::new();
    for key in keys {
        if seen.contains(key) {
            if !duplicated.contains(key) {
                duplicated.push(*key);
            }
        } else {
            seen.push(*key);
        }
    }
    duplicated
        .into_iter()
        .map(|key| KeybindingError::DuplicateKey {
            id: id.to_owned(),
            key: key.to_string(),
        })
        .collect()
}
