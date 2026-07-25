use std::{future::Future, pin::Pin};

use serde::Deserialize;

use crate::{
    cli::{CliError, CliRunner, CommandSpec},
    provider::{
        ProviderDiscovery, ProviderId, ProviderWorkspace, Resource, ResourceCommand, ResourceId,
        ResourcePanel, WorkspaceError, WorkspaceSnapshot,
    },
};

const PROVIDER_ID: &str = "incus";
const PROVIDER_NAME: &str = "Incus";

pub struct IncusWorkspace;

#[derive(Deserialize)]
struct InstanceRow {
    name: String,
    status: String,
    #[serde(rename = "type")]
    instance_type: String,
    architecture: String,
    location: String,
}

impl ProviderWorkspace for IncusWorkspace {
    fn id(&self) -> ProviderId {
        ProviderId::new(PROVIDER_ID)
    }

    fn discover<'a>(
        &'a self,
        cli: &'a dyn CliRunner,
    ) -> Pin<Box<dyn Future<Output = Option<ProviderDiscovery>> + Send + 'a>> {
        Box::pin(async move {
            let remote = match cli
                .run(CommandSpec::new("incus", &["remote", "get-default"]))
                .await
            {
                Err(CliError::NotFound) => return None,
                Err(CliError::Failed(message)) => {
                    return Some(discovery_error(message, "incus remote get-default"));
                }
                Ok(output) if !output.success => {
                    return Some(discovery_error(
                        output.stderr.trim(),
                        "incus remote get-default",
                    ));
                }
                Ok(output) => output.stdout.trim().to_owned(),
            };
            let project = match cli
                .run(CommandSpec::new("incus", &["project", "get-current"]))
                .await
            {
                Err(CliError::NotFound) => {
                    return Some(discovery_error(
                        "Incus CLI is no longer available",
                        "incus project get-current",
                    ));
                }
                Err(CliError::Failed(message)) => {
                    return Some(discovery_error(message, "incus project get-current"));
                }
                Ok(output) if !output.success => {
                    return Some(discovery_error(
                        output.stderr.trim(),
                        "incus project get-current",
                    ));
                }
                Ok(output) => output.stdout.trim().to_owned(),
            };

            Some(ProviderDiscovery {
                id: self.id(),
                name: PROVIDER_NAME.to_owned(),
                target_environment: format!("{remote} / {project}"),
                error: None,
            })
        })
    }

    fn refresh<'a>(
        &'a self,
        cli: &'a dyn CliRunner,
    ) -> Pin<Box<dyn Future<Output = Result<WorkspaceSnapshot, WorkspaceError>> + Send + 'a>> {
        Box::pin(async move {
            let output = cli
                .run(CommandSpec::new("incus", &["list", "--format=json"]))
                .await
                .map_err(cli_error)?;
            if !output.success {
                return Err(refresh_error(output.stderr.trim()));
            }
            let rows: Vec<InstanceRow> = serde_json::from_str(&output.stdout)
                .map_err(|error| WorkspaceError::new(error.to_string()))?;
            let resources = rows
                .into_iter()
                .map(|row| {
                    let available_commands = incus_commands(&row.status);
                    Resource {
                        id: ResourceId::new(&row.name),
                        name: row.name,
                        status: Some(row.status),
                        fields: vec![
                            ("Type".to_owned(), row.instance_type),
                            ("Architecture".to_owned(), row.architecture),
                            ("Location".to_owned(), row.location),
                        ],
                        available_commands,
                    }
                })
                .collect();

            Ok(WorkspaceSnapshot {
                panels: vec![ResourcePanel {
                    title: "Instances".to_owned(),
                    resources,
                }],
            })
        })
    }

    fn execute_command<'a>(
        &'a self,
        cli: &'a dyn CliRunner,
        resource_id: &'a ResourceId,
        command: ResourceCommand,
    ) -> Pin<Box<dyn Future<Output = Result<(), WorkspaceError>> + Send + 'a>> {
        Box::pin(async move {
            let verb = match command {
                ResourceCommand::Start => "start",
                ResourceCommand::Stop => "stop",
                ResourceCommand::Restart => "restart",
                ResourceCommand::Delete => "delete",
            };
            let output = cli
                .run(CommandSpec::new("incus", &[verb, resource_id.0.as_str()]))
                .await
                .map_err(command_cli_error)?;
            if !output.success {
                let message = output.stderr.trim();
                return Err(WorkspaceError::new(if message.is_empty() {
                    "Incus command failed"
                } else {
                    message
                }));
            }
            Ok(())
        })
    }
}

fn incus_commands(status: &str) -> Vec<ResourceCommand> {
    if status.eq_ignore_ascii_case("running") {
        vec![
            ResourceCommand::Stop,
            ResourceCommand::Restart,
            ResourceCommand::Delete,
        ]
    } else {
        vec![ResourceCommand::Start, ResourceCommand::Delete]
    }
}

fn discovery_error(message: impl AsRef<str>, command: &str) -> ProviderDiscovery {
    ProviderDiscovery {
        id: ProviderId::new(PROVIDER_ID),
        name: PROVIDER_NAME.to_owned(),
        target_environment: "unavailable".to_owned(),
        error: Some(WorkspaceError::new(format!(
            "{}. Run `{command}` to verify the selected Target Environment.",
            message.as_ref(),
        ))),
    }
}

fn cli_error(error: CliError) -> WorkspaceError {
    match error {
        CliError::NotFound => WorkspaceError::new("Incus CLI is not available"),
        CliError::Failed(message) => WorkspaceError::new(message),
    }
}

fn refresh_error(message: impl AsRef<str>) -> WorkspaceError {
    WorkspaceError::new(format!(
        "{}. Run `incus list` to verify access to the current Target Environment.",
        message.as_ref()
    ))
}

fn command_cli_error(error: CliError) -> WorkspaceError {
    let message = match error {
        CliError::NotFound => "Incus CLI is no longer available".to_owned(),
        CliError::Failed(message) => message,
    };
    WorkspaceError::new(message)
}
