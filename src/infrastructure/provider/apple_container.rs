use std::{future::Future, pin::Pin};

use serde::Deserialize;

use super::{
    DetailViewId, Provider, ProviderDiscovery, ProviderId, ProviderVersion, ProviderWorkspace,
    ResourceCommand, ResourceDetails, ResourcePanel, ResourcePanelId, ResourceState,
    ResourceTarget, WorkspaceError, WorkspaceSnapshot, provider_cli_error,
};
use crate::infrastructure::process::{CliRunner, ProcessError, ProcessSpec};

pub struct AppleContainerWorkspace;

const PROVIDER_NAME: &str = "Apple Container";
const RECOVERY: &str = "Run `container system start` outside Tuivir, then refresh the Workspace. Verify the CLI installation with `container system version --format json`; the tested baseline is CLI 1.5.0 on Apple Silicon with macOS 26.";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct VersionRow {
    app_name: String,
    version: String,
}

fn malformed(error: impl std::fmt::Display) -> WorkspaceError {
    WorkspaceError::with_help(
        format!("Apple Container returned malformed or incompatible data: {error}"),
        RECOVERY,
    )
}

impl ProviderWorkspace for AppleContainerWorkspace {
    fn id(&self) -> ProviderId {
        ProviderId::new("apple-container")
    }

    fn discover<'a>(
        &'a self,
        cli: &'a dyn CliRunner,
    ) -> Pin<Box<dyn Future<Output = Option<ProviderDiscovery>> + Send + 'a>> {
        Box::pin(async move {
            let result = cli
                .run(ProcessSpec::new(
                    "container",
                    &["system", "version", "--format", "json"],
                ))
                .await;
            let (version, error) = match result {
                Err(ProcessError::ExecutableNotFound) => return None,
                Err(error) => (
                    None,
                    Some(WorkspaceError::with_help(
                        provider_cli_error(
                            PROVIDER_NAME,
                            &error,
                            "Apple Container could not report its version",
                        ),
                        RECOVERY,
                    )),
                ),
                Ok(output) => match serde_json::from_str::<Vec<VersionRow>>(&output.stdout) {
                    Ok(rows) => match rows.into_iter().find(|row| row.app_name == "container") {
                        Some(row) if !row.version.trim().is_empty() => {
                            (Some(ProviderVersion::new(row.version)), None)
                        }
                        _ => (None, Some(malformed("missing container CLI version"))),
                    },
                    Err(error) => (None, Some(malformed(error))),
                },
            };
            Some(ProviderDiscovery::new(
                Provider::new(self.id(), PROVIDER_NAME, None, version),
                error,
            ))
        })
    }

    fn refresh<'a>(
        &'a self,
        cli: &'a dyn CliRunner,
    ) -> Pin<Box<dyn Future<Output = Result<WorkspaceSnapshot, WorkspaceError>> + Send + 'a>> {
        Box::pin(async move {
            let mut panels = Vec::new();
            for (id, title, args) in [
                (
                    "containers",
                    "Containers",
                    vec!["list", "--all", "--format", "json"],
                ),
                (
                    "images",
                    "Images",
                    vec!["image", "list", "--format", "json"],
                ),
                (
                    "networks",
                    "Networks",
                    vec!["network", "list", "--format", "json"],
                ),
                (
                    "volumes",
                    "Volumes",
                    vec!["volume", "list", "--format", "json"],
                ),
            ] {
                let output = cli
                    .run(ProcessSpec::new("container", &args))
                    .await
                    .map_err(|error| {
                        WorkspaceError::with_help(
                            provider_cli_error(
                                PROVIDER_NAME,
                                &error,
                                &format!("Apple Container could not list {title}"),
                            ),
                            RECOVERY,
                        )
                    })?;
                let _: Vec<serde_json::Value> =
                    serde_json::from_str(&output.stdout).map_err(malformed)?;
                panels.push(ResourcePanel {
                    id: ResourcePanelId::new(id),
                    title: title.into(),
                    detail_views: vec![],
                    resources: vec![],
                });
            }
            Ok(WorkspaceSnapshot { panels })
        })
    }

    fn execute_command<'a>(
        &'a self,
        _cli: &'a dyn CliRunner,
        _target: &'a ResourceTarget,
        _command: ResourceCommand,
        _state: Option<ResourceState>,
    ) -> Pin<Box<dyn Future<Output = Result<(), WorkspaceError>> + Send + 'a>> {
        Box::pin(async { Err(WorkspaceError::new("Apple Container Command unavailable")) })
    }

    fn load_details<'a>(
        &'a self,
        _cli: &'a dyn CliRunner,
        _target: &'a ResourceTarget,
        _view_id: &'a DetailViewId,
    ) -> Pin<Box<dyn Future<Output = Result<ResourceDetails, WorkspaceError>> + Send + 'a>> {
        Box::pin(async { Err(WorkspaceError::new("Apple Container Details unavailable")) })
    }
}
