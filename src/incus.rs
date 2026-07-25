use std::{future::Future, pin::Pin};

use serde::Deserialize;

use crate::{
    cli::{CliRunner, ProcessError, ProcessSpec},
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
                .run(ProcessSpec::new("incus", &["remote", "get-default"]))
                .await
            {
                Err(ProcessError::ExecutableNotFound) => return None,
                Err(ProcessError::SpawnFailed(message)) => {
                    return Some(discovery_error(
                        not_started(&message),
                        "incus remote get-default",
                    ));
                }
                Err(ProcessError::Exited(failure)) => {
                    return Some(discovery_error(
                        failure.message_or("Incus could not report its default remote"),
                        "incus remote get-default",
                    ));
                }
                Ok(output) => output.stdout.trim().to_owned(),
            };
            let project = match cli
                .run(ProcessSpec::new("incus", &["project", "get-current"]))
                .await
            {
                Err(ProcessError::ExecutableNotFound) => {
                    return Some(discovery_error(
                        "Incus CLI is no longer available",
                        "incus project get-current",
                    ));
                }
                Err(ProcessError::SpawnFailed(message)) => {
                    return Some(discovery_error(
                        not_started(&message),
                        "incus project get-current",
                    ));
                }
                Err(ProcessError::Exited(failure)) => {
                    return Some(discovery_error(
                        failure.message_or("Incus could not report the current project"),
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
                .run(ProcessSpec::new("incus", &["list", "--format=json"]))
                .await
                .map_err(refresh_failure)?;
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
            cli.run(ProcessSpec::new("incus", &[verb, resource_id.0.as_str()]))
                .await
                .map_err(|error| {
                    command_error(
                        error,
                        &format!("Incus could not {command} instance {resource_id}"),
                    )
                })?;
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

fn refresh_failure(error: ProcessError) -> WorkspaceError {
    match error {
        ProcessError::ExecutableNotFound => WorkspaceError::new("Incus CLI is not available"),
        ProcessError::SpawnFailed(message) => WorkspaceError::new(not_started(&message)),
        ProcessError::Exited(failure) => {
            refresh_error(failure.message_or("Incus could not list instances"))
        }
    }
}

fn not_started(reason: &str) -> String {
    format!("Incus CLI could not be started: {reason}")
}

fn refresh_error(message: impl AsRef<str>) -> WorkspaceError {
    WorkspaceError::new(format!(
        "{}. Run `incus list` to verify access to the current Target Environment.",
        message.as_ref()
    ))
}

fn command_error(error: ProcessError, fallback: &str) -> WorkspaceError {
    let message = match error {
        ProcessError::ExecutableNotFound => "Incus CLI is no longer available".to_owned(),
        ProcessError::SpawnFailed(message) => not_started(&message),
        ProcessError::Exited(failure) => failure.message_or(fallback),
    };
    WorkspaceError::new(message)
}
