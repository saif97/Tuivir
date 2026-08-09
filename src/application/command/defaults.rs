//! Compiled Command registration policy.
//!
//! User TOML supplies partial keybinding overrides through infrastructure; it
//! cannot be the source of these definitions because Virtui must have its full
//! Command registry when no configuration file exists.

use super::{Command, CommandScope, ResourceCommand};

pub(super) struct CommandDefinition {
    pub(super) id: &'static str,
    pub(super) description: &'static str,
    pub(super) command: Command,
    pub(super) scopes: &'static [CommandScope],
    pub(super) default_keys: &'static [&'static str],
}

pub(super) struct ResourcePanelFocusDefinition {
    pub(super) id: &'static str,
    pub(super) description: &'static str,
    pub(super) default_key: &'static str,
}

pub(super) const RESOURCE_PANEL_FOCUS_COMMANDS: &[ResourcePanelFocusDefinition] = &[
    ResourcePanelFocusDefinition {
        id: "focus.resources",
        description: "Focus first Resource Panel",
        default_key: "2",
    },
    ResourcePanelFocusDefinition {
        id: "focus.resources.2",
        description: "Focus second Resource Panel",
        default_key: "3",
    },
    ResourcePanelFocusDefinition {
        id: "focus.resources.3",
        description: "Focus third Resource Panel",
        default_key: "4",
    },
    ResourcePanelFocusDefinition {
        id: "focus.resources.4",
        description: "Focus fourth Resource Panel",
        default_key: "5",
    },
    ResourcePanelFocusDefinition {
        id: "focus.resources.5",
        description: "Focus fifth Resource Panel",
        default_key: "6",
    },
    ResourcePanelFocusDefinition {
        id: "focus.resources.6",
        description: "Focus sixth Resource Panel",
        default_key: "7",
    },
    ResourcePanelFocusDefinition {
        id: "focus.resources.7",
        description: "Focus seventh Resource Panel",
        default_key: "8",
    },
    ResourcePanelFocusDefinition {
        id: "focus.resources.8",
        description: "Focus eighth Resource Panel",
        default_key: "9",
    },
    ResourcePanelFocusDefinition {
        id: "focus.resources.9",
        description: "Focus ninth Resource Panel",
        default_key: "0",
    },
];

/// A Provider Workspace may expose at most this many Resource Panels so every
/// one retains a single-key numbered focus Command and visible hint.
pub const NUMBERED_RESOURCE_PANEL_CAPACITY: usize = RESOURCE_PANEL_FOCUS_COMMANDS.len();

/// Every scope in which the user is working inside a Provider Workspace rather
/// than answering a modal.
pub(super) const WORKSPACE: &[CommandScope] = &[
    CommandScope::ProviderSelector,
    CommandScope::ResourceView,
    CommandScope::Details,
];
const SELECTABLE: &[CommandScope] = &[CommandScope::ProviderSelector, CommandScope::ResourceView];
const RESOURCE_VIEW: &[CommandScope] = &[CommandScope::ResourceView];
const DETAILS: &[CommandScope] = &[CommandScope::Details];
/// Every modal scope. A modal replaces the workspace scope while it is open.
const MODAL: &[CommandScope] = &[
    CommandScope::Confirmation,
    CommandScope::CommandFailure,
    CommandScope::HelpOverlay,
];

/// Defaults follow lazydocker wherever an equivalent Command exists.
pub(super) const BUILTIN_COMMANDS: &[CommandDefinition] = &[
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
            CommandScope::Details,
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
        id: "focus.details",
        description: "Focus Details",
        command: Command::FocusDetails,
        scopes: WORKSPACE,
        default_keys: &["enter"],
    },
    CommandDefinition {
        id: "focus.next",
        description: "Focus next Pane",
        command: Command::FocusNextPane,
        scopes: WORKSPACE,
        default_keys: &["tab"],
    },
    CommandDefinition {
        id: "focus.previous",
        description: "Focus previous Pane",
        command: Command::FocusPreviousPane,
        scopes: WORKSPACE,
        default_keys: &["shift+tab"],
    },
    CommandDefinition {
        id: "selection.next",
        description: "Select next",
        command: Command::SelectNext,
        scopes: SELECTABLE,
        default_keys: &["j", "down"],
    },
    CommandDefinition {
        id: "selection.previous",
        description: "Select previous",
        command: Command::SelectPrevious,
        scopes: SELECTABLE,
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
    // lazydocker's own `[`/`]` already move the Active Workspace here, so the
    // detail views take the horizontal keys next to them instead.
    CommandDefinition {
        id: "detail.view.next",
        description: "Next detail view",
        command: Command::NextDetailView,
        scopes: DETAILS,
        default_keys: &["l", "right"],
    },
    CommandDefinition {
        id: "detail.view.previous",
        description: "Previous detail view",
        command: Command::PreviousDetailView,
        scopes: DETAILS,
        default_keys: &["h", "left"],
    },
    CommandDefinition {
        id: "detail.scroll.down",
        description: "Scroll details down",
        command: Command::ScrollDetailsDown,
        scopes: DETAILS,
        default_keys: &["ctrl+d", "pagedown"],
    },
    CommandDefinition {
        id: "detail.scroll.up",
        description: "Scroll details up",
        command: Command::ScrollDetailsUp,
        scopes: DETAILS,
        default_keys: &["ctrl+u", "pageup"],
    },
    CommandDefinition {
        id: "details.copy",
        description: "Copy selected details",
        command: Command::CopyDetails,
        scopes: DETAILS,
        default_keys: &["y"],
    },
    // The Pane Boundary is the shell's, not one Pane's, so it answers in every
    // workspace scope. `<` and `>` point the way the boundary travels.
    CommandDefinition {
        id: "layout.boundary.left",
        description: "Widen Details Pane",
        command: Command::MovePaneBoundaryLeft,
        scopes: WORKSPACE,
        default_keys: &["<"],
    },
    CommandDefinition {
        id: "layout.boundary.right",
        description: "Widen Resource Panels",
        command: Command::MovePaneBoundaryRight,
        scopes: WORKSPACE,
        default_keys: &[">"],
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
        id: "resource.shell",
        description: "Shell",
        command: Command::OpenShell,
        scopes: RESOURCE_VIEW,
        default_keys: &["E"],
    },
    CommandDefinition {
        id: "resource.delete",
        description: "Delete",
        command: Command::Resource(ResourceCommand::Delete),
        scopes: &[CommandScope::ResourceView],
        default_keys: &["d"],
    },
];
