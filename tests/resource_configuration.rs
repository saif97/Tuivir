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
    incus_app_with_capabilities(
        initial,
        extra,
        &[
            "cpu_hotplug",
            "memory_hotplug",
            "limits_memory_hotplug",
            "instance_limits_cpu_topology",
        ],
    )
    .await
}

async fn incus_app_with_capabilities(
    initial: &str,
    extra: Vec<(
        ProcessSpec,
        Result<
            tuivir::infrastructure::process::ProcessOutput,
            tuivir::infrastructure::process::ProcessError,
        >,
    )>,
    capabilities: &[&str],
) -> (App, ProviderRuntime, Arc<FixtureCli>) {
    let mut responses = ([
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
    .chain(extra))
    .collect::<Vec<_>>();
    let reads = responses
        .iter()
        .filter(|(spec, _)| {
            spec.program == "incus" && spec.args.len() == 3 && spec.args[0] == "list"
        })
        .count();
    let server = serde_json::json!({"api_extensions": capabilities}).to_string();
    responses.extend((0..reads).map(|_| {
        (
            ProcessSpec::new("incus", &["query", "/1.0"]),
            success(&server),
        )
    }));
    let cli = Arc::new(FixtureCli::new(responses));
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

fn incus_read(
    body: &str,
) -> (
    ProcessSpec,
    Result<
        tuivir::infrastructure::process::ProcessOutput,
        tuivir::infrastructure::process::ProcessError,
    >,
) {
    (
        ProcessSpec::new("incus", &["list", "api", "--format=json"]),
        success(body),
    )
}
fn incus_refresh() -> Vec<(
    ProcessSpec,
    Result<
        tuivir::infrastructure::process::ProcessOutput,
        tuivir::infrastructure::process::ProcessError,
    >,
)> {
    vec![
        (
            ProcessSpec::new("incus", &["list", "--format=json"]),
            success(include_str!("fixtures/incus/instances.json")),
        ),
        (
            ProcessSpec::new("incus", &["storage", "list", "--format=json"]),
            success("[]"),
        ),
    ]
}
#[tokio::test]
async fn incus_partial_success_cleans_only_the_confirmed_field_and_keeps_the_error() {
    let initial = r#"[{"name":"api","type":"container","status":"Running","expanded_config":{"limits.cpu":"2","limits.memory":"512MiB"}}]"#;
    let partial = r#"[{"name":"api","type":"container","status":"Running","expanded_config":{"limits.cpu":"4","limits.memory":"512MiB"}}]"#;
    let mut extra = vec![
        incus_read(initial),
        (
            ProcessSpec::new("incus", &["config", "set", "api", "limits.cpu=4"]),
            success(""),
        ),
        (
            ProcessSpec::new("incus", &["config", "set", "api", "limits.memory=1GiB"]),
            common::failure("memory limit rejected"),
        ),
        incus_read(partial),
    ];
    extra.extend(incus_refresh());
    let (mut app, runtime, cli) = incus_app(initial, extra).await;
    edit_cpu(&mut app, &runtime, "4").await;
    press(&mut app, "down");
    press(&mut app, "enter");
    press(&mut app, "ctrl+u");
    for c in "1GiB".chars() {
        press(&mut app, &c.to_string());
    }
    press(&mut app, "enter");
    press(&mut app, "ctrl+a");
    let requests = press(&mut app, "enter");
    drive(&mut app, &runtime, requests).await;
    let screen = render_to_text(app.state(), 160, 32);
    assert!(screen.contains("CPU count or IDs: 4"), "{screen}");
    assert!(!screen.contains("2 → 4"), "{screen}");
    assert!(screen.contains("512MiB → 1GiB"), "{screen}");
    assert!(screen.contains("memory limit rejected"), "{screen}");
    cli.assert_exhausted();
}

#[tokio::test]
async fn incus_vm_topology_change_discloses_downtime_and_restores_running_state() {
    let initial = r#"[{"name":"api","type":"virtual-machine","status":"Running","expanded_config":{"limits.cpu":"sockets=1,cores=2","limits.memory":"512MiB"}}]"#;
    let updated = r#"[{"name":"api","type":"virtual-machine","status":"Running","expanded_config":{"limits.cpu":"4","limits.memory":"512MiB"}}]"#;
    let mut extra = vec![
        incus_read(initial),
        (ProcessSpec::new("incus", &["stop", "api"]), success("")),
        (
            ProcessSpec::new("incus", &["config", "set", "api", "limits.cpu=4"]),
            success(""),
        ),
        (ProcessSpec::new("incus", &["start", "api"]), success("")),
        incus_read(updated),
    ];
    extra.extend(incus_refresh());
    let (mut app, runtime, cli) = incus_app(initial, extra).await;
    edit_cpu(&mut app, &runtime, "4").await;
    press(&mut app, "ctrl+a");
    let screen = render_to_text(app.state(), 180, 36);
    assert!(screen.contains("Downtime: stop/start"), "{screen}");
    let requests = press(&mut app, "enter");
    drive(&mut app, &runtime, requests).await;
    assert!(render_to_text(app.state(), 180, 36).contains("Resource State: Running"));
    cli.assert_exhausted();
}

#[tokio::test]
async fn sandbox_configuration_explains_read_only_limits_without_writes() {
    let cli = Arc::new(FixtureCli::new([
        (
            ProcessSpec::new("sbx", &["version"]),
            success("sbx version: v0.37.0"),
        ),
        (
            ProcessSpec::new("sbx", &["ls", "--json"]),
            success(include_str!("fixtures/docker-sandbox/sandboxes.json")),
        ),
        (
            ProcessSpec::new("sbx", &["ls", "--json"]),
            success(include_str!("fixtures/docker-sandbox/sandboxes.json")),
        ),
    ]));
    let runtime = ProviderRuntime::new(
        vec![Arc::new(
            tuivir::infrastructure::provider::DockerSandboxWorkspace,
        )],
        cli.clone(),
    );
    let mut app = App::new();
    let requests = app.update(runtime.discover().await.remove(0).into_event());
    drive(&mut app, &runtime, requests).await;
    let requests = app.invoke(Command::ActivateDetailView(2));
    drive(&mut app, &runtime, requests).await;
    let screen = render_to_text(app.state(), 180, 36);
    assert!(
        screen.contains("in-place CPU/memory resize is unsupported"),
        "{screen}"
    );
    assert!(
        screen.contains("CPU count: not reported (read-only)"),
        "{screen}"
    );
    assert!(press(&mut app, "enter").is_empty());
    assert!(press(&mut app, "ctrl+a").is_empty());
    assert!(app.state().confirmation.is_none());
    cli.assert_exhausted();
}

#[tokio::test]
async fn incus_restart_failure_reports_saved_settings_and_refreshed_stopped_state() {
    let initial = r#"[{"name":"api","type":"virtual-machine","status":"Running","expanded_config":{"limits.cpu":"sockets=1,cores=2","limits.memory":"512MiB"}}]"#;
    let stopped = r#"[{"name":"api","type":"virtual-machine","status":"Stopped","expanded_config":{"limits.cpu":"4","limits.memory":"512MiB"}}]"#;
    let mut extra = vec![
        incus_read(initial),
        (ProcessSpec::new("incus", &["stop", "api"]), success("")),
        (
            ProcessSpec::new("incus", &["config", "set", "api", "limits.cpu=4"]),
            success(""),
        ),
        (
            ProcessSpec::new("incus", &["start", "api"]),
            common::failure("guest failed to boot"),
        ),
        incus_read(stopped),
    ];
    extra.extend(incus_refresh());
    let (mut app, runtime, cli) = incus_app(initial, extra).await;
    edit_cpu(&mut app, &runtime, "4").await;
    press(&mut app, "ctrl+a");
    let requests = press(&mut app, "enter");
    drive(&mut app, &runtime, requests).await;
    let screen = render_to_text(app.state(), 180, 36);
    assert!(
        screen.contains("Settings saved, but restart failed"),
        "{screen}"
    );
    assert!(screen.contains("guest failed to boot"), "{screen}");
    assert!(screen.contains("Resource State: Stopped"), "{screen}");
    assert!(!screen.contains('→'), "{screen}");
    cli.assert_exhausted();
}

#[tokio::test]
async fn docker_update_failure_retains_attempted_values_alongside_actual_values() {
    let mut extra = vec![
        inspect_response(DOCKER_INITIAL),
        (
            ProcessSpec::new(
                "docker",
                &["container", "update", "--cpus", "2", "a1b2c3d4e5f6"],
            ),
            common::failure("CPU limit rejected"),
        ),
        inspect_response(DOCKER_INITIAL),
    ];
    extra.extend(docker_refresh());
    let (mut app, runtime, cli) = docker_app_with(DOCKER_INITIAL, extra).await;
    edit_cpu(&mut app, &runtime, "2").await;
    press(&mut app, "ctrl+a");
    let requests = press(&mut app, "enter");
    drive(&mut app, &runtime, requests).await;
    let screen = render_to_text(app.state(), 160, 32);
    assert!(screen.contains("CPU limit rejected"), "{screen}");
    assert!(screen.contains("1.5 → 2"), "{screen}");
    cli.assert_exhausted();
}
#[tokio::test]
async fn a_stopped_incus_vm_stays_stopped_after_configuration_changes() {
    let initial = r#"[{"name":"api","type":"virtual-machine","status":"Stopped","expanded_config":{"limits.cpu":"sockets=1,cores=2","limits.memory":"512MiB"}}]"#;
    let updated = r#"[{"name":"api","type":"virtual-machine","status":"Stopped","expanded_config":{"limits.cpu":"4","limits.memory":"512MiB"}}]"#;
    let mut extra = vec![
        incus_read(initial),
        (
            ProcessSpec::new("incus", &["config", "set", "api", "limits.cpu=4"]),
            success(""),
        ),
        incus_read(updated),
    ];
    extra.extend(incus_refresh());
    let (mut app, runtime, cli) = incus_app(initial, extra).await;
    edit_cpu(&mut app, &runtime, "4").await;
    press(&mut app, "ctrl+a");
    assert!(render_to_text(app.state(), 180, 36).contains("Resource remains stopped"));
    let requests = press(&mut app, "enter");
    drive(&mut app, &runtime, requests).await;
    assert!(render_to_text(app.state(), 180, 36).contains("Resource State: Stopped"));
    cli.assert_exhausted();
}

#[tokio::test]
async fn refresh_retries_a_failed_configuration_load() {
    let mut extra = vec![inspect_response(DOCKER_INITIAL)];
    extra.extend(docker_refresh());
    let (mut app, runtime, cli) = docker_app_with("not json", extra).await;
    let requests = app.invoke(Command::ActivateDetailView(4));
    drive(&mut app, &runtime, requests).await;
    assert!(render_to_text(app.state(), 160, 32).contains("Malformed Docker configuration"));
    let requests = press(&mut app, "ctrl+r");
    drive(&mut app, &runtime, requests).await;
    let screen = render_to_text(app.state(), 160, 32);
    assert!(screen.contains("CPU limit (CPUs): 1.5"), "{screen}");
    assert!(
        !screen.contains("Malformed Docker configuration"),
        "{screen}"
    );
    cli.assert_exhausted();
}

#[tokio::test]
async fn docker_memory_validation_explains_the_existing_memory_swap_ceiling() {
    let (mut app, runtime) = docker_app(DOCKER_INITIAL).await;
    edit_cpu(&mut app, &runtime, "1.5").await;
    press(&mut app, "down");
    press(&mut app, "enter");
    press(&mut app, "ctrl+u");
    for c in "2147483648".chars() {
        press(&mut app, &c.to_string());
    }
    press(&mut app, "enter");
    assert!(press(&mut app, "ctrl+a").is_empty());
    assert!(app.state().confirmation.is_none());
    let screen = render_to_text(app.state(), 180, 36);
    assert!(
        screen.contains("existing memory+swap limit (1073741824 bytes)"),
        "{screen}"
    );
}

#[tokio::test]
async fn dispatched_configuration_stays_with_its_original_resource_and_freezes_its_draft() {
    let mut extra = vec![
        inspect_response(DOCKER_INITIAL),
        (
            ProcessSpec::new(
                "docker",
                &["container", "update", "--cpus", "2", "a1b2c3d4e5f6"],
            ),
            success("updated"),
        ),
        inspect_response(DOCKER_UPDATED),
    ];
    extra.extend(docker_refresh());
    let (mut app, runtime, cli) = docker_app_with(DOCKER_INITIAL, extra).await;
    edit_cpu(&mut app, &runtime, "2").await;
    press(&mut app, "ctrl+a");
    let requests = press(&mut app, "enter");
    press(&mut app, "enter");
    assert!(!render_to_text(app.state(), 180, 36).contains("Editing draft"));
    app.invoke(Command::FocusResourcePanel(0));
    app.invoke(Command::SelectNext);
    let screen = render_to_text(app.state(), 180, 36);
    assert!(
        screen.contains("Applying configuration to Docker / api"),
        "{screen}"
    );
    drive(&mut app, &runtime, requests).await;
    app.invoke(Command::SelectPrevious);
    app.invoke(Command::FocusDetails);
    let screen = render_to_text(app.state(), 180, 36);
    assert!(screen.contains("CPU limit (CPUs): 2"), "{screen}");
    cli.assert_exhausted();
}

#[tokio::test]
async fn configuration_form_keeps_contextual_help_and_detail_tab_navigation() {
    let (mut app, runtime) = docker_app(DOCKER_INITIAL).await;
    let requests = app.invoke(Command::ActivateDetailView(4));
    drive(&mut app, &runtime, requests).await;
    press(&mut app, "?");
    let screen = render_to_text(app.state(), 180, 36);
    assert!(screen.contains("Apply Configuration Draft"), "{screen}");
    assert!(screen.contains("Edit selected field"), "{screen}");
    press(&mut app, "esc");
    assert!(press(&mut app, "right").is_empty());
    assert!(render_to_text(app.state(), 180, 36).contains("[ Shell ]"));
}

#[tokio::test]
async fn a_running_ephemeral_vm_cannot_apply_a_change_that_would_delete_it() {
    let initial = r#"[{"name":"api","type":"virtual-machine","status":"Running","ephemeral":true,"expanded_config":{"limits.cpu":"sockets=1,cores=2","limits.memory":"512MiB"}}]"#;
    let (mut app, runtime, cli) = incus_app(initial, vec![]).await;
    edit_cpu(&mut app, &runtime, "4").await;
    assert!(press(&mut app, "ctrl+a").is_empty());
    assert!(app.state().confirmation.is_none());
    let screen = render_to_text(app.state(), 180, 36);
    assert!(
        screen.contains("Stopping this Resource would remove it"),
        "{screen}"
    );
    cli.assert_exhausted();
}

#[tokio::test]
async fn incus_without_memory_hotplug_stops_and_restarts_a_vm_for_memory_changes() {
    let initial = r#"[{"name":"api","type":"virtual-machine","status":"Running","expanded_config":{"limits.cpu":"1","limits.memory":"256MiB"}}]"#;
    let updated = r#"[{"name":"api","type":"virtual-machine","status":"Running","expanded_config":{"limits.cpu":"1","limits.memory":"512MiB"}}]"#;
    let mut extra = vec![
        incus_read(initial),
        (ProcessSpec::new("incus", &["stop", "api"]), success("")),
        (
            ProcessSpec::new("incus", &["config", "set", "api", "limits.memory=512MiB"]),
            success(""),
        ),
        (ProcessSpec::new("incus", &["start", "api"]), success("")),
        incus_read(updated),
    ];
    extra.extend(incus_refresh());
    let (mut app, runtime, cli) =
        incus_app_with_capabilities(initial, extra, &["cpu_hotplug"]).await;
    edit_cpu(&mut app, &runtime, "1").await;
    press(&mut app, "down");
    press(&mut app, "enter");
    press(&mut app, "ctrl+u");
    for c in "512MiB".chars() {
        press(&mut app, &c.to_string());
    }
    press(&mut app, "enter");
    press(&mut app, "ctrl+a");
    assert!(render_to_text(app.state(), 180, 36).contains("Downtime: stop/start"));
    let requests = press(&mut app, "enter");
    drive(&mut app, &runtime, requests).await;
    assert!(render_to_text(app.state(), 180, 36).contains("Resource State: Running"));
    cli.assert_exhausted();
}

#[tokio::test]
async fn fractional_incus_memory_percentages_are_rejected_before_apply() {
    let initial = r#"[{"name":"api","type":"container","status":"Running","expanded_config":{"limits.cpu":"2","limits.memory":"50%"}}]"#;
    let (mut app, runtime, cli) = incus_app(initial, vec![]).await;
    edit_cpu(&mut app, &runtime, "2").await;
    press(&mut app, "down");
    press(&mut app, "enter");
    press(&mut app, "ctrl+u");
    for c in "12.5%".chars() {
        press(&mut app, &c.to_string());
    }
    press(&mut app, "enter");
    assert!(press(&mut app, "ctrl+a").is_empty());
    assert!(app.state().confirmation.is_none());
    assert!(render_to_text(app.state(), 180, 36).contains("whole percentage"));
    cli.assert_exhausted();
}

#[tokio::test]
async fn huge_page_vm_memory_changes_disclose_downtime_even_with_hotplug_support() {
    let initial = r#"[{"name":"api","type":"virtual-machine","status":"Running","expanded_config":{"limits.cpu":"2","limits.memory":"256MiB","limits.memory.hugepages":"true"}}]"#;
    let updated = r#"[{"name":"api","type":"virtual-machine","status":"Running","expanded_config":{"limits.cpu":"2","limits.memory":"512MiB","limits.memory.hugepages":"true"}}]"#;
    let mut extra = vec![
        incus_read(initial),
        (ProcessSpec::new("incus", &["stop", "api"]), success("")),
        (
            ProcessSpec::new("incus", &["config", "set", "api", "limits.memory=512MiB"]),
            success(""),
        ),
        (ProcessSpec::new("incus", &["start", "api"]), success("")),
        incus_read(updated),
    ];
    extra.extend(incus_refresh());
    let (mut app, runtime, cli) = incus_app(initial, extra).await;
    edit_cpu(&mut app, &runtime, "2").await;
    press(&mut app, "down");
    press(&mut app, "enter");
    press(&mut app, "ctrl+u");
    for c in "512MiB".chars() {
        press(&mut app, &c.to_string());
    }
    press(&mut app, "enter");
    press(&mut app, "ctrl+a");
    assert!(render_to_text(app.state(), 180, 36).contains("Downtime: stop/start"));
    let requests = press(&mut app, "enter");
    drive(&mut app, &runtime, requests).await;
    cli.assert_exhausted();
}

#[tokio::test]
async fn existing_docker_cpu_quota_limits_are_loaded_and_updated_in_their_native_mode() {
    let initial = r#"[{"HostConfig":{"NanoCpus":0,"CpuQuota":50000,"CpuPeriod":100000,"Memory":536870912},"State":{"Status":"running"}}]"#;
    let updated = r#"[{"HostConfig":{"NanoCpus":0,"CpuQuota":100000,"CpuPeriod":100000,"Memory":536870912},"State":{"Status":"running"}}]"#;
    let mut extra = vec![
        inspect_response(initial),
        (
            ProcessSpec::new(
                "docker",
                &[
                    "container",
                    "update",
                    "--cpu-period",
                    "100000",
                    "--cpu-quota",
                    "100000",
                    "a1b2c3d4e5f6",
                ],
            ),
            success("updated"),
        ),
        inspect_response(updated),
    ];
    extra.extend(docker_refresh());
    let (mut app, runtime, cli) = docker_app_with(initial, extra).await;
    let requests = app.invoke(Command::ActivateDetailView(4));
    drive(&mut app, &runtime, requests).await;
    let screen = render_to_text(app.state(), 180, 36);
    assert!(screen.contains("CPU limit (CPUs): 0.5"), "{screen}");
    edit_cpu(&mut app, &runtime, "1").await;
    press(&mut app, "ctrl+a");
    let requests = press(&mut app, "enter");
    drive(&mut app, &runtime, requests).await;
    assert!(render_to_text(app.state(), 180, 36).contains("CPU limit (CPUs): 1"));
    cli.assert_exhausted();
}
