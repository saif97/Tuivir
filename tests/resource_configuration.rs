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
    let cli = Arc::new(FixtureCli::new([
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
    ]));
    let runtime = ProviderRuntime::new(vec![Arc::new(DockerWorkspace)], cli);
    let mut app = App::new();
    let discovery = runtime.discover().await.remove(0);
    let requests = app.update(discovery.into_event());
    drive(&mut app, &runtime, requests).await;
    (app, runtime)
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
