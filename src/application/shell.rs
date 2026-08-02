#[derive(Clone, Debug, Eq, PartialEq)]
/// The explicit Provider CLI program and arguments for one Interactive Shell.
///
/// This value describes the application request without exposing a process
/// runner or its failure types. The host hands it to an infrastructure adapter.
pub struct InteractiveShellProcess {
    program: String,
    args: Vec<String>,
}

impl InteractiveShellProcess {
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

#[derive(Clone, Debug, Eq, PartialEq)]
/// What the application needs to know after an Interactive Shell handover.
pub enum InteractiveShellOutcome {
    /// The shell started; its eventual exit status belongs to the user.
    Exited,
    /// The host could not start the Provider CLI process.
    StartFailed(String),
}
