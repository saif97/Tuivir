mod common;

use common::{FixtureCli, resource_target, success};
use tuivir::infrastructure::{
    process::ProcessSpec,
    provider::{DockerWorkspace, ProviderWorkspace},
};

#[tokio::test]
async fn docker_discovers_accounts_only_through_getent() {
    let cli = FixtureCli::new([(
        ProcessSpec::new(
            "docker",
            &[
                "exec",
                "web",
                "/bin/sh",
                "-c",
                "if command -v getent >/dev/null 2>&1; then exec getent passwd; else printf 'TUIVIR_GETENT_MISSING\\n'; fi",
            ],
        ),
        success("root:x:0:0:root:/root:/bin/sh\nworker:x:1000:1000::/home/worker:/bin/sh\n"),
    )]);
    let accounts = DockerWorkspace
        .shell_accounts(&cli, &resource_target("containers", "web"))
        .await
        .unwrap();
    assert_eq!(accounts, Some(vec!["root".into(), "worker".into()]));
    cli.assert_exhausted();
}

#[tokio::test]
async fn selected_docker_account_is_verified_before_returning_an_interactive_launch() {
    let cli = FixtureCli::new([(
        ProcessSpec::new(
            "docker",
            &[
                "exec",
                "--user",
                "worker",
                "web",
                "/bin/sh",
                "-c",
                "printf TUIVIR_SHELL_VERIFIED",
            ],
        ),
        success("TUIVIR_SHELL_VERIFIED"),
    )]);
    let shell = DockerWorkspace
        .shell_as(&cli, &resource_target("containers", "web"), "worker")
        .await
        .unwrap();
    assert_eq!(shell.program(), "docker");
    assert_eq!(
        &shell.args()[..5],
        &["exec", "-it", "--user", "worker", "web"]
    );
    assert!(shell.startup_marker().is_some());
    cli.assert_exhausted();
}
