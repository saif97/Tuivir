use std::fs;

use serde_yaml::Value;

#[test]
fn ci_workflow_is_valid_yaml() {
    let source = fs::read_to_string(".github/workflows/ci.yml")
        .expect("the CI workflow should exist");

    let document: Value = serde_yaml::from_str(&source).expect("the CI workflow should be YAML");

    assert!(document.is_mapping());
}
