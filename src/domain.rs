use std::fmt;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ProviderId(pub String);

impl ProviderId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

impl fmt::Display for ProviderId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
/// The Provider-selected destination that owns the Resources Virtui operates.
///
/// This is deliberately distinct from [`ProviderVersion`]: a Docker context
/// or Incus remote/project says where work happens, while a version says which
/// Provider build happens to be installed.
pub struct TargetEnvironment(String);

impl TargetEnvironment {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for TargetEnvironment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl PartialEq<&str> for TargetEnvironment {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
/// The installed Provider build, when discovery can report it.
pub struct ProviderVersion(String);

impl ProviderVersion {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ProviderVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// The stable identity and descriptive information of one installed Provider.
pub struct Provider {
    id: ProviderId,
    name: String,
    target_environment: Option<TargetEnvironment>,
    version: Option<ProviderVersion>,
}

impl Provider {
    pub fn new(
        id: ProviderId,
        name: impl Into<String>,
        target_environment: Option<TargetEnvironment>,
        version: Option<ProviderVersion>,
    ) -> Self {
        Self {
            id,
            name: name.into(),
            target_environment,
            version,
        }
    }

    pub fn id(&self) -> &ProviderId {
        &self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn target_environment(&self) -> Option<&TargetEnvironment> {
        self.target_environment.as_ref()
    }

    pub fn version(&self) -> Option<&ProviderVersion> {
        self.version.as_ref()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// What a Provider reported a Resource to be doing at the last refresh.
///
/// This is a provider-neutral vocabulary that each Provider Workspace maps its
/// own status words into so application policy never branches on Provider
/// identity.
///
/// Only [`ResourceState::Stopped`] is ever positively determined. Every other
/// variant, `Unknown` included, means "not settled and stopped", which is what
/// makes forcing a deletion the safe default: an unrecognised status can never
/// masquerade as a stopped Resource.
pub enum ResourceState {
    Running,
    /// Settled and not running: safe to remove without stopping anything first.
    Stopped,
    /// Suspended but still resident — Docker `paused`, Incus `Frozen`.
    Paused,
    /// Moving between states — Docker `restarting`/`removing`, Incus
    /// `Starting`/`Stopping`/`Freezing`/`Thawing`.
    Transitioning,
    /// The Provider reports the Resource as unusable — Docker `dead`, Incus
    /// `Error`.
    Broken,
    /// A status word this Provider Workspace does not recognise.
    Unknown,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
/// Stable provider-defined identity for one Resource Panel.
pub struct ResourcePanelId(pub String);

impl ResourcePanelId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

impl fmt::Display for ResourcePanelId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ResourceId(pub String);

impl ResourceId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

impl fmt::Display for ResourceId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
/// Identifies one Resource within its provider-defined Resource Panel.
pub struct ResourceTarget {
    panel_id: ResourcePanelId,
    resource_id: ResourceId,
}

impl ResourceTarget {
    pub fn new(panel_id: ResourcePanelId, resource_id: ResourceId) -> Self {
        Self {
            panel_id,
            resource_id,
        }
    }

    pub(crate) fn panel_id(&self) -> &ResourcePanelId {
        &self.panel_id
    }

    pub(crate) fn resource_id(&self) -> &ResourceId {
        &self.resource_id
    }
}

impl fmt::Display for ResourceTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.resource_id.fmt(formatter)
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
/// Identifies one Detail View a Provider Workspace offers for its Resources.
pub struct DetailViewId(pub String);

impl DetailViewId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

impl fmt::Display for DetailViewId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}
