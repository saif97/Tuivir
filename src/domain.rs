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
    target_environment: TargetEnvironment,
    version: Option<ProviderVersion>,
}

impl Provider {
    pub fn new(
        id: ProviderId,
        name: impl Into<String>,
        target_environment: TargetEnvironment,
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

    pub fn target_environment(&self) -> &TargetEnvironment {
        &self.target_environment
    }

    pub fn version(&self) -> Option<&ProviderVersion> {
        self.version.as_ref()
    }
}
