//! Compatibility facade for infrastructure process execution.

pub use crate::infrastructure::process::{
    CliRunner, InteractiveRunner, ProcessError, ProcessFailure, ProcessOutput, ProcessSpec,
    TokioCliRunner,
};
