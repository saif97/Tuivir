mod common;

use common::{FixtureCli, success};
use std::sync::Arc;
use tuivir::{
    application::{App, Command, ProviderRequest},
    infrastructure::{process::ProcessSpec, provider::DockerWorkspace, runtime::ProviderRuntime},
    presentation::render_to_text,
};

async fn drive(app: &mut App, runtime: &ProviderRuntime, requests: Vec<ProviderRequest>) {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let mut pending = requests;
    while !pending.is_empty() {
        let count = pending.len();
        for request in pending.drain(..) {
            runtime.dispatch(request, tx.clone());
        }
        for _ in 0..count {
            let event = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
                .await
                .unwrap()
                .unwrap();
            pending.extend(app.update(event));
        }
    }
}

async fn docker_app(inspect: &str) -> (App, ProviderRuntime) {
    let (app, runtime, _) = docker_app_with(inspect, Vec::new()).await;
    (app, runtime)
}

async fn docker_app_with(
    inspect: &str,
    extra: Vec<(
        ProcessSpec,
        Result<
            tuivir::infrastructure::process::ProcessOutput,
            tuivir::infrastructure::process::ProcessError,
        >,
    )>,
) -> (App, ProviderRuntime, Arc<FixtureCli>) {
    let cli = Arc::new(FixtureCli::new(
        [
            (
                ProcessSpec::new("docker", &["context", "show"]),
                success("default"),
            ),
            (
                ProcessSpec::new(
                    "docker",
                    &[
                        "container",
                        "ls",
                        "--all",
                        "--no-trunc",
                        "--format",
                        "{{json .}}",
                    ],
                ),
                success(include_str!("fixtures/docker/containers.jsonl")),
            ),
            (
                ProcessSpec::new(
                    "docker",
                    &["image", "ls", "--no-trunc", "--format", "{{json .}}"],
                ),
                success(""),
            ),
            (
                ProcessSpec::new("docker", &["volume", "ls", "--format", "{{json .}}"]),
                success(""),
            ),
            (
                ProcessSpec::new("docker", &["container", "inspect", "a1b2c3d4e5f6"]),
                success(inspect),
            ),
        ]
        .into_iter()
        .chain(extra),
    ));
    let runtime = ProviderRuntime::new(vec![Arc::new(DockerWorkspace)], cli.clone());
    let mut app = App::new();
    let discovery = runtime.discover().await.remove(0);
    let requests = app.update(discovery.into_event());
    drive(&mut app, &runtime, requests).await;
    (app, runtime, cli)
}

#[tokio::test]
async fn configuration_tab_displays_current_docker_limits_with_units() {
    let (mut app, runtime) = docker_app(r#"[{"HostConfig":{"NanoCpus":1500000000,"Memory":536870912,"MemorySwap":1073741824},"State":{"Status":"running"}}]"#).await;
    app.invoke(Command::FocusDetails);
    let requests = app.invoke(Command::ActivateDetailView(4));
    drive(&mut app, &runtime, requests).await;
    let screen = render_to_text(app.state(), 140, 32);
    assert!(screen.contains("CPU limit (CPUs): 1.5"), "{screen}");
    assert!(
        screen.contains("Memory limit (bytes): 536870912"),
        "{screen}"
    );
    assert!(screen.contains("Apply"), "{screen}");
}

fn press(app: &mut App, spelling: &str) -> Vec<ProviderRequest> {
    let key = tuivir::application::Key::parse(spelling).unwrap();
    let command = app
        .resolve_command(key)
        .unwrap_or_else(|| panic!("unbound {spelling}"));
    app.invoke(command)
}

#[tokio::test]
async fn typing_configuration_retains_an_unapplied_draft_across_navigation() {
    let (mut app, runtime) = docker_app(r#"[{"HostConfig":{"NanoCpus":1500000000,"Memory":536870912},"State":{"Status":"running"}}]"#).await;
    let requests = app.invoke(Command::ActivateDetailView(4));
    drive(&mut app, &runtime, requests).await;
    press(&mut app, "enter");
    press(&mut app, "ctrl+u");
    press(&mut app, "2");
    press(&mut app, "enter");
    let screen = render_to_text(app.state(), 140, 32);
    assert!(screen.contains("1.5 → 2"), "{screen}");
    app.invoke(Command::FocusResourcePanel(0));
    app.invoke(Command::SelectNext);
    app.invoke(Command::SelectPrevious);
    app.invoke(Command::FocusDetails);
    let screen = render_to_text(app.state(), 140, 32);
    assert!(screen.contains("1.5 → 2"), "{screen}");
}

#[tokio::test]
async fn invalid_cpu_is_explained_without_dispatching_provider_work() {
    let (mut app, runtime) = docker_app(r#"[{"HostConfig":{"NanoCpus":1500000000,"Memory":536870912},"State":{"Status":"running"}}]"#).await;
    let requests = app.invoke(Command::ActivateDetailView(4));
    drive(&mut app, &runtime, requests).await;
    press(&mut app, "enter");
    press(&mut app, "ctrl+u");
    press(&mut app, "-");
    press(&mut app, "1");
    press(&mut app, "enter");
    assert!(press(&mut app, "ctrl+a").is_empty());
    let screen = render_to_text(app.state(), 160, 32);
    assert!(
        screen.contains("CPU limit must be a finite number"),
        "{screen}"
    );
    assert!(screen.contains("1.5 → -1"), "{screen}");
}

#[tokio::test]
async fn apply_requires_review_of_old_and_new_values_and_can_be_cancelled() {
    let (mut app, runtime) = docker_app(r#"[{"HostConfig":{"NanoCpus":1500000000,"Memory":536870912},"State":{"Status":"running"}}]"#).await;
    let requests = app.invoke(Command::ActivateDetailView(4));
    drive(&mut app, &runtime, requests).await;
    press(&mut app, "enter");
    press(&mut app, "ctrl+u");
    press(&mut app, "2");
    press(&mut app, "enter");
    assert!(press(&mut app, "ctrl+a").is_empty());
    let screen = render_to_text(app.state(), 160, 32);
    assert!(screen.contains("Confirm configuration"), "{screen}");
    assert!(screen.contains("CPU limit (CPUs): 1.5 → 2"), "{screen}");
    assert!(screen.contains("no restart required"), "{screen}");
    assert!(press(&mut app, "esc").is_empty());
    assert!(app.state().confirmation.is_none());
    assert!(render_to_text(app.state(), 160, 32).contains("1.5 → 2"));
}

fn inspect_response(
    body: &str,
) -> (
    ProcessSpec,
    Result<
        tuivir::infrastructure::process::ProcessOutput,
        tuivir::infrastructure::process::ProcessError,
    >,
) {
    (
        ProcessSpec::new("docker", &["container", "inspect", "a1b2c3d4e5f6"]),
        success(body),
    )
}
fn docker_refresh() -> Vec<(
    ProcessSpec,
    Result<
        tuivir::infrastructure::process::ProcessOutput,
        tuivir::infrastructure::process::ProcessError,
    >,
)> {
    vec![
        (
            ProcessSpec::new(
                "docker",
                &[
                    "container",
                    "ls",
                    "--all",
                    "--no-trunc",
                    "--format",
                    "{{json .}}",
                ],
            ),
            success(include_str!("fixtures/docker/containers.jsonl")),
        ),
        (
            ProcessSpec::new(
                "docker",
                &["image", "ls", "--no-trunc", "--format", "{{json .}}"],
            ),
            success(""),
        ),
        (
            ProcessSpec::new("docker", &["volume", "ls", "--format", "{{json .}}"]),
            success(""),
        ),
    ]
}
const DOCKER_INITIAL: &str = r#"[{"HostConfig":{"NanoCpus":1500000000,"Memory":536870912,"MemorySwap":1073741824},"State":{"Status":"running"}}]"#;
const DOCKER_UPDATED: &str = r#"[{"HostConfig":{"NanoCpus":2000000000,"Memory":536870912,"MemorySwap":1073741824},"State":{"Status":"running"}}]"#;
async fn edit_cpu(app: &mut App, runtime: &ProviderRuntime, value: &str) {
    let requests = app.invoke(Command::ActivateDetailView(4));
    drive(app, runtime, requests).await;
    press(app, "enter");
    press(app, "ctrl+u");
    for character in value.chars() {
        assert!(press(app, &character.to_string()).is_empty());
    }
    press(app, "enter");
}
#[tokio::test]
async fn confirmed_docker_cpu_updates_the_same_container_and_cleans_the_draft() {
    let mut extra = vec![
        inspect_response(DOCKER_INITIAL),
        (
            ProcessSpec::new(
                "docker",
                &["container", "update", "--cpus", "2", "a1b2c3d4e5f6"],
            ),
            success("a1b2c3d4e5f6"),
        ),
        inspect_response(DOCKER_UPDATED),
    ];
    extra.extend(docker_refresh());
    let (mut app, runtime, cli) = docker_app_with(DOCKER_INITIAL, extra).await;
    edit_cpu(&mut app, &runtime, "2").await;
    press(&mut app, "ctrl+a");
    let requests = press(&mut app, "enter");
    assert!(!requests.is_empty(), "confirmation must dispatch the write");
    drive(&mut app, &runtime, requests).await;
    let screen = render_to_text(app.state(), 160, 32);
    assert!(screen.contains("CPU limit (CPUs): 2"), "{screen}");
    assert!(!screen.contains("1.5 → 2"), "{screen}");
    cli.assert_exhausted();
}

#[tokio::test]
async fn confirmed_docker_memory_updates_without_changing_cpu_or_restarting() {
    let updated = r#"[{"HostConfig":{"NanoCpus":1500000000,"Memory":805306368,"MemorySwap":1073741824},"State":{"Status":"running"}}]"#;
    let mut extra = vec![
        inspect_response(DOCKER_INITIAL),
        (
            ProcessSpec::new(
                "docker",
                &[
                    "container",
                    "update",
                    "--memory",
                    "805306368",
                    "a1b2c3d4e5f6",
                ],
            ),
            success("updated"),
        ),
        inspect_response(updated),
    ];
    extra.extend(docker_refresh());
    let (mut app, runtime, cli) = docker_app_with(DOCKER_INITIAL, extra).await;
    edit_cpu(&mut app, &runtime, "1.5").await;
    press(&mut app, "down");
    press(&mut app, "enter");
    press(&mut app, "ctrl+u");
    for c in "805306368".chars() {
        press(&mut app, &c.to_string());
    }
    press(&mut app, "enter");
    press(&mut app, "ctrl+a");
    let requests = press(&mut app, "enter");
    drive(&mut app, &runtime, requests).await;
    let screen = render_to_text(app.state(), 160, 32);
    assert!(
        screen.contains("Memory limit (bytes): 805306368"),
        "{screen}"
    );
    assert!(screen.contains("CPU limit (CPUs): 1.5"), "{screen}");
    cli.assert_exhausted();
}

#[tokio::test]
async fn external_changes_to_an_edited_field_require_another_review_without_writing() {
    let external = r#"[{"HostConfig":{"NanoCpus":3000000000,"Memory":536870912},"State":{"Status":"running"}}]"#;
    let mut extra = vec![inspect_response(external)];
    extra.extend(docker_refresh());
    let (mut app, runtime, cli) = docker_app_with(DOCKER_INITIAL, extra).await;
    edit_cpu(&mut app, &runtime, "2").await;
    press(&mut app, "ctrl+a");
    let requests = press(&mut app, "enter");
    drive(&mut app, &runtime, requests).await;
    let screen = render_to_text(app.state(), 160, 32);
    assert!(screen.contains("changed externally"), "{screen}");
    assert!(screen.contains("3 → 2"), "{screen}");
    press(&mut app, "ctrl+a");
    assert!(render_to_text(app.state(), 160, 32).contains("CPU limit (CPUs): 3 → 2"));
    cli.assert_exhausted();
}

#[tokio::test]
async fn successful_apply_reconciles_equivalent_cpu_input_and_unedited_external_memory() {
    let fresh = r#"[{"HostConfig":{"NanoCpus":1500000000,"Memory":805306368},"State":{"Status":"running"}}]"#;
    let updated = r#"[{"HostConfig":{"NanoCpus":2000000000,"Memory":805306368},"State":{"Status":"running"}}]"#;
    let mut extra = vec![
        inspect_response(fresh),
        (
            ProcessSpec::new(
                "docker",
                &["container", "update", "--cpus", "2.0", "a1b2c3d4e5f6"],
            ),
            success("updated"),
        ),
        inspect_response(updated),
    ];
    extra.extend(docker_refresh());
    let (mut app, runtime, cli) = docker_app_with(DOCKER_INITIAL, extra).await;
    edit_cpu(&mut app, &runtime, "2.0").await;
    press(&mut app, "ctrl+a");
    let requests = press(&mut app, "enter");
    drive(&mut app, &runtime, requests).await;
    let screen = render_to_text(app.state(), 160, 32);
    assert!(
        !screen.contains('→'),
        "equivalent values and untouched fields must be clean:\n{screen}"
    );
    assert!(
        screen.contains("Memory limit (bytes): 805306368"),
        "{screen}"
    );
    cli.assert_exhausted();
}

async fn incus_app(
    initial: &str,
    extra: Vec<(
        ProcessSpec,
        Result<
            tuivir::infrastructure::process::ProcessOutput,
            tuivir::infrastructure::process::ProcessError,
        >,
    )>,
) -> (App, ProviderRuntime, Arc<FixtureCli>) {
    let cli = Arc::new(FixtureCli::new(
        [
            (
                ProcessSpec::new("incus", &["remote", "get-default"]),
                success("local"),
            ),
            (
                ProcessSpec::new("incus", &["project", "get-current"]),
                success("default"),
            ),
            (
                ProcessSpec::new("incus", &["list", "--format=json"]),
                success(include_str!("fixtures/incus/instances.json")),
            ),
            (
                ProcessSpec::new("incus", &["storage", "list", "--format=json"]),
                success("[]"),
            ),
            (
                ProcessSpec::new("incus", &["list", "api", "--format=json"]),
                success(initial),
            ),
        ]
        .into_iter()
        .chain(extra),
    ));
    let runtime = ProviderRuntime::new(
        vec![Arc::new(tuivir::infrastructure::provider::IncusWorkspace)],
        cli.clone(),
    );
    let mut app = App::new();
    let requests = app.update(runtime.discover().await.remove(0).into_event());
    drive(&mut app, &runtime, requests).await;
    (app, runtime, cli)
}

#[tokio::test]
async fn incus_configuration_displays_effective_limits_with_provider_units() {
    let initial = r#"[{"name":"api","type":"container","status":"Running","expanded_config":{"limits.cpu":"0-1,3","limits.memory":"50%"}}]"#;
    let (mut app, runtime, cli) = incus_app(initial, vec![]).await;
    let requests = app.invoke(Command::ActivateDetailView(4));
    drive(&mut app, &runtime, requests).await;
    let screen = render_to_text(app.state(), 160, 32);
    assert!(screen.contains("CPU count or IDs: 0-1,3"), "{screen}");
    assert!(
        screen.contains("Memory limit (bytes, units or %): 50%"),
        "{screen}"
    );
    assert!(screen.contains("selected instance"), "{screen}");
    cli.assert_exhausted();
}
