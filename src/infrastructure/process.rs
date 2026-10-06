use std::{future::Future, io, pin::Pin, process::Stdio, time::Duration};

#[derive(Clone, Debug, Eq, PartialEq)]
/// A program and explicit argument list, never a shell command string.
pub struct ProcessSpec {
    pub program: String,
    pub args: Vec<String>,
    background: bool,
}

impl ProcessSpec {
    pub fn new(program: &str, args: &[&str]) -> Self {
        Self {
            program: program.to_owned(),
            args: args.iter().map(|arg| (*arg).to_owned()).collect(),
            background: false,
        }
    }

    /// Launches a Provider process that owns its Resource's lifetime. It gets
    /// its own session and no terminal input, so it can outlive Tuivir.
    /// Waiting for such a process to exit would leave Start or Resume pending
    /// until the Resource shuts down.
    /// Immediate failures are captured during a one-second startup window;
    /// later output goes to a private log in Tuivir's state directory.
    pub fn background(program: &str, args: &[&str]) -> Self {
        Self {
            background: true,
            ..Self::new(program, args)
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

/// System-boundary abstraction for running provider CLI processes.
///
/// Production uses [`TokioCliRunner`]; tests provide recorded fixtures without
/// requiring a live provider daemon. Background launches return after their
/// startup window. Interactive shell and PTY execution is out of scope.
pub trait CliRunner: Send + Sync {
    /// Captures a completed process, or launches a background process and
    /// checks for immediate failure without waiting for its entire lifetime.
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
            if process.background {
                return launch_background(process).await;
            }
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

fn spawn_error(error: io::Error) -> ProcessError {
    match error.kind() {
        io::ErrorKind::NotFound => ProcessError::ExecutableNotFound,
        _ => ProcessError::SpawnFailed(error.to_string()),
    }
}

async fn launch_background(process: ProcessSpec) -> Result<ProcessOutput, ProcessError> {
    use std::os::unix::fs::OpenOptionsExt;
    use std::{
        fs::OpenOptions,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    let state = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))
        .ok_or_else(|| {
            ProcessError::SpawnFailed(
                "no home or XDG state directory for Provider launch logs".into(),
            )
        })?;
    let directory = state.join("tuivir/provider-processes");
    std::fs::create_dir_all(&directory)
        .map_err(|error| ProcessError::SpawnFailed(error.to_string()))?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let path = directory.join(format!("launch-{}-{timestamp}.log", std::process::id()));
    let log = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .map_err(|error| ProcessError::SpawnFailed(error.to_string()))?;
    let stderr = log.try_clone().map_err(|error| {
        let _ = std::fs::remove_file(&path);
        ProcessError::SpawnFailed(error.to_string())
    })?;
    let mut command = tokio::process::Command::new(&process.program);
    command
        .args(&process.args)
        .stdin(Stdio::null())
        .stdout(log)
        .stderr(stderr);
    // No shell is involved. A new session detaches the VM-owning process from
    // Tuivir's controlling terminal and its terminal-generated signals.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command.spawn().map_err(|error| {
        let _ = std::fs::remove_file(&path);
        spawn_error(error)
    })?;
    match tokio::time::timeout(Duration::from_secs(1), child.wait()).await {
        Ok(result) => {
            let output = std::fs::read_to_string(&path).unwrap_or_default();
            let _ = std::fs::remove_file(&path);
            let status = result.map_err(spawn_error)?;
            if status.success() {
                Ok(ProcessOutput {
                    stdout: output,
                    stderr: String::new(),
                })
            } else {
                Err(ProcessError::Exited(ProcessFailure {
                    exit_code: status.code(),
                    stdout: String::new(),
                    stderr: output,
                }))
            }
        }
        Err(_) => {
            // Reap while Tuivir is alive. Dropping this task on Tuivir exit
            // leaves the independent Provider process running, with its log.
            tokio::spawn(async move {
                if child.wait().await.is_ok_and(|status| status.success()) {
                    let _ = std::fs::remove_file(path);
                }
            });
            Ok(ProcessOutput {
                stdout: String::new(),
                stderr: String::new(),
            })
        }
    }
}
