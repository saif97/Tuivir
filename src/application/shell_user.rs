use crate::domain::{ProviderId, ResourceTarget, TargetEnvironment};

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ShellUserKey {
    pub provider_id: ProviderId,
    pub target_environment: Option<TargetEnvironment>,
    pub target: ResourceTarget,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DefaultShellUser {
    pub key: ShellUserKey,
    pub username: String,
}
