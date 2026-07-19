use std::{future::Future, pin::Pin};

use serde::Deserialize;

use crate::{
    cli::{CliError, CliRunner, CommandSpec},
    provider::{
        ProviderDiscovery, ProviderId, ProviderWorkspace, Resource, ResourceId, ResourcePanel,
        WorkspaceError, WorkspaceSnapshot,
    },
};

const PROVIDER_ID: &str = "docker";
const PROVIDER_NAME: &str = "Docker";

pub struct DockerWorkspace;

#[derive(Deserialize)]
struct ContainerRow {
    #[serde(rename = "ID")]
    id: String,
    #[serde(rename = "Image")]
    image: String,
    #[serde(rename = "Names")]
    names: String,
    #[serde(rename = "State")]
    state: String,
    #[serde(rename = "Status")]
    status: String,
}

impl ProviderWorkspace for DockerWorkspace {
    fn id(&self) -> ProviderId {
        ProviderId::new(PROVIDER_ID)
    }

    fn discover<'a>(
        &'a self,
        cli: &'a dyn CliRunner,
    ) -> Pin<Box<dyn Future<Output = Option<ProviderDiscovery>> + Send + 'a>> {
        Box::pin(async move {
            let result = cli
                .run(CommandSpec::new("docker", &["context", "show"]))
                .await;

            match result {
                Err(CliError::NotFound) => None,
                Err(CliError::Failed(message)) => Some(discovery_with_error(message)),
                Ok(output) if !output.success => Some(discovery_with_error(cli_message(
                    &output.stderr,
                    "Docker could not report its current context",
                ))),
                Ok(output) => Some(ProviderDiscovery {
                    id: self.id(),
                    name: PROVIDER_NAME.to_owned(),
                    target_environment: output.stdout.trim().to_owned(),
                    error: None,
                }),
            }
        })
    }

    fn refresh<'a>(
        &'a self,
        cli: &'a dyn CliRunner,
    ) -> Pin<Box<dyn Future<Output = Result<WorkspaceSnapshot, WorkspaceError>> + Send + 'a>> {
        Box::pin(async move {
            let output = cli
                .run(CommandSpec::new(
                    "docker",
                    &[
                        "container",
                        "ls",
                        "--all",
                        "--no-trunc",
                        "--format",
                        "{{json .}}",
                    ],
                ))
                .await
                .map_err(|error| match error {
                    CliError::NotFound => WorkspaceError::new("Docker CLI is no longer available"),
                    CliError::Failed(message) => WorkspaceError::new(message),
                })?;

            if !output.success {
                return Err(refresh_error(cli_message(
                    &output.stderr,
                    "Docker could not list containers",
                )));
            }

            let resources = output
                .stdout
                .lines()
                .filter(|line| !line.trim().is_empty())
                .map(|line| {
                    let row: ContainerRow = serde_json::from_str(line).map_err(|error| {
                        refresh_error(format!("Docker returned malformed container data: {error}"))
                    })?;
                    Ok(Resource {
                        id: ResourceId::new(row.id),
                        name: row.names,
                        status: Some(row.state),
                        fields: vec![
                            ("Image".to_owned(), row.image),
                            ("Status".to_owned(), row.status),
                        ],
                    })
                })
                .collect::<Result<Vec<_>, WorkspaceError>>()?;

            Ok(WorkspaceSnapshot {
                panels: vec![ResourcePanel {
                    title: "Containers".to_owned(),
                    resources,
                }],
            })
        })
    }
}

fn discovery_with_error(message: impl Into<String>) -> ProviderDiscovery {
    let message = message.into();
    ProviderDiscovery {
        id: ProviderId::new(PROVIDER_ID),
        name: PROVIDER_NAME.to_owned(),
        target_environment: "unavailable".to_owned(),
        error: Some(WorkspaceError::new(format!(
            "{message}. Run `docker context show` to verify the selected context and ensure Docker is running."
        ))),
    }
}

fn cli_message(stderr: &str, fallback: &str) -> String {
    let stderr = stderr.trim();
    if stderr.is_empty() {
        fallback.to_owned()
    } else {
        stderr.to_owned()
    }
}

fn refresh_error(message: impl Into<String>) -> WorkspaceError {
    WorkspaceError::new(format!(
        "{}. Run `docker container ls --all` to verify access to the current Target Environment.",
        message.into()
    ))
}
