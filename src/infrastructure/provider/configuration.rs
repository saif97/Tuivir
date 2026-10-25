use super::ProviderWorkspace;
use crate::{
    application::{ConfigurationOutcome, ConfigurationReview, WorkspaceError},
    infrastructure::process::CliRunner,
};

pub async fn apply_configuration(
    workspace: &dyn ProviderWorkspace,
    cli: &dyn CliRunner,
    review: &ConfigurationReview,
) -> ConfigurationOutcome {
    let actual = match workspace.load_configuration(cli, &review.target).await {
        Ok(actual) => actual,
        Err(error) => {
            return ConfigurationOutcome {
                actual: None,
                error: Some(error.message),
            };
        }
    };
    let validation = review.changes.iter().try_for_each(|change| {
        let field = actual
            .fields
            .iter()
            .find(|field| field.id == change.field.id)
            .ok_or_else(|| WorkspaceError::new("Edited field is no longer available"))?;
        field
            .validate(&change.proposed)
            .map_err(WorkspaceError::new)?;
        if field.update != change.field.update
            || field.constraint != change.field.constraint
            || !field.matches(&change.field.value)
        {
            return Err(WorkspaceError::new(
                "Configuration changed externally. Review actual values and Apply again.",
            ));
        }
        Ok(())
    });
    if let Err(error) = validation {
        return ConfigurationOutcome {
            actual: Some(actual),
            error: Some(error.message),
        };
    }
    if actual.state != review.actual.state || actual.notice != review.actual.notice {
        return ConfigurationOutcome {
            actual: Some(actual),
            error: Some(
                "Resource State or downtime requirements changed. Review and Apply again.".into(),
            ),
        };
    }
    if !matches!(
        actual.state,
        crate::domain::ResourceState::Running | crate::domain::ResourceState::Stopped
    ) {
        return ConfigurationOutcome { actual: Some(actual), error: Some("Configuration changes require a running or stopped Resource; resolve its current Resource State first.".into()) };
    }
    if actual.state != crate::domain::ResourceState::Stopped
        && !actual.stop_preserves_resource
        && review
            .changes
            .iter()
            .any(|change| change.field.update.requires_stop(&change.proposed))
    {
        return ConfigurationOutcome {
            actual: Some(actual),
            error: Some(
                "Stopping this Resource would remove it; refusing an in-place update.".into(),
            ),
        };
    }
    let result = workspace
        .write_configuration(cli, &review.target, &actual, &review.changes)
        .await;
    let refreshed = workspace.load_configuration(cli, &review.target).await;
    let mut error = result.err().map(|error| error.message);
    match refreshed {
        Ok(actual) => ConfigurationOutcome {
            actual: Some(actual),
            error,
        },
        Err(refresh_error) => {
            error = Some(match error {
                Some(error) => format!(
                    "{error}; could not refresh actual configuration: {}",
                    refresh_error.message
                ),
                None => format!(
                    "Could not verify actual configuration: {}",
                    refresh_error.message
                ),
            });
            ConfigurationOutcome {
                actual: None,
                error,
            }
        }
    }
}

/// Preserve native settings that have no dedicated control or readable field.
/// Paths use JSON Pointer syntax so native keys containing dots stay distinct.
pub(super) fn append_native_fields(
    fields: &mut Vec<crate::application::ConfigurationField>,
    value: &serde_json::Value,
    excluded: &[&str],
) {
    fn append(
        fields: &mut Vec<crate::application::ConfigurationField>,
        value: &serde_json::Value,
        path: String,
        label: String,
        excluded: &[&str],
    ) {
        if excluded.contains(&path.as_str()) {
            return;
        }
        if let Some(object) = value.as_object()
            && !object.is_empty()
        {
            for (key, value) in object {
                let escaped = key.replace('~', "~0").replace('/', "~1");
                append(
                    fields,
                    value,
                    format!("{path}/{escaped}"),
                    if label.is_empty() {
                        key.clone()
                    } else {
                        format!("{label}.{key}")
                    },
                    excluded,
                );
            }
        } else if !path.is_empty() {
            fields.push(crate::application::ConfigurationField::read_only(
                format!("native:{path}"),
                label,
                value
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| value.to_string()),
            ));
        }
    }
    append(fields, value, String::new(), String::new(), excluded);
}
