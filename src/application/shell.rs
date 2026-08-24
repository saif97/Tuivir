use crate::domain::{ProviderId, ResourceTarget};

#[derive(Clone, Debug, Eq, PartialEq)]
/// The explicit Provider CLI program and arguments for one Resource Shell
/// Session.
///
/// This value describes the application request without exposing a process
/// runner or its failure types. The host hands it to an infrastructure adapter.
pub struct ResourceShellProcess {
    program: String,
    args: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
/// Stable identity allocated by application state for one Resource Shell
/// Session lifetime.
pub struct ResourceShellSessionId(pub(crate) u64);

impl ResourceShellSessionId {
    /// Reconstructs an application-allocated identity at a host seam.
    pub fn new(value: u64) -> Self {
        Self(value)
    }

    /// Numeric value used only to give the host's PTY a distinct identity.
    pub fn value(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// The user-visible lifecycle of a Resource Shell Session.
pub enum ResourceShellSessionLifecycle {
    Starting,
    Running,
    Exited,
    StartFailed(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// Application-owned identity and lifecycle for one Resource Shell Session.
///
/// Live child, PTY, I/O, and terminal-engine state intentionally do not cross
/// this boundary. The host registry owns them under `id`.
pub struct ResourceShellSession {
    pub id: ResourceShellSessionId,
    pub provider_id: ProviderId,
    pub target: ResourceTarget,
    pub lifecycle: ResourceShellSessionLifecycle,
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// Host work requested by the application for a Resource Shell Session.
pub enum ResourceShellEffect {
    Start {
        session: ResourceShellSession,
        process: ResourceShellProcess,
    },
    /// The session's Resource disappeared from an accepted Provider snapshot.
    /// The host must terminate and reap its private runtime before forgetting
    /// the matching application identity.
    Stop { session_id: ResourceShellSessionId },
}

impl ResourceShellProcess {
    pub fn new(program: &str, args: &[&str]) -> Self {
        Self {
            program: program.to_owned(),
            args: args.iter().map(|arg| (*arg).to_owned()).collect(),
        }
    }

    pub fn program(&self) -> &str {
        &self.program
    }

    pub fn args(&self) -> &[String] {
        &self.args
    }
}
