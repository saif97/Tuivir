use std::{future::Future, io, pin::Pin, process::Stdio};

#[derive(Clone, Debug, Eq, PartialEq)]
/// A program and explicit argument list, never a shell command string.
pub struct CommandSpec {
    pub program: String,
    pub args: Vec<String>,
}

impl CommandSpec {
    pub fn new(program: &str, args: &[&str]) -> Self {
        Self {
            program: program.to_owned(),
            args: args.iter().map(|arg| (*arg).to_owned()).collect(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// The observable result of a short-lived provider CLI process.
pub struct CliOutput {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CliError {
    NotFound,
    Failed(String),
}

/// System-boundary abstraction for running provider CLI processes.
///
/// Production uses [`TokioCliRunner`]; tests provide recorded fixtures without
/// requiring a live provider daemon.
pub trait CliRunner: Send + Sync {
    fn run<'a>(
        &'a self,
        command: CommandSpec,
    ) -> Pin<Box<dyn Future<Output = Result<CliOutput, CliError>> + Send + 'a>>;
}

pub struct TokioCliRunner;

impl CliRunner for TokioCliRunner {
    fn run<'a>(
        &'a self,
        command: CommandSpec,
    ) -> Pin<Box<dyn Future<Output = Result<CliOutput, CliError>> + Send + 'a>> {
        Box::pin(async move {
            let output = tokio::process::Command::new(&command.program)
                .args(&command.args)
                .stdin(Stdio::null())
                .output()
                .await
                .map_err(|error| match error.kind() {
                    io::ErrorKind::NotFound => CliError::NotFound,
                    _ => CliError::Failed(error.to_string()),
                })?;

            Ok(CliOutput {
                success: output.status.success(),
                stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            })
        })
    }
}
