use std::{fs, sync::OnceLock};

use serde_yaml::{Mapping, Value};

fn workflow() -> &'static Mapping {
    static WORKFLOW: OnceLock<Mapping> = OnceLock::new();

    WORKFLOW.get_or_init(|| {
        let source =
            fs::read_to_string(".github/workflows/ci.yml").expect("the CI workflow should exist");
        let document: Value =
            serde_yaml::from_str(&source).expect("the CI workflow should be YAML");

        document
            .as_mapping()
            .expect("the CI workflow should have a mapping document")
            .clone()
    })
}

fn field<'a>(workflow: &'a Mapping, name: &str) -> &'a Value {
    workflow
        .get(Value::String(name.to_owned()))
        .unwrap_or_else(|| panic!("the CI workflow should define {name}"))
}

fn commands(job: &Mapping) -> Vec<&str> {
    job.get(Value::String("steps".to_owned()))
        .and_then(Value::as_sequence)
        .expect("CI jobs should define steps")
        .iter()
        .filter_map(Value::as_mapping)
        .filter_map(|step| step.get(Value::String("run".to_owned())))
        .filter_map(Value::as_str)
        .collect()
}

#[test]
fn ci_runs_for_pull_requests_and_master_pushes() {
    let document = workflow();
    let triggers = field(document, "on")
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
    let permissions = field(document, "permissions")
        .as_mapping()
        .expect("CI permissions should be a mapping");

    assert_eq!(permissions.len(), 1);
    assert_eq!(
        permissions.get(Value::String("contents".to_owned())),
        Some(&Value::String("read".to_owned()))
    );

    let concurrency = field(document, "concurrency")
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

#[test]
fn ci_runs_locked_checks_with_the_declared_toolchain() {
    let document = workflow();
    let jobs = field(document, "jobs")
        .as_mapping()
        .expect("CI should define jobs");
    let checks = jobs
        .get(Value::String("checks".to_owned()))
        .and_then(Value::as_mapping)
        .expect("CI should define a checks job");
    let checks = commands(checks);

    assert!(
        checks
            .iter()
            .any(|command| command.contains(
                "rustup toolchain install \"$(awk -F '\"' '$1 ~ /^channel/ { print $2 }' rust-toolchain.toml)\""
            ))
    );
    assert!(
        checks
            .iter()
            .any(|command| command.contains("cargo fmt --all -- --check"))
    );
    assert!(checks.iter().any(|command| {
        command.contains("cargo clippy --all-targets --all-features --locked")
            && command.contains("-D warnings")
    }));
    assert!(
        checks.iter().any(|command| {
            command.contains("cargo test --all-targets --all-features --locked")
        })
    );
}

#[test]
fn ci_action_references_are_immutable_commit_pins() {
    let document = workflow();
    let jobs = field(document, "jobs")
        .as_mapping()
        .expect("CI should define jobs");

    for (job_name, job) in jobs {
        let job = job
            .as_mapping()
            .unwrap_or_else(|| panic!("CI job {job_name:?} should be a mapping"));
        let steps = job
            .get(Value::String("steps".to_owned()))
            .and_then(Value::as_sequence)
            .expect("CI jobs should define steps");

        for step in steps {
            let Some(action) = step
                .as_mapping()
                .and_then(|step| step.get(Value::String("uses".to_owned())))
                .and_then(Value::as_str)
            else {
                continue;
            };
            let (_, commit) = action
                .rsplit_once('@')
                .expect("CI actions should include a commit pin");
            assert_eq!(commit.len(), 40, "action {action} is not pinned to a SHA");
            assert!(commit.bytes().all(|byte| byte.is_ascii_hexdigit()));
        }
    }
}

#[test]
fn ci_validates_its_workflow_definition() {
    let document = workflow();
    let jobs = field(document, "jobs")
        .as_mapping()
        .expect("CI should define jobs");
    let validate = jobs
        .get(Value::String("validate".to_owned()))
        .and_then(Value::as_mapping)
        .expect("CI should define a workflow validation job");
    let validate = commands(validate);

    assert!(
        validate
            .iter()
            .any(|command| command.contains("actionlint_1.7.12_linux_amd64.tar.gz"))
    );
    assert!(
        validate
            .iter()
            .any(|command| command.contains("sha256sum --check"))
    );
    assert!(validate.iter().any(|command| {
        command.contains("8aca8db96f1b94770f1b0d72b6dddcb1ebb8123cb3712530b08cc387b349a3d8")
    }));
    assert!(
        validate
            .iter()
            .any(|command| command.lines().any(|line| line.trim() == "./actionlint"))
    );
}
