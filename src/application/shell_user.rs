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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShellUserPicker {
    pub key: ShellUserKey,
    pub input: String,
    pub accounts: Option<Vec<String>>,
    pub selected: usize,
    pub loading: bool,
    pub verifying: bool,
    pub error: Option<String>,
    pub(crate) request_id: super::ProviderRequestId,
    pub(crate) enlarged: bool,
}

impl ShellUserPicker {
    /// Subsequence matching keeps typing useful without imposing account policy.
    pub fn candidates(&self) -> Vec<&str> {
        self.accounts
            .iter()
            .flatten()
            .filter(|name| {
                let query = self.input.to_lowercase();
                let mut wanted = query.chars();
                let mut next = wanted.next();
                for c in name.to_lowercase().chars() {
                    if next == Some(c) {
                        next = wanted.next();
                    }
                }
                next.is_none()
            })
            .map(String::as_str)
            .collect()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedShellUser {
    pub key: ShellUserKey,
    pub username: String,
    pub(crate) process: super::ResourceShellProcess,
    pub(crate) enlarged: bool,
}

pub(crate) struct ShellUserVerification {
    pub request_id: super::ProviderRequestId,
    pub key: ShellUserKey,
    pub username: String,
    pub enlarged: bool,
}
