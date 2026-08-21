mod app;
mod command;
mod key;
mod keybinding;
mod pane_boundary;
mod provider;
mod shell;
mod workspace;

pub use app::{
    App, AppEvent, AppState, FocusedPane, HelpEntry, HelpOverlay, KeyHints, PendingShell,
    ResourceCommandInvocation, RunningResourceCommand,
};
pub use command::{
    Command, CommandRegistry, CommandScope, EffectiveCommand, NUMBERED_RESOURCE_PANEL_CAPACITY,
    ResourceCommand,
};
pub use key::{InvalidKey, Key, Named};
pub use keybinding::KeybindingError;
pub use pane_boundary::PaneBoundary;
pub use provider::{
    DetailView, LifecycleCommandPolicy, ProviderRequest, ProviderRequestId, Resource,
    ResourceDetails, ResourcePanel, WorkspaceError, WorkspaceSnapshot, lifecycle_commands,
};
pub use shell::{
    InteractiveShellOutcome, InteractiveShellProcess, ResourceShellEffect, ResourceShellSession,
    ResourceShellSessionId, ResourceShellSessionLifecycle,
};
pub use workspace::{
    DetailCompletion, DetailContent, DetailLoad, DetailPosition, DetailSelection,
    ProviderWorkspaceState, ResourceDetailsView, ResourcePanelView, WorkspaceLoadState,
    WorkspacePresentation, WorkspaceView,
};
