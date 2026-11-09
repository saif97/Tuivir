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
