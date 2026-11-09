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

#[tokio::test]
async fn supported_providers_validate_using_their_native_account_selection() {
    use tuivir::infrastructure::provider::{
        AppleContainerWorkspace, DockerSandboxWorkspace, IncusWorkspace,
    };
    let cases: Vec<(Box<dyn ProviderWorkspace>, &str, Vec<&str>, Vec<&str>)> = vec![
        (
            Box::new(IncusWorkspace),
            "incus",
            vec![
                "exec",
                "web",
                "--",
                "su",
                "-l",
                "worker",
                "-c",
                "printf TUIVIR_SHELL_VERIFIED",
            ],
            vec!["exec", "web", "--", "su", "-l", "worker"],
        ),
        (
            Box::new(AppleContainerWorkspace),
            "container",
            vec![
                "exec",
                "--user",
                "worker",
                "--",
                "web",
                "/bin/sh",
                "-c",
                "printf TUIVIR_SHELL_VERIFIED",
            ],
            vec![
                "exec",
                "--interactive",
                "--tty",
                "--user",
                "worker",
                "--",
                "web",
            ],
        ),
        (
            Box::new(DockerSandboxWorkspace),
            "sbx",
            vec![
                "exec",
                "--user",
                "worker",
                "web",
                "bash",
                "-c",
                "printf TUIVIR_SHELL_VERIFIED",
            ],
            vec!["exec", "-it", "--user", "worker", "web"],
        ),
    ];
    for (provider, program, verification, interactive) in cases {
        let cli = FixtureCli::new([(
            ProcessSpec::new(program, &verification),
            success("TUIVIR_SHELL_VERIFIED"),
        )]);
        let shell = provider
            .shell_as(&cli, &resource_target("resources", "web"), "worker")
            .await
            .unwrap();
        assert_eq!(shell.program(), program);
        assert_eq!(&shell.args()[..interactive.len()], interactive);
        assert!(shell.startup_marker().is_some());
        cli.assert_exhausted();
    }
}
