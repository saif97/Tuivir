use super::{ProviderRequestId, TextSelection};
use crate::domain::ResourceState;

/// An exact memory size displayed and edited in GiB.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MemoryGiB(u64);

impl MemoryGiB {
    pub fn from_bytes(bytes: u64) -> Self {
        Self(bytes)
    }
    pub fn bytes(self) -> u64 {
        self.0
    }

    pub fn from_native_quantity(value: &str) -> Option<Self> {
        quantity_bytes(value).map(Self)
    }

    pub fn parse(value: &str) -> Option<Self> {
        if value.chars().any(|c| c.is_ascii_alphabetic()) {
            return Self::from_native_quantity(value);
        }
        let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
        let whole = if whole.is_empty() && !fraction.is_empty() {
            0
        } else {
            whole.parse::<u64>().ok()?
        };
        let fraction = fraction.trim_end_matches('0');
        if !fraction.bytes().all(|c| c.is_ascii_digit()) || fraction.len() > 30 {
            return None;
        }
        let fraction_bytes = if fraction.is_empty() {
            0
        } else {
            let numerator = fraction.parse::<u128>().ok()?;
            let divisor = 5u128.pow(fraction.len() as u32);
            if numerator % divisor != 0 {
                return None;
            }
            u64::try_from((numerator / divisor) << (30 - fraction.len())).ok()?
        };
        whole
            .checked_mul(1 << 30)?
            .checked_add(fraction_bytes)
            .map(Self)
    }
}

impl std::fmt::Display for MemoryGiB {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let whole = self.0 / (1 << 30);
        let remainder = self.0 % (1 << 30);
        if remainder == 0 {
            return write!(formatter, "{whole}");
        }
        let fraction = format!("{:030}", u128::from(remainder) * 5u128.pow(30));
        write!(formatter, "{whole}.{}", fraction.trim_end_matches('0'))
    }
}

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
    pub stop_preserves_resource: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ConfigurationDraft {
    pub actual: Option<ResourceConfiguration>,
    pub error: Option<String>,
    pub proposed: Vec<String>,
    pub selected_field: usize,
    pub editing: bool,
    pub applying: bool,
    pub scroll: u16,
    pub selection: Option<ConfigurationSelection>,
    pub(crate) scroll_limit: u16,
    pub(crate) request_id: Option<ProviderRequestId>,
}

/// A selection in one original value; visual wrapping never changes its byte offsets.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfigurationSelection {
    pub field: usize,
    pub text: TextSelection<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FieldConstraint {
    ReadOnly,
    MemoryGiB {
        minimum_bytes: u64,
        maximum_bytes: Option<u64>,
        step_bytes: u64,
        unlimited: bool,
        percent: bool,
    },
    Integer {
        minimum: u64,
        maximum: u64,
    },
    CpuSelection {
        topology: bool,
    },
    Decimal {
        units_per_value: u64,
        minimum_units: u64,
    },
}
impl ConfigurationField {
    pub fn read_only(
        id: impl Into<String>,
        label: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            value: value.into(),
            constraint: FieldConstraint::ReadOnly,
            update: ConfigurationUpdate::Live,
        }
    }

    pub fn editable(&self) -> bool {
        self.constraint != FieldConstraint::ReadOnly
    }

    pub fn matches(&self, value: &str) -> bool {
        if self.value == value {
            return true;
        }
        match self.constraint {
            FieldConstraint::Integer { .. } => self
                .value
                .parse::<u64>()
                .ok()
                .zip(value.parse::<u64>().ok())
                .is_some_and(|(a, b)| a == b),
            FieldConstraint::MemoryGiB { .. } => MemoryGiB::parse(&self.value)
                .zip(MemoryGiB::parse(value))
                .is_some_and(|(a, b)| a == b),
            FieldConstraint::ReadOnly => false,
            FieldConstraint::CpuSelection { .. } => false,
            FieldConstraint::Decimal { .. } => self
                .value
                .parse::<f64>()
                .ok()
                .zip(value.parse::<f64>().ok())
                .is_some_and(|(a, b)| a == b),
        }
    }

    pub fn validate(&self, value: &str) -> Result<(), String> {
        let valid = match self.constraint {
            FieldConstraint::Integer { minimum, maximum } => value
                .parse::<u64>()
                .is_ok_and(|n| n >= minimum && n <= maximum),
            FieldConstraint::MemoryGiB {
                minimum_bytes,
                maximum_bytes,
                step_bytes,
                unlimited,
                percent,
            } => {
                (percent && valid_memory_percent(value))
                    || MemoryGiB::parse(value).is_some_and(|size| {
                        let bytes = size.bytes();
                        (unlimited && bytes == 0)
                            || (bytes >= minimum_bytes
                                && maximum_bytes.is_none_or(|max| bytes <= max)
                                && bytes % step_bytes == 0)
                    })
            }
            FieldConstraint::ReadOnly => false,
            FieldConstraint::CpuSelection { topology } => valid_cpu_selection(value, topology),
            FieldConstraint::Decimal {
                units_per_value,
                minimum_units,
            } => value.parse::<f64>().is_ok_and(|n| {
                n.is_finite()
                    && n >= 0.0
                    && (n == 0.0 || n * units_per_value as f64 >= minimum_units as f64)
                    && n < i64::MAX as f64 / units_per_value as f64
            }),
        };
        if valid {
            Ok(())
        } else {
            Err(match self.constraint {
                FieldConstraint::Integer { minimum, maximum } => format!(
                    "{} must be a whole number from {minimum} to {maximum}.",
                    self.label
                ),
                FieldConstraint::MemoryGiB {
                    minimum_bytes,
                    step_bytes,
                    unlimited,
                    percent,
                    maximum_bytes,
                } => {
                    if let Some(maximum) = maximum_bytes
                        && MemoryGiB::parse(value).is_some_and(|size| size.bytes() > maximum)
                    {
                        format!(
                            "Memory exceeds the existing Provider limit ({} GiB). Adjust that limit outside Tuivir first.",
                            MemoryGiB::from_bytes(maximum)
                        )
                    } else {
                        format!(
                            "Memory must be a size in GiB (for example 0.5 or 1.5), at least {} GiB, in increments of {} GiB{}{}.",
                            MemoryGiB::from_bytes(minimum_bytes),
                            MemoryGiB::from_bytes(step_bytes),
                            if unlimited { " (0 is unlimited)" } else { "" },
                            if percent {
                                "; or a whole percentage from 1–100%"
                            } else {
                                ""
                            }
                        )
                    }
                }
                FieldConstraint::ReadOnly => "This field is read-only.".into(),
                FieldConstraint::CpuSelection { topology } => format!(
                    "CPU setting must be a positive count or CPU IDs (0-3,5){}.",
                    if topology {
                        " or topology (sockets=2,cores=4,threads=2)"
                    } else {
                        ""
                    }
                ),
                FieldConstraint::Decimal { .. } => {
                    "CPU limit must be a finite number of CPUs, 0 or greater (0 is unlimited)."
                        .into()
                }
            })
        }
    }
}
impl ConfigurationDraft {
    pub fn select_value(&mut self, field: usize, offset: usize) {
        let Some(value) = self
            .actual
            .as_ref()
            .and_then(|actual| actual.fields.get(field))
        else {
            return;
        };
        if value.editable() || !value.value.is_char_boundary(offset) {
            return;
        }
        self.editing = false;
        self.selection = Some(ConfigurationSelection {
            field,
            text: TextSelection::whole_value(offset, 0..value.value.len()),
        });
    }

    pub fn extend_selection(&mut self, field: usize, offset: usize) {
        let Some(selection) = &mut self.selection else {
            return;
        };
        let Some(value) = self
            .actual
            .as_ref()
            .and_then(|actual| actual.fields.get(selection.field))
        else {
            return;
        };
        let cursor = match field.cmp(&selection.field) {
            std::cmp::Ordering::Less => 0,
            std::cmp::Ordering::Greater => value.value.len(),
            std::cmp::Ordering::Equal => offset,
        };
        if value.value.is_char_boundary(cursor) {
            selection.text.extend(cursor);
        }
    }

    pub fn selected_value_text(&self) -> Option<String> {
        let selection = self.selection.as_ref()?;
        let field = self.actual.as_ref()?.fields.get(selection.field)?;
        if field.editable() {
            return None;
        }
        field
            .value
            .get(selection.text.range())
            .filter(|text| !text.is_empty())
            .map(str::to_owned)
    }

    pub fn reconcile(&mut self, actual: ResourceConfiguration) {
        self.selection = None;
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
        if actual.state != ResourceState::Stopped
            && !actual.stop_preserves_resource
            && actual
                .fields
                .iter()
                .zip(&self.proposed)
                .any(|(field, proposed)| {
                    !field.matches(proposed) && field.update.requires_stop(proposed)
                })
        {
            return Some(
                "Stopping this Resource would remove it; this change cannot be applied in place."
                    .into(),
            );
        }
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
    let mut ranges = Vec::new();
    for part in value.split(',') {
        let range = if let Some((start, end)) = part.split_once('-') {
            let (Ok(start), Ok(end)) = (start.parse::<u32>(), end.parse::<u32>()) else {
                return false;
            };
            if end < start || end - start > 65536 {
                return false;
            }
            (start, end)
        } else {
            let Ok(id) = part.parse::<u32>() else {
                return false;
            };
            (id, id)
        };
        ranges.push(range);
    }
    ranges.sort_unstable();
    ranges.windows(2).all(|pair| pair[0].1 < pair[1].0)
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

fn valid_memory_percent(value: &str) -> bool {
    value
        .strip_suffix('%')
        .and_then(|n| n.parse::<u8>().ok())
        .is_some_and(|n| n > 0 && n <= 100)
}
