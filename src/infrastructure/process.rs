use std::{future::Future, io, pin::Pin, process::Stdio};

#[derive(Clone, Debug, Eq, PartialEq)]
/// A program and explicit argument list, never a shell command string.
pub struct ProcessSpec {
    pub program: String,
    pub args: Vec<String>,
}

impl ProcessSpec {
    pub fn new(program: &str, args: &[&str]) -> Self {
        Self {
            program: program.to_owned(),
            args: args.iter().map(|arg| (*arg).to_owned()).collect(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// Captured output of a process that exited with status 0.
///
/// Both streams are preserved verbatim; callers decide what to trim and parse.
pub struct ProcessOutput {
    pub stdout: String,
    pub stderr: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// A process that ran to completion and reported failure.
pub struct ProcessFailure {
    /// The exit code the process reported, or `None` when a signal ended it.
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl ProcessFailure {
    /// The most specific text the process left behind: trimmed stderr, then
    /// trimmed stdout, then the caller's own fallback prose.
    ///
    /// The fallback carries the caller's meaning; this only decides which of
    /// the process's own streams, if any, said anything at all.
    pub fn message_or(&self, fallback: &str) -> String {
        let stderr = self.stderr.trim();
        if !stderr.is_empty() {
            return stderr.to_owned();
        }
        let stdout = self.stdout.trim();
        if !stdout.is_empty() {
            return stdout.to_owned();
        }
        fallback.to_owned()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// Why a process produced no successful output.
pub enum ProcessError {
    /// The program is not installed, or is not on `PATH`.
    ExecutableNotFound,
    /// The process could not be started for any other reason.
    SpawnFailed(String),
    /// The process started and exited with a non-zero status.
    Exited(ProcessFailure),
}

impl ProcessError {}

/// System-boundary abstraction for running short-lived provider CLI processes.
///
/// Production uses [`TokioCliRunner`]; tests provide recorded fixtures without
/// requiring a live provider daemon. Interactive shell and PTY execution is out
/// of scope: every process here runs to completion and is captured.
pub trait CliRunner: Send + Sync {
    /// Runs a process to completion, succeeding only on exit status 0.
    fn run<'a>(
        &'a self,
        process: ProcessSpec,
    ) -> Pin<Box<dyn Future<Output = Result<ProcessOutput, ProcessError>> + Send + 'a>>;
}

pub struct TokioCliRunner;

impl CliRunner for TokioCliRunner {
    fn run<'a>(
        &'a self,
        process: ProcessSpec,
    ) -> Pin<Box<dyn Future<Output = Result<ProcessOutput, ProcessError>> + Send + 'a>> {
        Box::pin(async move {
            let output = tokio::process::Command::new(&process.program)
                .args(&process.args)
                .stdin(Stdio::null())
                .output()
                .await
                .map_err(|error| match error.kind() {
                    io::ErrorKind::NotFound => ProcessError::ExecutableNotFound,
                    _ => ProcessError::SpawnFailed(error.to_string()),
                })?;

            let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
            let stderr = String::from_utf8_lossy(&output.stderr).into_owned();

            if output.status.success() {
                Ok(ProcessOutput { stdout, stderr })
            } else {
                Err(ProcessError::Exited(ProcessFailure {
                    exit_code: output.status.code(),
                    stdout,
                    stderr,
                }))
            }
        })
    }
}
