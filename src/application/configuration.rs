use super::ProviderRequestId;
use crate::domain::ResourceState;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfigurationField {
    pub id: String,
    pub label: String,
    pub value: String,
    pub constraint: FieldConstraint,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResourceConfiguration {
    pub fields: Vec<ConfigurationField>,
    pub state: ResourceState,
    pub notice: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfigurationDraft {
    pub actual: Option<ResourceConfiguration>,
    pub error: Option<String>,
    pub proposed: Vec<String>,
    pub selected_field: usize,
    pub editing: bool,
    pub(crate) request_id: Option<ProviderRequestId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FieldConstraint {
    Decimal,
    Bytes { minimum: u64, unlimited: bool },
}
impl ConfigurationField {
    pub fn matches(&self, value: &str) -> bool {
        if self.value == value {
            return true;
        }
        match self.constraint {
            FieldConstraint::Decimal => self
                .value
                .parse::<f64>()
                .ok()
                .zip(value.parse::<f64>().ok())
                .is_some_and(|(a, b)| a == b),
            FieldConstraint::Bytes { .. } => self
                .value
                .parse::<u64>()
                .ok()
                .zip(value.parse::<u64>().ok())
                .is_some_and(|(a, b)| a == b),
        }
    }

    pub fn validate(&self, value: &str) -> Result<(), String> {
        let valid = match self.constraint {
            FieldConstraint::Decimal => value
                .parse::<f64>()
                .is_ok_and(|n| n.is_finite() && n >= 0.0 && n <= u64::MAX as f64 / 1_000_000_000.0),
            FieldConstraint::Bytes { minimum, unlimited } => value
                .parse::<u64>()
                .is_ok_and(|n| (unlimited && n == 0) || n >= minimum),
        };
        if valid {
            Ok(())
        } else {
            Err(match self.constraint {
                FieldConstraint::Decimal => {
                    "CPU limit must be a finite number of CPUs, 0 or greater (0 is unlimited)."
                        .into()
                }
                FieldConstraint::Bytes { minimum, unlimited } => format!(
                    "{} must be whole bytes, at least {minimum}{}.",
                    self.label,
                    if unlimited {
                        " (or 0 for unlimited)"
                    } else {
                        ""
                    }
                ),
            })
        }
    }
}
impl ConfigurationDraft {
    pub fn reconcile(&mut self, actual: ResourceConfiguration) {
        self.proposed = actual
            .fields
            .iter()
            .map(|field| {
                let prior = self.actual.as_ref().and_then(|old| {
                    old.fields
                        .iter()
                        .position(|old| old.id == field.id)
                        .map(|index| (&old.fields[index], index))
                });
                let Some((old, index)) = prior else {
                    return field.value.clone();
                };
                let Some(attempted) = self.proposed.get(index) else {
                    return field.value.clone();
                };
                if old.matches(attempted) || field.matches(attempted) {
                    field.value.clone()
                } else {
                    attempted.clone()
                }
            })
            .collect();
        self.actual = Some(actual);
    }

    pub fn validation_error(&self) -> Option<String> {
        let actual = self.actual.as_ref()?;
        actual
            .fields
            .iter()
            .zip(&self.proposed)
            .filter(|(field, value)| !field.matches(value))
            .find_map(|(field, value)| field.validate(value).err())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfigurationChange {
    pub field: ConfigurationField,
    pub proposed: String,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfigurationReview {
    pub provider_id: crate::domain::ProviderId,
    pub target: crate::domain::ResourceTarget,
    pub resource_name: String,
    pub actual: ResourceConfiguration,
    pub changes: Vec<ConfigurationChange>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfigurationOutcome {
    pub actual: Option<ResourceConfiguration>,
    pub error: Option<String>,
}
