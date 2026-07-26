use crate::config::ConfigError;
use crate::keys::Key;
use crate::provider::ResourceCommand;

/// One registered user intention.
///
/// A Command is what the user meant, not what happened: facts and asynchronous
/// completions stay in [`crate::app::AppEvent`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Command {
    Quit,
    ToggleHelp,
    /// Refreshes the Active Workspace now rather than waiting for the clock.
    Refresh,
    FocusProviders,
    FocusResources,
    /// Moves the selection in whichever panel has focus.
    SelectNext,
    SelectPrevious,
    /// Moves the resource selection by a five-item delta, clamped at the ends.
    SelectNextFast,
    SelectPreviousFast,
    NextWorkspace,
    PreviousWorkspace,
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
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandScope {
    /// The provider selector has focus.
    ProviderSelector,
    /// The Provider Workspace's resource view has focus.
    ResourceView,
    /// A Resource Command is waiting to be confirmed.
    Confirmation,
    /// A failed Resource Command is being reported.
    CommandFailure,
    /// The contextual help overlay is open.
    HelpOverlay,
}

impl CommandScope {
    /// Every structural scope.
    ///
    /// Conflict detection asks whether two Commands are ever reachable at once
    /// rather than assuming it, so a scope missing here would hide a conflict.
    /// [`Self::listed`] fails to compile until a new variant is added.
    pub const ALL: &'static [Self] = &[
        Self::ProviderSelector,
        Self::ResourceView,
        Self::Confirmation,
        Self::CommandFailure,
        Self::HelpOverlay,
    ];

    /// Exists only so that adding a variant forces [`Self::ALL`] to be revisited.
    #[allow(dead_code)]
    fn listed(self) -> bool {
        let named = match self {
            Self::ProviderSelector => Self::ProviderSelector,
            Self::ResourceView => Self::ResourceView,
            Self::Confirmation => Self::Confirmation,
            Self::CommandFailure => Self::CommandFailure,
            Self::HelpOverlay => Self::HelpOverlay,
        };
        Self::ALL.contains(&named)
    }
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
        Self {
            commands: BUILTIN_COMMANDS
                .iter()
                .map(|definition| EffectiveCommand {
                    id: definition.id,
                    description: definition.description,
                    command: definition.command,
                    scopes: definition.scopes,
                    keys: definition
                        .default_keys
                        .iter()
                        .map(|text| {
                            Key::parse(text).expect("compiled default keys are representable")
                        })
                        .collect(),
                })
                .collect(),
        }
    }

    /// Layers a `[keybindings]` table over the compiled defaults.
    ///
    /// The table is a partial override: mentioning a Command replaces its
    /// complete key list, omitting it preserves the defaults, and an empty list
    /// leaves it unbound. Nothing is applied unless everything validates.
    pub fn effective(overrides: &[(String, Vec<String>)]) -> Result<Self, Vec<ConfigError>> {
        let mut registry = Self::builtin();
        let mut errors = Vec::new();

        for (id, keys) in overrides {
            let parsed = keys
                .iter()
                .filter_map(|text| match Key::parse(text) {
                    Ok(key) => Some(key),
                    Err(invalid) => {
                        errors.push(ConfigError::InvalidKey {
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
                None => errors.push(ConfigError::UnknownCommand { id: id.clone() }),
            }
        }

        errors.extend(registry.reserve_emergency_quit());
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
    fn reserve_emergency_quit(&mut self) -> Vec<ConfigError> {
        let emergency = Self::emergency_quit_key();
        let stolen = self
            .commands
            .iter()
            .filter(|command| command.command != Command::Quit)
            .filter(|command| command.keys.contains(&emergency))
            .map(|command| ConfigError::ReservedKey {
                id: command.id.to_owned(),
                key: emergency.to_string(),
            })
            .collect();

        if let Some(quit) = self
            .commands
            .iter_mut()
            .find(|command| command.command == Command::Quit)
            && !quit.keys.contains(&emergency)
        {
            quit.keys.push(emergency);
        }
        stolen
    }

    /// Reports every key claimed by two Commands that share a scope.
    ///
    /// Commands whose scopes cannot overlap are never reachable together, so
    /// they may share a key freely; those that can overlap would otherwise be
    /// resolved by registration order, which is a priority the user cannot see.
    fn conflicting_keys(&self) -> Vec<ConfigError> {
        let mut conflicts: Vec<ConfigError> = Vec::new();
        for scope in CommandScope::ALL {
            let mut claimed: Vec<(Key, &str)> = Vec::new();
            for command in self.in_scope(*scope) {
                for key in &command.keys {
                    // Taking the emergency Quit is already reported against the
                    // Command that took it, which says more than a conflict
                    // with the Quit it can never win against.
                    if *key == Self::emergency_quit_key() {
                        continue;
                    }
                    let Some((_, first)) = claimed.iter().find(|(claimed, _)| claimed == key)
                    else {
                        claimed.push((*key, command.id));
                        continue;
                    };
                    // One Command listing a key twice is a duplicate, not a
                    // conflict, and is reported as such.
                    if *first == command.id {
                        continue;
                    }
                    let conflict = ConfigError::ConflictingKey {
                        key: key.to_string(),
                        first: (*first).to_owned(),
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
        self.commands
            .iter()
            .filter(move |command| command.scopes.contains(&scope) && !command.keys.is_empty())
    }

    /// The preferred inline hint for `command`, or `None` when it is unbound.
    pub fn first_key(&self, command: Command) -> Option<Key> {
        self.commands
            .iter()
            .find(|registered| registered.command == command)
            .and_then(|registered| registered.keys.first().copied())
    }
}

/// Reports each key one Command lists more than once, naming it once however
/// many times it was repeated.
///
/// Keys are compared after parsing, so two spellings of one key — `space` and a
/// literal blank — are the duplicate they actually are.
fn duplicate_keys(id: &str, keys: &[Key]) -> Vec<ConfigError> {
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
        .map(|key| ConfigError::DuplicateKey {
            id: id.to_owned(),
            key: key.to_string(),
        })
        .collect()
}

struct CommandDefinition {
    id: &'static str,
    description: &'static str,
    command: Command,
    scopes: &'static [CommandScope],
    default_keys: &'static [&'static str],
}

/// Every scope in which the user is working inside a Provider Workspace rather
/// than answering a modal.
const WORKSPACE: &[CommandScope] = &[CommandScope::ProviderSelector, CommandScope::ResourceView];
const RESOURCE_VIEW: &[CommandScope] = &[CommandScope::ResourceView];
/// Every modal scope. A modal replaces the workspace scope while it is open.
const MODAL: &[CommandScope] = &[
    CommandScope::Confirmation,
    CommandScope::CommandFailure,
    CommandScope::HelpOverlay,
];

/// Defaults follow lazydocker wherever an equivalent Command exists.
const BUILTIN_COMMANDS: &[CommandDefinition] = &[
    CommandDefinition {
        id: "app.quit",
        description: "Quit",
        command: Command::Quit,
        scopes: WORKSPACE,
        default_keys: &["q"],
    },
    CommandDefinition {
        id: "app.help",
        description: "Help",
        command: Command::ToggleHelp,
        scopes: &[
            CommandScope::ProviderSelector,
            CommandScope::ResourceView,
            CommandScope::HelpOverlay,
        ],
        default_keys: &["?"],
    },
    CommandDefinition {
        id: "app.refresh",
        // Plain `r` stays lazydocker's Restart in a resource view.
        description: "Refresh",
        command: Command::Refresh,
        scopes: WORKSPACE,
        default_keys: &["ctrl+r"],
    },
    CommandDefinition {
        id: "focus.providers",
        description: "Focus providers",
        command: Command::FocusProviders,
        scopes: WORKSPACE,
        default_keys: &["1"],
    },
    CommandDefinition {
        id: "focus.resources",
        description: "Focus resources",
        command: Command::FocusResources,
        scopes: WORKSPACE,
        default_keys: &["2"],
    },
    CommandDefinition {
        id: "selection.next",
        description: "Select next",
        command: Command::SelectNext,
        scopes: WORKSPACE,
        default_keys: &["j", "down"],
    },
    CommandDefinition {
        id: "selection.previous",
        description: "Select previous",
        command: Command::SelectPrevious,
        scopes: WORKSPACE,
        default_keys: &["k", "up"],
    },
    CommandDefinition {
        id: "selection.next.fast",
        description: "Select five ahead",
        command: Command::SelectNextFast,
        scopes: RESOURCE_VIEW,
        default_keys: &["J"],
    },
    CommandDefinition {
        id: "selection.previous.fast",
        description: "Select five back",
        command: Command::SelectPreviousFast,
        scopes: RESOURCE_VIEW,
        default_keys: &["K"],
    },
    CommandDefinition {
        id: "workspace.next",
        description: "Next workspace",
        command: Command::NextWorkspace,
        scopes: WORKSPACE,
        default_keys: &["]"],
    },
    CommandDefinition {
        id: "workspace.previous",
        description: "Previous workspace",
        command: Command::PreviousWorkspace,
        scopes: WORKSPACE,
        default_keys: &["["],
    },
    CommandDefinition {
        id: "modal.confirm",
        description: "Confirm",
        command: Command::Confirm,
        scopes: MODAL,
        default_keys: &["y", "enter"],
    },
    CommandDefinition {
        id: "modal.cancel",
        // `Esc` leads, so it is the hint a modal shows for backing out.
        description: "Cancel",
        command: Command::Cancel,
        scopes: MODAL,
        default_keys: &["esc", "n"],
    },
    CommandDefinition {
        id: "resource.start",
        description: "Start",
        command: Command::Resource(ResourceCommand::Start),
        scopes: &[CommandScope::ResourceView],
        default_keys: &["S"],
    },
    CommandDefinition {
        id: "resource.stop",
        description: "Stop",
        command: Command::Resource(ResourceCommand::Stop),
        scopes: &[CommandScope::ResourceView],
        default_keys: &["s"],
    },
    CommandDefinition {
        id: "resource.restart",
        description: "Restart",
        command: Command::Resource(ResourceCommand::Restart),
        scopes: &[CommandScope::ResourceView],
        default_keys: &["r"],
    },
    CommandDefinition {
        id: "resource.resume",
        description: "Resume",
        command: Command::Resource(ResourceCommand::Resume),
        scopes: &[CommandScope::ResourceView],
        default_keys: &["p"],
    },
    CommandDefinition {
        id: "resource.delete",
        description: "Delete",
        command: Command::Resource(ResourceCommand::Delete),
        scopes: &[CommandScope::ResourceView],
        default_keys: &["d"],
    },
];
