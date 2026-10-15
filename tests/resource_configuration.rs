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

fn open_settings(app: &mut App) -> Vec<ProviderRequest> {
    let tuivir::application::WorkspacePresentation::Ready(view) =
        app.state().active_workspace().unwrap().presentation()
    else {
        panic!("Workspace not loaded")
    };
    let index = view
        .detail_views
        .iter()
        .position(|view| view.is_configuration())
        .unwrap()
        + 1;
    app.invoke(Command::ActivateDetailView(index))
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
    docker_app_with_registry(
        inspect,
        extra,
        tuivir::application::CommandRegistry::builtin(),
    )
    .await
}

async fn docker_app_with_registry(
    inspect: &str,
    extra: Vec<(
        ProcessSpec,
        Result<
            tuivir::infrastructure::process::ProcessOutput,
            tuivir::infrastructure::process::ProcessError,
        >,
    )>,
    registry: tuivir::application::CommandRegistry,
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
    let mut app = App::with_registry(registry);
    let discovery = runtime.discover().await.remove(0);
    let requests = app.update(discovery.into_event());
    drive(&mut app, &runtime, requests).await;
    (app, runtime, cli)
}

#[tokio::test]
async fn configuration_tab_displays_current_docker_limits_with_units() {
    let (mut app, runtime) = docker_app(r#"[{"HostConfig":{"NanoCpus":1500000000,"Memory":536870912,"MemorySwap":1073741824},"State":{"Status":"running"}}]"#).await;
    app.invoke(Command::FocusDetails);
    let requests = open_settings(&mut app);
    drive(&mut app, &runtime, requests).await;
    let screen = render_to_text(app.state(), 140, 32);
    assert!(screen.contains("CPU limit (CPUs): 1.5"), "{screen}");
    assert!(screen.contains("Memory limit (GiB): 0.5"), "{screen}");
    assert!(screen.contains("Review"), "{screen}");
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
    let requests = open_settings(&mut app);
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
    let requests = open_settings(&mut app);
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
    let requests = open_settings(&mut app);
    drive(&mut app, &runtime, requests).await;
    press(&mut app, "enter");
    press(&mut app, "ctrl+u");
    press(&mut app, "2");
    press(&mut app, "enter");
    assert!(press(&mut app, "ctrl+a").is_empty());
    let screen = render_to_text(app.state(), 160, 32);
    assert!(screen.contains("Review changes"), "{screen}");
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
    let requests = open_settings(app);
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
    for c in "0.75".chars() {
        press(&mut app, &c.to_string());
    }
    press(&mut app, "enter");
    press(&mut app, "ctrl+a");
    let requests = press(&mut app, "enter");
    drive(&mut app, &runtime, requests).await;
    let screen = render_to_text(app.state(), 160, 32);
    assert!(screen.contains("Memory limit (GiB): 0.75"), "{screen}");
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
    assert!(screen.contains("Memory limit (GiB): 0.75"), "{screen}");
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
    let requests = open_settings(&mut app);
    drive(&mut app, &runtime, requests).await;
    let screen = render_to_text(app.state(), 160, 32);
    assert!(screen.contains("CPU count or IDs: 0-1,3"), "{screen}");
    assert!(screen.contains("Memory limit (GiB or %): 50%"), "{screen}");
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
            ProcessSpec::new(
                "incus",
                &["config", "set", "api", "limits.memory=1073741824"],
            ),
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
    for c in "1".chars() {
        press(&mut app, &c.to_string());
    }
    press(&mut app, "enter");
    press(&mut app, "ctrl+a");
    let requests = press(&mut app, "enter");
    drive(&mut app, &runtime, requests).await;
    let screen = render_to_text(app.state(), 160, 32);
    assert!(screen.contains("CPU count or IDs: 4"), "{screen}");
    assert!(!screen.contains("2 → 4"), "{screen}");
    assert!(screen.contains("0.5 → 1"), "{screen}");
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
            success(
                &include_str!("fixtures/docker-sandbox/sandboxes.json").replace(
                    "\"protocol\": \"tcp\"",
                    "\"protocol\": \"tcp\", \"description\": \"forwarded\"",
                ),
            ),
        ),
        (
            ProcessSpec::new("sbx", &["ls", "--json"]),
            success(
                &include_str!("fixtures/docker-sandbox/sandboxes.json").replace(
                    "\"protocol\": \"tcp\"",
                    "\"protocol\": \"tcp\", \"description\": \"forwarded\"",
                ),
            ),
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
    let text = screen.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        screen.contains("in-place CPU/memory resize is unsupported"),
        "{screen}"
    );
    assert!(text.contains("CPU count: not reported"), "{screen}");
    assert!(text.contains("agent: claude"), "{screen}");
    assert!(screen.contains("/home/vibebox/projects/tuivir"), "{screen}");
    assert!(
        app.state()
            .selected_configuration()
            .unwrap()
            .actual
            .as_ref()
            .unwrap()
            .fields
            .iter()
            .any(|field| field.label == "ports" && field.value.contains("forwarded"))
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
    let requests = open_settings(&mut app);
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
    for c in "2".chars() {
        press(&mut app, &c.to_string());
    }
    press(&mut app, "enter");
    assert!(press(&mut app, "ctrl+a").is_empty());
    assert!(app.state().confirmation.is_none());
    let screen = render_to_text(app.state(), 180, 36);
    assert!(
        screen.contains("existing Provider limit (1 GiB)"),
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
    let requests = open_settings(&mut app);
    drive(&mut app, &runtime, requests).await;
    press(&mut app, "?");
    let screen = render_to_text(app.state(), 180, 36);
    assert!(screen.contains("Review & apply"), "{screen}");
    assert!(screen.contains("Edit field"), "{screen}");
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
            ProcessSpec::new(
                "incus",
                &["config", "set", "api", "limits.memory=536870912"],
            ),
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
    for c in "0.5".chars() {
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
    for (key, value) in [
        ("limits.memory.hugepages", "true"),
        ("limits.memory.hugepages", "YES"),
        ("limits.memory.hugepages", "1"),
        ("limits.memory.hugepages", "on"),
        ("limits.memory.hotplug", "false"),
        ("limits.memory.hotplug", "NO"),
        ("limits.memory.hotplug", "0"),
        ("limits.memory.hotplug", "off"),
    ] {
        let initial = r#"[{"name":"api","type":"virtual-machine","status":"Running","expanded_config":{"limits.cpu":"2","limits.memory":"256MiB","limits.memory.hugepages":"true"}}]"#;
        let updated = r#"[{"name":"api","type":"virtual-machine","status":"Running","expanded_config":{"limits.cpu":"2","limits.memory":"512MiB","limits.memory.hugepages":"true"}}]"#;
        let initial = initial
            .replace("limits.memory.hugepages", key)
            .replace("true", value);
        let updated = updated
            .replace("limits.memory.hugepages", key)
            .replace("true", value);
        let initial = initial.as_str();
        let updated = updated.as_str();
        let mut extra = vec![
            incus_read(initial),
            (ProcessSpec::new("incus", &["stop", "api"]), success("")),
            (
                ProcessSpec::new(
                    "incus",
                    &["config", "set", "api", "limits.memory=536870912"],
                ),
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
        for c in "0.5".chars() {
            press(&mut app, &c.to_string());
        }
        press(&mut app, "enter");
        press(&mut app, "ctrl+a");
        assert!(render_to_text(app.state(), 180, 36).contains("Downtime: stop/start"));
        let requests = press(&mut app, "enter");
        drive(&mut app, &runtime, requests).await;
        cli.assert_exhausted();
    }
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
    let requests = open_settings(&mut app);
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

#[tokio::test]
async fn configuration_instructions_follow_effective_keybindings() {
    let registry = tuivir::application::CommandRegistry::effective(&[
        ("configuration_edit".into(), vec!["e".into()]),
        ("configuration_apply".into(), vec!["alt+a".into()]),
        (
            "configuration_finish_field".into(),
            vec!["alt+enter".into()],
        ),
        ("configuration_clear_field".into(), vec!["alt+u".into()]),
    ])
    .unwrap();
    let (mut app, runtime, cli) = docker_app_with_registry(DOCKER_INITIAL, vec![], registry).await;
    let requests = open_settings(&mut app);
    drive(&mut app, &runtime, requests).await;
    let screen = render_to_text(app.state(), 180, 36);
    assert!(!screen.contains("ctrl+a Apply"), "{screen}");
    assert!(!screen.contains("Enter edits"), "{screen}");
    assert!(screen.contains("alt+a"), "{screen}");
    press(&mut app, "e");
    let screen = render_to_text(app.state(), 180, 36);
    assert!(screen.contains("alt+enter"), "{screen}");
    assert!(screen.contains("alt+u"), "{screen}");
    assert!(!screen.contains("ctrl+u clears"), "{screen}");
    cli.assert_exhausted();
}

const TART_INITIAL: &str = r#"{"CPU":2,"Memory":4096,"Running":true,"State":"running"}"#;
const TART_LIST: &str =
    r#"[{"Source":"local","Name":"mac","Size":20,"Accessed":"now","State":"running"}]"#;
type CliResponse = (
    ProcessSpec,
    Result<
        tuivir::infrastructure::process::ProcessOutput,
        tuivir::infrastructure::process::ProcessError,
    >,
);

fn tart_read(json: &str) -> CliResponse {
    (
        ProcessSpec::new("tart", &["get", "--format", "json", "--", "mac"]),
        success(json),
    )
}

async fn tart_app(
    initial: &str,
    extra: Vec<CliResponse>,
) -> (App, ProviderRuntime, Arc<FixtureCli>) {
    let cli = Arc::new(FixtureCli::new(
        [
            (ProcessSpec::new("tart", &["--version"]), success("2.31.0")),
            (
                ProcessSpec::new("tart", &["list", "--format", "json"]),
                success(TART_LIST),
            ),
            tart_read(initial),
        ]
        .into_iter()
        .chain(extra),
    ));
    let runtime = ProviderRuntime::new(
        vec![Arc::new(tuivir::infrastructure::provider::TartWorkspace)],
        cli.clone(),
    );
    let mut app = App::new();
    let requests = app.update(runtime.discover().await.remove(0).into_event());
    drive(&mut app, &runtime, requests).await;
    let requests = app.invoke(Command::ActivateDetailView(1));
    drive(&mut app, &runtime, requests).await;
    (app, runtime, cli)
}

#[tokio::test]
async fn tart_configuration_displays_cpu_count_and_memory_in_gib() {
    let (app, _, cli) = tart_app(TART_INITIAL, vec![]).await;
    let screen = render_to_text(app.state(), 180, 36);
    assert!(screen.contains("CPU count: 2"), "{screen}");
    assert!(screen.contains("Memory (GiB): 4"), "{screen}");
    cli.assert_exhausted();
}

fn edit_selected_field(app: &mut App, value: &str) {
    press(app, "enter");
    press(app, "ctrl+u");
    for character in value.chars() {
        assert!(press(app, &character.to_string()).is_empty());
    }
    press(app, "enter");
}

#[tokio::test]
async fn tart_apply_reviews_downtime_then_updates_and_restarts_the_same_vm() {
    let updated = r#"{"CPU":4,"Memory":8192,"Running":true,"State":"running"}"#;
    let (mut app, runtime, cli) = tart_app(
        TART_INITIAL,
        vec![
            tart_read(TART_INITIAL),
            (
                ProcessSpec::new("tart", &["stop", "--", "mac"]),
                success(""),
            ),
            (
                ProcessSpec::new(
                    "tart",
                    &["set", "--cpu", "4", "--memory", "8192", "--", "mac"],
                ),
                success(""),
            ),
            (
                ProcessSpec::background("tart", &["run", "--no-graphics", "--", "mac"]),
                success(""),
            ),
            tart_read(updated),
            (
                ProcessSpec::new("tart", &["list", "--format", "json"]),
                success(TART_LIST),
            ),
        ],
    )
    .await;
    edit_selected_field(&mut app, "4");
    press(&mut app, "down");
    edit_selected_field(&mut app, "8");
    press(&mut app, "ctrl+a");
    let screen = render_to_text(app.state(), 180, 36);
    assert!(screen.contains("2 → 4"), "{screen}");
    assert!(screen.contains("4 → 8"), "{screen}");
    assert!(screen.contains("Downtime: stop/start"), "{screen}");
    let requests = press(&mut app, "enter");
    drive(&mut app, &runtime, requests).await;
    let screen = render_to_text(app.state(), 180, 36);
    assert!(screen.contains("CPU count: 4"), "{screen}");
    assert!(screen.contains("Memory (GiB): 8"), "{screen}");
    assert!(screen.contains("Resource State: Running"), "{screen}");
    assert!(!screen.contains('→'), "{screen}");
    cli.assert_exhausted();
}

#[tokio::test]
async fn apple_container_configuration_shows_read_only_values_and_explains_resize_limit() {
    let containers = include_str!("fixtures/apple-container/containers.json");
    let cli = Arc::new(FixtureCli::new([
        (
            ProcessSpec::new("container", &["system", "version", "--format", "json"]),
            success(include_str!("fixtures/apple-container/version.json")),
        ),
        (
            ProcessSpec::new("container", &["list", "--all", "--format", "json"]),
            success(containers),
        ),
        (
            ProcessSpec::new("container", &["image", "list", "--format", "json"]),
            success("[]"),
        ),
        (
            ProcessSpec::new("container", &["network", "list", "--format", "json"]),
            success("[]"),
        ),
        (
            ProcessSpec::new("container", &["volume", "list", "--format", "json"]),
            success("[]"),
        ),
        (
            ProcessSpec::new("container", &["inspect", "--", "web"]),
            success(containers),
        ),
    ]));
    let runtime = ProviderRuntime::new(
        vec![Arc::new(
            tuivir::infrastructure::provider::AppleContainerWorkspace,
        )],
        cli.clone(),
    );
    let mut app = App::new();
    let requests = app.update(runtime.discover().await.remove(0).into_event());
    drive(&mut app, &runtime, requests).await;
    let requests = open_settings(&mut app);
    drive(&mut app, &runtime, requests).await;
    let screen = render_to_text(app.state(), 200, 36);
    let text = screen.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(screen.contains("Read only"), "{screen}");
    assert!(!screen.contains("No unsaved changes"), "{screen}");
    assert!(!screen.contains("Edit field"), "{screen}");
    assert!(!screen.contains("Review & apply"), "{screen}");
    press(&mut app, "?");
    let help = render_to_text(app.state(), 200, 36);
    assert!(!help.contains("Edit field"), "{help}");
    assert!(!help.contains("Review & apply"), "{help}");
    press(&mut app, "esc");
    assert!(text.contains("CPU count: 2"), "{screen}");
    assert!(text.contains("Memory (GiB): 1"), "{screen}");
    let tabs = screen
        .lines()
        .find(|line| line.contains("Overview"))
        .unwrap();
    assert!(!tabs.contains("Inspect"), "{tabs}");
    let fields = &app
        .state()
        .selected_configuration()
        .unwrap()
        .actual
        .as_ref()
        .unwrap()
        .fields;
    assert!(fields.iter().any(
        |field| field.label == "configuration.initProcess.executable" && field.value == "/bin/sh"
    ));
    assert!(
        !fields
            .iter()
            .any(|field| field.label == "configuration.resources.memoryInBytes")
    );
    assert!(screen.contains("Read-only settings"), "{screen}");
    assert!(
        text.contains("Image: docker.io/library/alpine:3.23"),
        "{screen}"
    );
    assert!(
        screen.contains("in-place CPU/memory resize is unsupported"),
        "{screen}"
    );
    assert!(press(&mut app, "enter").is_empty());
    assert!(press(&mut app, "ctrl+a").is_empty());
    assert!(app.state().confirmation.is_none());
    cli.assert_exhausted();
}

#[tokio::test]
async fn tart_stopped_vm_remains_stopped_after_memory_changes() {
    let initial = r#"{"CPU":2,"Memory":4096,"Running":false,"State":"stopped"}"#;
    let updated = initial.replace("4096", "8192");
    let (mut app, runtime, cli) = tart_app(
        initial,
        vec![
            tart_read(initial),
            (
                ProcessSpec::new("tart", &["set", "--memory", "8192", "--", "mac"]),
                success(""),
            ),
            tart_read(&updated),
            (
                ProcessSpec::new("tart", &["list", "--format", "json"]),
                success(&TART_LIST.replace("running", "stopped")),
            ),
        ],
    )
    .await;
    press(&mut app, "down");
    edit_selected_field(&mut app, "8");
    press(&mut app, "ctrl+a");
    assert!(render_to_text(app.state(), 180, 36).contains("Resource remains stopped"));
    let requests = press(&mut app, "enter");
    drive(&mut app, &runtime, requests).await;
    let screen = render_to_text(app.state(), 180, 36);
    assert!(screen.contains("Resource State: Stopped"), "{screen}");
    assert!(screen.contains("Memory (GiB): 8"), "{screen}");
    cli.assert_exhausted();
}

#[tokio::test]
async fn tart_rejected_native_minimum_preserves_draft_and_restarts_the_vm() {
    let (mut app, runtime, cli) = tart_app(
        TART_INITIAL,
        vec![
            tart_read(TART_INITIAL),
            (
                ProcessSpec::new("tart", &["stop", "--", "mac"]),
                success(""),
            ),
            (
                ProcessSpec::new("tart", &["set", "--memory", "1", "--", "mac"]),
                common::failure("VM should have 4096 MiB of memory at minimum"),
            ),
            (
                ProcessSpec::background("tart", &["run", "--no-graphics", "--", "mac"]),
                success(""),
            ),
            tart_read(TART_INITIAL),
            (
                ProcessSpec::new("tart", &["list", "--format", "json"]),
                success(TART_LIST),
            ),
        ],
    )
    .await;
    press(&mut app, "down");
    edit_selected_field(&mut app, "0.0009765625");
    press(&mut app, "ctrl+a");
    let requests = press(&mut app, "enter");
    drive(&mut app, &runtime, requests).await;
    let screen = render_to_text(app.state(), 180, 36);
    assert!(screen.contains("4 → 0.0009765625"), "{screen}");
    assert!(screen.contains("memory at minimum"), "{screen}");
    assert!(screen.contains("Resource State: Running"), "{screen}");
    cli.assert_exhausted();
}

#[tokio::test]
async fn tart_restart_failure_reports_saved_settings_and_actual_stopped_state() {
    let updated = r#"{"CPU":4,"Memory":4096,"Running":false,"State":"stopped"}"#;
    let (mut app, runtime, cli) = tart_app(
        TART_INITIAL,
        vec![
            tart_read(TART_INITIAL),
            (
                ProcessSpec::new("tart", &["stop", "--", "mac"]),
                success(""),
            ),
            (
                ProcessSpec::new("tart", &["set", "--cpu", "4", "--", "mac"]),
                success(""),
            ),
            (
                ProcessSpec::background("tart", &["run", "--no-graphics", "--", "mac"]),
                common::failure("guest failed to boot"),
            ),
            tart_read(updated),
            (
                ProcessSpec::new("tart", &["list", "--format", "json"]),
                success(&TART_LIST.replace("running", "stopped")),
            ),
        ],
    )
    .await;
    edit_selected_field(&mut app, "4");
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
    assert!(screen.contains("CPU count: 4"), "{screen}");
    assert!(!screen.contains('→'), "{screen}");
    cli.assert_exhausted();
}

#[tokio::test]
async fn tart_rejects_invalid_counts_before_review_or_provider_writes() {
    for (field, value) in [
        (0, "0"),
        (0, "1.5"),
        (0, "65536"),
        (0, "0-3"),
        (1, "0"),
        (1, "0.1"),
        (1, "17592186044416"),
    ] {
        let (mut app, _, cli) = tart_app(TART_INITIAL, vec![]).await;
        if field == 1 {
            press(&mut app, "down");
        }
        edit_selected_field(&mut app, value);
        assert!(press(&mut app, "ctrl+a").is_empty());
        assert!(app.state().confirmation.is_none());
        let screen = render_to_text(app.state(), 180, 36);
        assert!(
            screen.contains(if field == 0 {
                "must be a whole number"
            } else {
                "Memory must be a size in GiB"
            }),
            "{screen}"
        );
        cli.assert_exhausted();
    }
}

#[tokio::test]
async fn configuration_form_distinguishes_unsaved_values_and_shows_compact_controls() {
    let (mut app, _, cli) = tart_app(TART_INITIAL, vec![]).await;
    edit_selected_field(&mut app, "4");
    let screen = render_to_text(app.state(), 160, 28);
    assert!(screen.contains("1 unsaved change"), "{screen}");
    let controls = screen
        .lines()
        .find(|line| line.contains("ctrl+a") && line.contains("Review"));
    assert!(
        controls.is_some_and(|line| line.contains("enter") && line.contains("Edit")),
        "{screen}"
    );
    assert!(!screen.contains("Apply Configuration Draft"), "{screen}");
    assert!(screen.contains("┌"), "{screen}");
    press(&mut app, "enter");
    let screen = render_to_text(app.state(), 160, 28);
    assert!(screen.contains("Editing"), "{screen}");
    assert!(screen.contains("4▏"), "{screen}");
    assert!(
        screen.lines().any(|line| line.contains("enter")
            && line.contains("Keep draft")
            && line.contains("ctrl+u")),
        "{screen}"
    );
    cli.assert_exhausted();
}

#[tokio::test]
async fn narrow_configuration_keeps_memory_values_and_editing_feedback_visible() {
    let (mut app, runtime) = docker_app(DOCKER_INITIAL).await;
    let requests = open_settings(&mut app);
    drive(&mut app, &runtime, requests).await;
    press(&mut app, "down");
    edit_selected_field(&mut app, "0.75");
    press(&mut app, "enter");
    let screen = render_to_text(app.state(), 80, 24);
    assert!(screen.contains("0.75▏"), "{screen}");
    assert!(screen.contains("1 unsaved change"), "{screen}");
    assert!(screen.contains("Editing"), "{screen}");
}

#[tokio::test]
async fn short_configuration_keeps_selected_input_and_validation_visible() {
    let (mut app, _, cli) = tart_app(TART_INITIAL, vec![]).await;
    press(&mut app, "down");
    edit_selected_field(&mut app, "0");
    press(&mut app, "enter");
    let screen = render_to_text(app.state(), 120, 12);
    assert!(screen.contains("4 → 0▏"), "{screen}");
    assert!(screen.contains("Memory must be a size in GiB"), "{screen}");
    cli.assert_exhausted();
}

#[tokio::test]
async fn tart_fractional_gib_memory_is_reviewed_then_converted_to_native_mib() {
    let updated = r#"{"CPU":2,"Memory":1536,"Running":false,"State":"stopped"}"#;
    let initial = TART_INITIAL
        .replace("true", "false")
        .replace("running", "stopped");
    let (mut app, runtime, cli) = tart_app(
        &initial,
        vec![
            tart_read(&initial),
            (
                ProcessSpec::new("tart", &["set", "--memory", "1536", "--", "mac"]),
                success(""),
            ),
            tart_read(updated),
            (
                ProcessSpec::new("tart", &["list", "--format", "json"]),
                success(&TART_LIST.replace("running", "stopped")),
            ),
        ],
    )
    .await;
    assert!(render_to_text(app.state(), 160, 28).contains("Memory (GiB): 4"));
    press(&mut app, "down");
    edit_selected_field(&mut app, "1.5");
    press(&mut app, "ctrl+a");
    let screen = render_to_text(app.state(), 160, 28);
    assert!(screen.contains("Memory (GiB): 4 → 1.5"), "{screen}");
    let requests = press(&mut app, "enter");
    drive(&mut app, &runtime, requests).await;
    let screen = render_to_text(app.state(), 160, 28);
    assert!(screen.contains("Memory (GiB): 1.5"), "{screen}");
    assert!(!screen.contains('→'), "{screen}");
    cli.assert_exhausted();
}

#[tokio::test]
async fn configuration_review_explains_the_target_downtime_and_apply_controls() {
    let (mut app, _, cli) = tart_app(TART_INITIAL, vec![]).await;
    edit_selected_field(&mut app, "4");
    press(&mut app, "ctrl+a");
    let screen = render_to_text(app.state(), 100, 22);
    assert!(screen.contains("Review changes"), "{screen}");
    assert!(screen.contains("mac"), "{screen}");
    assert!(screen.contains("CPU count: 2 → 4"), "{screen}");
    assert!(screen.contains("Downtime: stop/start"), "{screen}");
    assert!(screen.contains("Apply changes"), "{screen}");
    assert!(screen.contains("Cancel"), "{screen}");
    press(&mut app, "esc");
    assert!(app.state().confirmation.is_none());
    assert!(render_to_text(app.state(), 100, 22).contains("2 → 4"));
    cli.assert_exhausted();
}

#[tokio::test]
async fn tart_separates_editable_settings_from_read_only_details_and_skips_their_focus() {
    let initial = r#"{"CPU":2,"Memory":4096,"State":"stopped","OS":"darwin","Disk":80,"Display":"1920x1080"}"#;
    let (mut app, _, cli) = tart_app(initial, vec![]).await;
    let screen = render_to_text(app.state(), 160, 32);
    let text = screen.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(screen.contains("Editable settings"), "{screen}");
    assert!(screen.contains("Read-only settings"), "{screen}");
    assert!(text.contains("OS: darwin"), "{screen}");
    assert!(text.contains("Disk capacity (GB): 80"), "{screen}");
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(160, 32)).unwrap();
    let drawn = terminal
        .draw(|frame| tuivir::presentation::render(app.state(), frame))
        .unwrap();
    let heading_row = screen
        .lines()
        .position(|line| line.contains("Read-only settings"))
        .unwrap();
    let heading_column = screen
        .lines()
        .nth(heading_row)
        .unwrap()
        .chars()
        .position(|c| c == 'R')
        .unwrap();
    let heading = &drawn.buffer[(heading_column as u16, heading_row as u16)];
    let label = &drawn.buffer[(heading_column as u16, heading_row as u16 + 1)];
    assert_ne!(
        heading.fg, label.fg,
        "Section heading should stand out from muted labels"
    );
    assert!(heading.modifier.contains(ratatui::style::Modifier::BOLD));
    press(&mut app, "down");
    press(&mut app, "down");
    edit_selected_field(&mut app, "8");
    let screen = render_to_text(app.state(), 160, 32);
    let text = screen.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(screen.contains("Memory (GiB): 4 → 8"), "{screen}");
    assert!(text.contains("Disk capacity (GB): 80"), "{screen}");
    cli.assert_exhausted();
}

#[tokio::test]
async fn docker_configuration_includes_read_only_image_and_restart_policy() {
    let initial = r#"[{"Config":{"Image":"alpine:3.23"},"HostConfig":{"NanoCpus":1500000000,"Memory":536870912,"RestartPolicy":{"Name":"unless-stopped"}},"State":{"Status":"running"}}]"#;
    let (mut app, runtime) = docker_app(initial).await;
    let requests = open_settings(&mut app);
    drive(&mut app, &runtime, requests).await;
    let screen = render_to_text(app.state(), 160, 32);
    let text = screen.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(screen.contains("Read-only settings"), "{screen}");
    assert!(text.contains("Image: alpine:3.23"), "{screen}");
    assert!(text.contains("Restart policy: unless-stopped"), "{screen}");
}

#[tokio::test]
async fn incus_configuration_shows_kind_architecture_and_profiles_as_read_only() {
    let initial = r#"[{"name":"api","type":"container","architecture":"aarch64","profiles":["default","dev"],"status":"Running","expanded_config":{"limits.cpu":"2","limits.memory":"512MiB"}}]"#;
    let (mut app, runtime, cli) = incus_app(initial, vec![]).await;
    let requests = open_settings(&mut app);
    drive(&mut app, &runtime, requests).await;
    let screen = render_to_text(app.state(), 160, 32);
    let text = screen.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(screen.contains("Read-only settings"), "{screen}");
    assert!(text.contains("Kind: container"), "{screen}");
    assert!(text.contains("Architecture: aarch64"), "{screen}");
    assert!(text.contains("Profiles: default, dev"), "{screen}");
    cli.assert_exhausted();
}

#[tokio::test]
async fn narrow_configuration_review_keeps_downtime_and_both_actions_visible() {
    let (mut app, _, cli) = tart_app(TART_INITIAL, vec![]).await;
    edit_selected_field(&mut app, "4");
    press(&mut app, "ctrl+a");
    for width in [20, 30] {
        let screen = render_to_text(app.state(), width, 24);
        assert!(screen.contains("stop/start"), "{screen}");
        assert!(screen.contains("running."), "{screen}");
        assert!(screen.contains("Apply changes"), "{screen}");
        assert!(screen.contains("Cancel"), "{screen}");
    }
    cli.assert_exhausted();
}

#[tokio::test]
async fn tart_settings_merges_native_fields_without_a_duplicate_config_tab() {
    let initial = r#"{"CPU":2,"Memory":4096,"State":"stopped","OS":"darwin","Disk":80,"Display":"1280x800","Architecture":"arm64","Network":{"Mode":"shared"}}"#;
    let (app, _, cli) = tart_app(initial, vec![]).await;
    let screen = render_to_text(app.state(), 180, 36);
    assert!(screen.contains("[ Settings ]"), "{screen}");
    let tabs = screen
        .lines()
        .find(|line| line.contains("Overview"))
        .unwrap();
    assert!(!tabs.contains("Config"), "{tabs}");
    assert!(
        screen.contains("Architecture:") && screen.contains("arm64"),
        "{screen}"
    );
    assert!(
        screen.contains("Network.Mode:") && screen.contains("shared"),
        "{screen}"
    );
    let fields = &app
        .state()
        .selected_configuration()
        .unwrap()
        .actual
        .as_ref()
        .unwrap()
        .fields;
    assert_eq!(
        fields
            .iter()
            .filter(|field| field.label.contains("Memory"))
            .count(),
        1
    );
    cli.assert_exhausted();
}

#[tokio::test]
async fn incus_settings_includes_remaining_effective_config_and_devices() {
    let initial = r#"[{"name":"api","type":"container","status":"Running","expanded_config":{"limits.cpu":"2","limits.memory":"1GiB","security.nesting":"true"},"expanded_devices":{"root":{"type":"disk","path":"/","pool":"default"}}}]"#;
    let (mut app, runtime, cli) = incus_app(initial, vec![]).await;
    let requests = open_settings(&mut app);
    drive(&mut app, &runtime, requests).await;
    let screen = render_to_text(app.state(), 180, 40);
    let tabs = screen
        .lines()
        .find(|line| line.contains("Overview"))
        .unwrap();
    assert!(!tabs.contains("Config"), "{tabs}");
    assert!(
        screen.contains("security.nesting:") && screen.contains("true"),
        "{screen}"
    );
    assert!(
        screen.contains("expanded_devices.root.pool:") && screen.contains("default"),
        "{screen}"
    );
    let fields = &app
        .state()
        .selected_configuration()
        .unwrap()
        .actual
        .as_ref()
        .unwrap()
        .fields;
    assert!(
        !fields.iter().any(
            |field| field.label.contains("limits.cpu") || field.label.contains("limits.memory")
        )
    );
    cli.assert_exhausted();
}

#[tokio::test]
async fn docker_settings_preserves_nested_native_settings_and_hides_duplicate_inspect() {
    let initial = r#"[{"Config":{"Image":"alpine:3.23","Env":["MODE=dev"],"Labels":{"team":"tools"}},"HostConfig":{"NanoCpus":1000000000,"Memory":1073741824,"RestartPolicy":{"Name":"always"},"ReadonlyRootfs":true},"State":{"Status":"running"}}]"#;
    let (mut app, runtime, cli) = docker_app_with(initial, vec![]).await;
    let requests = open_settings(&mut app);
    drive(&mut app, &runtime, requests).await;
    let screen = render_to_text(app.state(), 200, 40);
    let tabs = screen
        .lines()
        .find(|line| line.contains("Overview"))
        .unwrap();
    assert!(!tabs.contains("Inspect"), "{tabs}");
    assert!(
        screen.contains("Config.Env:") && screen.contains("MODE=dev"),
        "{screen}"
    );
    assert!(
        screen.contains("Config.Labels.team:") && screen.contains("tools"),
        "{screen}"
    );
    assert!(
        screen.contains("HostConfig.ReadonlyRootfs:") && screen.contains("true"),
        "{screen}"
    );
    let fields = &app
        .state()
        .selected_configuration()
        .unwrap()
        .actual
        .as_ref()
        .unwrap()
        .fields;
    assert!(!fields.iter().any(|field| field.label.contains("NanoCpus")
        || field.label == "HostConfig.Memory"
        || field.label == "Config.Image"));
    cli.assert_exhausted();
}

#[tokio::test]
async fn settings_scrolls_remaining_configuration_without_moving_editable_controls() {
    let mut initial = serde_json::json!({"CPU":2,"Memory":4096,"State":"stopped"});
    for index in 0..35 {
        initial[format!("Extra{index:02}")] = serde_json::json!(format!("value-{index:02}"));
    }
    let (mut app, _, cli) = tart_app(&initial.to_string(), vec![]).await;
    let screen = render_to_text(app.state(), 100, 20);
    assert!(!screen.contains("value-34"), "{screen}");
    let layout = tuivir::presentation::ScreenLayout::measure(
        app.state(),
        ratatui::layout::Rect::new(0, 0, 100, 20),
    );
    app.set_configuration_scroll_limit(tuivir::presentation::configuration_scroll_limit(
        app.state(),
        &layout,
    ));
    assert!(render_to_text(app.state(), 100, 20).contains("ctrl+u/ctrl+d scroll"));
    for _ in 0..32 {
        press(&mut app, "pagedown");
    }
    let screen = render_to_text(app.state(), 100, 20);
    assert!(screen.contains("value-34"), "{screen}");
    assert!(screen.contains("CPU count: 2"), "{screen}");
    assert!(screen.contains("Memory (GiB): 4"), "{screen}");
    press(&mut app, "pageup");
    let screen = render_to_text(app.state(), 100, 20);
    assert!(
        !screen.contains("value-34"),
        "Scrolling up should respond immediately: {screen}"
    );
    press(&mut app, "enter");
    let editing = render_to_text(app.state(), 100, 20);
    assert!(
        !editing.contains("ctrl+u/ctrl+d scroll"),
        "Scroll hints must not advertise input-clearing keys while editing: {editing}"
    );
    cli.assert_exhausted();
}

#[tokio::test]
async fn tart_settings_keeps_native_nulls_and_unknown_fields() {
    let initial = r#"{"CPU":2,"Memory":4096,"State":"stopped","Disk":null,"Display":null,"DiskFormat":"asif","Size":"20.125"}"#;
    let (app, _, cli) = tart_app(initial, vec![]).await;
    let fields = &app
        .state()
        .selected_configuration()
        .unwrap()
        .actual
        .as_ref()
        .unwrap()
        .fields;
    for (label, value) in [
        ("Disk", "null"),
        ("Display", "null"),
        ("DiskFormat", "asif"),
        ("Size", "20.125"),
    ] {
        assert!(
            fields
                .iter()
                .any(|field| field.label == label && field.value == value),
            "Missing {label}: {value}"
        );
    }
    cli.assert_exhausted();
}

#[tokio::test]
async fn wrapped_settings_values_keep_their_column_and_do_not_hide_long_labels() {
    let initial = r#"{"CPU":2,"Memory":4096,"State":"stopped","Digest":"sha256:0123456789abcdefghijklmnopqrstuvwxyz0123456789abcdefghijklmnopqrstuvwxyz","AnExtremelyLongNativeSettingName":{"NestedKey":"value"}}"#;
    let (app, _, cli) = tart_app(initial, vec![]).await;
    let screen = render_to_text(app.state(), 150, 40);
    let lines: Vec<_> = screen.lines().collect();
    let (first_row, column) = lines
        .iter()
        .enumerate()
        .find_map(|(row, line)| {
            line.find("sha256:")
                .map(|column| (row, line[..column].chars().count()))
        })
        .unwrap();
    let continuation = lines[first_row + 1];
    assert!(
        continuation
            .chars()
            .take(column)
            .skip(column - 3)
            .all(|c| c == ' '),
        "Continuation should stay in the value column: {screen}"
    );
    assert!(
        continuation
            .chars()
            .skip(column)
            .any(|c| c.is_ascii_alphanumeric()),
        "Wrapped value should continue in the same column: {screen}"
    );
    assert!(screen.contains("NestedKey"), "{screen}");
    cli.assert_exhausted();
}
