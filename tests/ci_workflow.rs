use std::fs;

use serde_yaml::{Mapping, Value};

fn workflow() -> Mapping {
    let source = fs::read_to_string(".github/workflows/ci.yml")
        .expect("the CI workflow should exist");
    let document: Value = serde_yaml::from_str(&source).expect("the CI workflow should be YAML");

    document
        .as_mapping()
        .expect("the CI workflow should have a mapping document")
        .clone()
}

fn field<'a>(workflow: &'a Mapping, name: &str) -> &'a Value {
    workflow
        .get(Value::String(name.to_owned()))
        .unwrap_or_else(|| panic!("the CI workflow should define {name}"))
}

#[test]
fn ci_workflow_is_valid_yaml() {
    assert!(!workflow().is_empty());
}

#[test]
fn ci_runs_for_pull_requests_and_master_pushes() {
    let document = workflow();
    let triggers = field(&document, "on")
        .as_mapping()
        .expect("CI triggers should be a mapping");

    assert!(triggers.contains_key(Value::String("pull_request".to_owned())));

    let push = triggers
        .get(Value::String("push".to_owned()))
        .expect("CI should run on pushes")
        .as_mapping()
        .expect("push trigger should be a mapping");
    let branches = push
        .get(Value::String("branches".to_owned()))
        .expect("push trigger should select branches")
        .as_sequence()
        .expect("push branches should be a sequence");

    assert!(branches.contains(&Value::String("master".to_owned())));
}

#[test]
fn ci_has_read_only_contents_and_cancels_superseded_revisions() {
    let document = workflow();
    let permissions = field(&document, "permissions")
        .as_mapping()
        .expect("CI permissions should be a mapping");

    assert_eq!(permissions.len(), 1);
    assert_eq!(
        permissions.get(Value::String("contents".to_owned())),
        Some(&Value::String("read".to_owned()))
    );

    let concurrency = field(&document, "concurrency")
        .as_mapping()
        .expect("CI concurrency should be a mapping");
    assert_eq!(
        concurrency.get(Value::String("cancel-in-progress".to_owned())),
        Some(&Value::Bool(true))
    );
    let group = concurrency
        .get(Value::String("group".to_owned()))
        .and_then(Value::as_str)
        .expect("CI concurrency should define a group");
    assert!(group.contains("github.workflow"));
    assert!(group.contains("github.ref"));
}
