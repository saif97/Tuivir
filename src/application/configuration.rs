use super::ProviderRequestId;
use crate::domain::ResourceState;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfigurationField {
    pub id: String,
    pub label: String,
    pub value: String,
    pub constraint: FieldConstraint,
    pub update: ConfigurationUpdate,
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
    pub applying: bool,
    pub(crate) request_id: Option<ProviderRequestId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FieldConstraint {
    ReadOnly,
    CpuSelection {
        topology: bool,
    },
    Quantity {
        percent: bool,
    },
    Decimal,
    Bytes {
        minimum: u64,
        unlimited: bool,
        maximum: Option<u64>,
    },
}
impl ConfigurationField {
    pub fn editable(&self) -> bool {
        self.constraint != FieldConstraint::ReadOnly
    }

    pub fn matches(&self, value: &str) -> bool {
        if self.value == value {
            return true;
        }
        match self.constraint {
            FieldConstraint::ReadOnly => false,
            FieldConstraint::CpuSelection { .. } => false,
            FieldConstraint::Quantity { .. } => quantity_bytes(&self.value)
                .zip(quantity_bytes(value))
                .is_some_and(|(a, b)| a == b),
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
            FieldConstraint::ReadOnly => false,
            FieldConstraint::CpuSelection { topology } => valid_cpu_selection(value, topology),
            FieldConstraint::Quantity { percent } => {
                quantity_bytes(value).is_some_and(|n| n > 0)
                    || (percent
                        && value
                            .strip_suffix('%')
                            .and_then(|n| n.parse::<f64>().ok())
                            .is_some_and(|n| n.is_finite() && n > 0.0 && n <= 100.0))
            }
            FieldConstraint::Decimal => value
                .parse::<f64>()
                .is_ok_and(|n| n.is_finite() && n >= 0.0 && n <= u64::MAX as f64 / 1_000_000_000.0),
            FieldConstraint::Bytes {
                minimum,
                unlimited,
                maximum,
            } => value.parse::<u64>().is_ok_and(|n| {
                (unlimited && n == 0) || (n >= minimum && maximum.is_none_or(|max| n <= max))
            }),
        };
        if valid {
            Ok(())
        } else {
            Err(match self.constraint {
                FieldConstraint::ReadOnly => "This field is read-only.".into(),
                FieldConstraint::CpuSelection { topology } => format!(
                    "CPU setting must be a positive count or CPU IDs (0-3,5){}.",
                    if topology {
                        " or topology (sockets=2,cores=4,threads=2)"
                    } else {
                        ""
                    }
                ),
                FieldConstraint::Quantity { percent } => format!(
                    "Memory must be positive bytes or a size such as 512MiB{}.",
                    if percent { " or 1–100%" } else { "" }
                ),
                FieldConstraint::Decimal => {
                    "CPU limit must be a finite number of CPUs, 0 or greater (0 is unlimited)."
                        .into()
                }
                FieldConstraint::Bytes {
                    maximum: Some(maximum),
                    ..
                } if value.parse::<u64>().is_ok_and(|n| n > maximum) => format!(
                    "Memory exceeds the existing memory+swap limit ({maximum} bytes). Adjust that limit outside Tuivir first."
                ),
                FieldConstraint::Bytes {
                    minimum, unlimited, ..
                } => format!(
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

fn valid_cpu_selection(value: &str, topology: bool) -> bool {
    if let Ok(count) = value.parse::<u32>() {
        return count > 0;
    }
    if topology && value.contains('=') {
        let mut keys = std::collections::HashSet::new();
        let mut count = 1u32;
        return value.split(',').all(|part| {
            let Some((key, number)) = part.split_once('=') else {
                return false;
            };
            if !["sockets", "cores", "threads"].contains(&key) || !keys.insert(key) {
                return false;
            }
            let Ok(number) = number.parse::<u32>() else {
                return false;
            };
            let Some(next) = count.checked_mul(number) else {
                return false;
            };
            count = next;
            number > 0
        });
    }
    let mut ids = std::collections::HashSet::new();
    value.split(',').all(|part| {
        let range = if let Some((start, end)) = part.split_once('-') {
            let (Ok(start), Ok(end)) = (start.parse::<u32>(), end.parse::<u32>()) else {
                return false;
            };
            if end < start || end - start > 65536 {
                return false;
            };
            start..=end
        } else {
            let Ok(id) = part.parse::<u32>() else {
                return false;
            };
            id..=id
        };
        range.into_iter().all(|id| ids.insert(id))
    })
}

fn quantity_bytes(value: &str) -> Option<u64> {
    if let Ok(bytes) = value.parse::<u64>() {
        return Some(bytes);
    }
    let boundary = value.find(|c: char| c.is_ascii_alphabetic())?;
    let number = value[..boundary].parse::<f64>().ok()?;
    let multiplier = match &value[boundary..] {
        "B" => 1.0,
        "kB" | "KB" => 1_000.0,
        "MB" => 1_000_000.0,
        "GB" => 1_000_000_000.0,
        "TB" => 1_000_000_000_000.0,
        "KiB" => 1024.0,
        "MiB" => 1_048_576.0,
        "GiB" => 1_073_741_824.0,
        "TiB" => 1_099_511_627_776.0,
        _ => return None,
    };
    let bytes = number * multiplier;
    (bytes.is_finite() && bytes >= 1.0 && bytes < u64::MAX as f64 && bytes.fract() == 0.0)
        .then_some(bytes as u64)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConfigurationUpdate {
    Live,
    Stopped,
    CpuHotplug { maximum: u32 },
}
impl ConfigurationUpdate {
    pub fn requires_stop(&self, proposed: &str) -> bool {
        match self {
            Self::Live => false,
            Self::Stopped => true,
            Self::CpuHotplug { maximum } => proposed
                .parse::<u32>()
                .ok()
                .is_none_or(|count| count > *maximum),
        }
    }
}
impl ConfigurationReview {
    pub fn downtime(&self) -> &'static str {
        if self.actual.state == ResourceState::Stopped {
            "Resource remains stopped."
        } else if self
            .changes
            .iter()
            .any(|change| change.field.update.requires_stop(&change.proposed))
        {
            "Downtime: stop/start the same Resource; return it to running."
        } else {
            "Live update; no restart required."
        }
    }
}
