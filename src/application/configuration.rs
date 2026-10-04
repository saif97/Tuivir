use super::ProviderRequestId;
use crate::domain::ResourceState;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfigurationField {
    pub id: String,
    pub label: String,
    pub value: String,
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
