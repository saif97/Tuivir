use crate::{
    application::WorkspaceError,
    infrastructure::process::{CliRunner, ProcessSpec},
};

pub(super) const DISCOVER: &str = "if command -v getent >/dev/null 2>&1; then exec getent passwd; else printf 'TUIVIR_GETENT_MISSING\\n'; fi";

pub(super) async fn discover(
    cli: &dyn CliRunner,
    process: ProcessSpec,
) -> Result<Option<Vec<String>>, WorkspaceError> {
    let output = cli.run(process).await.map_err(|error| {
        WorkspaceError::new(super::provider_cli_error(
            "Provider",
            &error,
            "Account discovery failed",
        ))
    })?;
    if output.stdout.trim() == "TUIVIR_GETENT_MISSING" {
        return Ok(None);
    }
    let mut accounts: Vec<String> = output
        .stdout
        .lines()
        .filter_map(|line| {
            let fields: Vec<_> = line.split(':').collect();
            (fields.len() >= 7 && !fields[0].is_empty()).then(|| fields[0].to_owned())
        })
        .collect();
    accounts.sort();
    accounts.dedup();
    Ok(Some(accounts))
}

pub(super) const VERIFY: &str = "printf TUIVIR_SHELL_VERIFIED";
pub(super) const READY: &str = "tuivir-shell-ready";
pub(super) const START_SH: &str = "printf '\\033]2;tuivir-shell-ready\\007'; exec /bin/sh -i";

pub(super) async fn verify(
    cli: &dyn CliRunner,
    process: ProcessSpec,
) -> Result<(), WorkspaceError> {
    let output = tokio::time::timeout(std::time::Duration::from_secs(10), cli.run(process))
        .await
        .map_err(|_| WorkspaceError::new("Shell account verification timed out"))?
        .map_err(|error| {
            WorkspaceError::new(super::provider_cli_error(
                "Provider",
                &error,
                "Shell account verification failed",
            ))
        })?;
    if !output.stdout.contains("TUIVIR_SHELL_VERIFIED") {
        return Err(WorkspaceError::new(if output.stderr.trim().is_empty() {
            "Provider did not confirm execution as the selected account".to_owned()
        } else {
            output.stderr.trim().to_owned()
        }));
    }
    Ok(())
}

pub(super) const START_BASH: &str = "printf '\\033]2;tuivir-shell-ready\\007'; exec bash -i";
pub(super) const START_LOGIN: &str = "printf '\\033]2;tuivir-shell-ready\\007'; exec \"$SHELL\" -i";
